//! Tile-interaction physics. Today this is just the **footing check**:
//! whenever a creature steps onto a tile, the simulation rolls
//! against the friction of the surface (any coating + the floor
//! material), the friction of the creature's footwear, and the
//! creature's current `Mobility` capacity. Slippery substrates and
//! low Mobility lose footing; sturdy boots and good legs keep it.
//!
//! Slipping is no longer a special-case `Hazard::Slippery` enum —
//! it's the natural consequence of low friction. Anything that wants
//! to make a tile slippery just attaches a `Coating { material }` to
//! an entity at that position.

use bevy_ecs::prelude::{Component, Entity, World};

use crate::anatomy::{function_capacity, Function};
use crate::components::Position;
use crate::items::{BodySlot, ItemMaterial, Wearing};
use crate::log::{Event, EventLog};
use crate::rng::Rng;
use crate::tasks::{Task, TaskQueue};
use crate::time::Clock;
use crate::world::{MaterialId, Pos, VoxelWorld};

/// How an entity is moving right now. Affects how much footing
/// matters: a running soldier on oil is going down hard; a sneaking
/// thief on the same tile probably catches themselves.
///
/// If a creature has no `Locomotion` component, the footing check
/// derives one from the head of its task queue:
/// `Attack` → `Running`, `MoveTo` → `Walking`, `Wait` → `Standing`,
/// anything else → `Walking`.
#[derive(Component, Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum Locomotion {
    /// Not moving. Skips the footing roll entirely.
    Standing,
    /// Quiet, careful, slow.
    Sneaking,
    /// On hands and knees — low center of gravity.
    Crawling,
    /// Default upright movement.
    #[default]
    Walking,
    /// Faster than walking, with momentum that bites if footing fails.
    Running,
}

impl Locomotion {
    /// Slip-probability multiplier vs. walking baseline.
    pub fn slip_factor(self) -> f32 {
        match self {
            Locomotion::Standing => 0.0,
            Locomotion::Sneaking => 0.5,
            Locomotion::Crawling => 0.4,
            Locomotion::Walking => 1.0,
            Locomotion::Running => 1.8,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Locomotion::Standing => "standing",
            Locomotion::Sneaking => "sneaking",
            Locomotion::Crawling => "crawling",
            Locomotion::Walking => "walking",
            Locomotion::Running => "running",
        }
    }
}

/// A puddle, slick, dust patch, blood splatter — any material
/// covering a tile and interacting with whatever moves through it.
/// Volume is read but not yet consumed (drying, blotting, fire-spread
/// will hook in here).
#[derive(Component, Clone, Debug)]
pub struct Coating {
    pub material: MaterialId,
    pub volume: f32,
}

/// Effective friction at or above this is considered safe walking;
/// below it, slip probability ramps linearly to 1.0 at zero friction.
const SAFE_EFFECTIVE_FRICTION: f32 = 0.3;
/// Friction assumed for an entity with no `Wearing.Feet` slot.
const BARE_FOOT_FRICTION: f32 = 0.7;
/// Mobility capacity treated as "fully balanced". For a humanoid plan
/// that's 2 legs + 2 feet = 4.0; for quadrupeds (8.0) and dragons
/// (4.0) we just clamp.
const FULL_BALANCE_MOBILITY: f32 = 4.0;

/// Read this tick's `EntityMoved` events and run a footing check on
/// each move. Run AFTER `execute_tasks` in the schedule so the move
/// events are visible.
pub fn footing_check(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let moves: Vec<(Entity, Pos)> = world
        .resource::<EventLog>()
        .events_at(tick)
        .filter_map(|e| match e {
            Event::EntityMoved { entity, to, .. } => Some((*entity, *to)),
            _ => None,
        })
        .collect();
    if moves.is_empty() {
        return;
    }

    // Snapshot all coatings once.
    let coatings: Vec<(Pos, MaterialId)> = {
        let mut q = world.query::<(&Position, &Coating)>();
        q.iter(world).map(|(p, c)| (p.0, c.material)).collect()
    };

    for (mover, dest) in moves {
        let locomotion = current_locomotion(world, mover);
        let loco_factor = locomotion.slip_factor();
        if loco_factor == 0.0 {
            continue; // Standing entities can't slip.
        }

        // The slipperiest coating sitting on this tile (if any).
        let coating_here: Option<(MaterialId, f32)> = coatings
            .iter()
            .filter(|(p, _)| *p == dest)
            .filter_map(|(_, m)| {
                world
                    .resource::<VoxelWorld>()
                    .material(*m)
                    .map(|mat| (*m, mat.friction))
            })
            .min_by(|a, b| {
                a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
            });
        let coating_friction = coating_here.map(|(_, f)| f).unwrap_or(1.0);

        // Floor material under the destination.
        let voxel = world.resource::<VoxelWorld>().voxel(dest);
        let floor_friction = world
            .resource::<VoxelWorld>()
            .material(voxel.material)
            .map(|m| m.friction)
            .unwrap_or(0.6);

        // Whatever's on the mover's feet.
        let footwear_friction = world
            .get::<Wearing>(mover)
            .and_then(|w| w.get(BodySlot::Feet))
            .and_then(|item| world.get::<ItemMaterial>(item).copied())
            .and_then(|m| {
                world
                    .resource::<VoxelWorld>()
                    .material(m.0)
                    .map(|mat| mat.friction)
            })
            .unwrap_or(BARE_FOOT_FRICTION);

        // Balance derived from current Mobility (broken legs = wobbly).
        let mobility = function_capacity(world, mover, Function::Mobility);
        let balance = (mobility / FULL_BALANCE_MOBILITY).clamp(0.0, 1.0);

        // The tile is as slippery as its slipperiest layer (coating
        // dominates floor when present); footwear and balance are
        // multiplicative buffers. Slip probability ramps linearly
        // from 0 (effective ≥ SAFE) to 1 (effective = 0).
        let surface = coating_friction.min(floor_friction);
        let effective = surface * footwear_friction * balance;
        if effective >= SAFE_EFFECTIVE_FRICTION {
            continue;
        }
        let base_slip =
            ((SAFE_EFFECTIVE_FRICTION - effective) / SAFE_EFFECTIVE_FRICTION).clamp(0.0, 1.0);
        let slip_prob = (base_slip * loco_factor).clamp(0.0, 1.0);

        let slipped = world.resource_mut::<Rng>().chance(slip_prob);
        if !slipped {
            continue;
        }

        // Worse footing + faster motion → longer recovery (1..=6 ticks).
        let prone_ticks = (1.0 + base_slip * loco_factor * 4.0).round() as u32;

        if let Some(mut q) = world.get_mut::<TaskQueue>(mover) {
            q.push_front(Task::Wait(prone_ticks));
        }

        // Name the cause for narration: the coating if there is one,
        // otherwise the floor material.
        let cause = match coating_here {
            Some((mat, _)) => world
                .resource::<VoxelWorld>()
                .material(mat)
                .map(|m| format!("{} on the floor", m.name))
                .unwrap_or_else(|| "slick patch".into()),
            None => world
                .resource::<VoxelWorld>()
                .material(voxel.material)
                .map(|m| format!("{} floor", m.name))
                .unwrap_or_else(|| "floor".into()),
        };

        let tick_now = world.resource::<Clock>().tick;
        world.resource_mut::<EventLog>().push(
            tick_now,
            Event::Slipped {
                entity: mover,
                hazard: cause,
                prone_ticks,
            },
        );
    }
}

/// Current locomotion mode of `entity`. Explicit `Locomotion`
/// component wins; otherwise we infer from the actor's current task.
fn current_locomotion(world: &World, entity: Entity) -> Locomotion {
    if let Some(loco) = world.get::<Locomotion>(entity) {
        return *loco;
    }
    let queue = match world.get::<TaskQueue>(entity) {
        Some(q) => q,
        None => return Locomotion::Standing,
    };
    match queue.front() {
        Some(Task::Attack(_)) => Locomotion::Running,
        Some(Task::MoveTo(_)) => Locomotion::Walking,
        Some(Task::Wait(_)) => Locomotion::Standing,
        Some(_) => Locomotion::Walking,
        None => Locomotion::Standing,
    }
}
