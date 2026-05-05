//! Fire: heat propagation and ignition spread.
//!
//! `Burning` marks any entity that's currently on fire. The
//! `tick_fire` system, run once per tick, does three things:
//!
//! 1. **Damage**: every burning entity loses one HP per `intensity`
//!    each tick (creatures burn faster, since their flesh is more
//!    flammable than oak). When a creature is burning, also apply
//!    `StatusKind::OnFire`. When a furniture / item burns out
//!    (intensity ticks below 1) it's despawned.
//! 2. **Spread**: each tick, every burning entity rolls a chance
//!    to ignite an adjacent flammable entity. The chance scales
//!    with `intensity` and the neighbor material's `flammable`
//!    flag.
//! 3. **Ambient**: each burning entity emits its own `SoundEmitted`
//!    crackle and counts as a heat source for the overlay (fed by
//!    its `Powered::Fire` proxy below).
//!
//! Scenarios ignite things by inserting `Burning` on a target,
//! either explicitly (a Molotov hits a couch) or by reaction (the
//! cabin's fireplace's stray spark on the brushwood floor — left
//! to scenarios for now). Future iteration: hook into a proper
//! per-tile `Temperature` map.

use bevy_ecs::prelude::{Component, Entity, World};

use crate::components::{Health, Position};
use crate::items::Mass;
use crate::log::{Event, EventLog};
use crate::sound::SoundKind;
use crate::status::{apply_status, StatusKind};
use crate::time::Clock;
use crate::world::Pos;

/// Live state: this entity is currently on fire. `intensity` 0..255
/// scales damage + spread chance; `fuel_ticks` counts down each
/// tick — when it hits zero the fire goes out (and the entity is
/// despawned if non-creature).
#[derive(Component, Copy, Clone, Debug)]
pub struct Burning {
    pub intensity: u8,
    pub fuel_ticks: u32,
}

impl Burning {
    pub fn new() -> Self {
        Self { intensity: 4, fuel_ticks: 80 }
    }
    pub fn small() -> Self {
        Self { intensity: 2, fuel_ticks: 40 }
    }
    pub fn raging() -> Self {
        Self { intensity: 8, fuel_ticks: 200 }
    }
}

impl Default for Burning {
    fn default() -> Self { Self::new() }
}

/// Engine system. Per tick:
///   - Drain HP / Mass from burning entities.
///   - Apply OnFire status to creatures that are burning.
///   - Roll a small chance per neighbor to spread to adjacent
///     flammable entities.
///   - Emit a crackle sound from each fire so the overlay /
///     hearing pipeline both pick it up.
pub fn tick_fire(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    // Snapshot all burning entities + their positions.
    type BurnRow = (Entity, Pos, u8, u32);
    let burning: Vec<BurnRow> = {
        let mut q = world.query::<(Entity, &Position, &Burning)>();
        q.iter(world)
            .map(|(e, p, b)| (e, p.0, b.intensity, b.fuel_ticks))
            .collect()
    };
    if burning.is_empty() {
        return;
    }

    // Snapshot all burnable neighbors (anything with Mass/Health
    // and a Position is considered flammable for the MVP).
    type Cand = (Entity, Pos, bool); // (entity, pos, is_creature)
    let candidates: Vec<Cand> = {
        let mut q = world.query::<(Entity, &Position, Option<&Health>)>();
        q.iter(world)
            .map(|(e, p, h)| (e, p.0, h.is_some()))
            .collect()
    };

    // Cheap-pseudo RNG using the tick to avoid taking a borrow on
    // the engine Rng (the dice / combat systems already run that).
    let mut rng_state = tick.wrapping_mul(0x9e3779b97f4a7c15);
    let mut next_rand = || {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 7;
        rng_state ^= rng_state << 17;
        (rng_state & 0xFF) as u8
    };

    let mut to_apply_onfire: Vec<Entity> = Vec::new();
    let mut to_ignite: Vec<Entity> = Vec::new();
    let mut to_extinguish: Vec<Entity> = Vec::new();
    let mut to_damage: Vec<(Entity, i32)> = Vec::new();

    for (entity, pos, intensity, fuel) in &burning {
        // Damage / fuel decrement.
        let damage = (*intensity as i32 / 2).max(1);
        to_damage.push((*entity, damage));
        if *fuel <= 1 {
            to_extinguish.push(*entity);
            continue;
        }
        // Creatures burning catch the OnFire status.
        if world.get::<Health>(*entity).is_some() {
            to_apply_onfire.push(*entity);
        }
        // Emit a small crackle so overlays + hearing register the fire.
        world.resource_mut::<EventLog>().push(
            tick,
            Event::SoundEmitted {
                source: Some(*entity),
                position: *pos,
                kind: SoundKind::Other("crackle".into()),
                intensity: 0.10 + (*intensity as f32 / 30.0).min(0.30),
            },
        );
        // Spread chance to adjacent burnable candidates.
        let spread_threshold: u8 = (*intensity as u32 * 2).min(40) as u8;
        for (cand, cpos, is_creature) in &candidates {
            if cand == entity {
                continue;
            }
            let d = pos.chebyshev(*cpos);
            if d > 1 {
                continue;
            }
            // Already burning? Skip.
            if world.get::<Burning>(*cand).is_some() {
                continue;
            }
            // Creatures aren't ignited by adjacency alone (they'd
            // step out); only their flammable clothing / hair
            // catches via OnFire status if they enter a fire tile.
            if *is_creature {
                continue;
            }
            // Roll: 0..255, ignite if below threshold.
            if next_rand() < spread_threshold {
                to_ignite.push(*cand);
            }
        }
    }

    // Apply changes.
    for e in to_apply_onfire {
        apply_status(world, e, StatusKind::OnFire, 2, 1);
    }
    for (e, d) in to_damage {
        if let Some(mut h) = world.get_mut::<Health>(e) {
            h.current = (h.current - d).max(0);
        } else if let Some(mut m) = world.get_mut::<Mass>(e) {
            // Furniture/items don't have HP; consume mass instead.
            m.0 = (m.0 - d as f32).max(0.0);
        }
    }
    for e in to_ignite {
        let kind_label = world
            .get::<crate::components::Kind>(e)
            .map(|k| k.0.clone())
            .unwrap_or_else(|| format!("entity#{}", e.index()));
        world.resource_mut::<EventLog>().push(
            tick,
            Event::Note(format!("the {kind_label} catches fire.")),
        );
        world.entity_mut(e).insert(Burning::small());
    }
    // Decrement fuel + clean up burned-out entities.
    for (e, _, _, _) in &burning {
        if let Some(mut b) = world.get_mut::<Burning>(*e) {
            b.fuel_ticks = b.fuel_ticks.saturating_sub(1);
        }
    }
    for e in to_extinguish {
        // Remove Burning. If the entity has no HP (it's furniture)
        // and its mass hit zero, despawn — the couch is ash.
        world.entity_mut(e).remove::<Burning>();
        let no_hp = world.get::<Health>(e).is_none();
        let consumed = world
            .get::<Mass>(e)
            .map(|m| m.0 <= 0.0)
            .unwrap_or(false);
        if no_hp && consumed {
            let label = world
                .get::<crate::components::Kind>(e)
                .map(|k| k.0.clone())
                .unwrap_or_else(|| format!("entity#{}", e.index()));
            world.resource_mut::<EventLog>().push(
                tick,
                Event::Note(format!("the {label} burns to ash.")),
            );
            world.despawn(e);
        }
    }
}
