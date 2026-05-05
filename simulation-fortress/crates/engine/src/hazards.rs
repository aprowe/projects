//! Environmental hazards: tiles or entities that *do something* to a
//! creature passing through them. Today the only kind is `Slippery`
//! (oil, ice, mopped floor) — when a creature steps onto its position
//! they roll against `slip_chance` and, on a hit, get a `Wait` task
//! shoved to the front of their queue (the simulation's stand-in for
//! "knocked prone for N ticks"). The kind is an enum so future
//! variants (burning, poisonous, charged) can be added without
//! rewiring the system.

use bevy_ecs::prelude::{Component, Entity, World};

use crate::components::Position;
use crate::log::{Event, EventLog};
use crate::rng::Rng;
use crate::tasks::{Task, TaskQueue};
use crate::time::Clock;
use crate::world::Pos;

#[derive(Component, Clone, Debug)]
pub enum Hazard {
    Slippery {
        /// 0.0..=1.0 — probability of slipping on each entry.
        slip_chance: f32,
        /// Ticks the creature spends prone after slipping.
        prone_ticks: u32,
        /// Human-readable name ("puddle of oil", "patch of ice").
        label: String,
    },
}

/// Check `EntityMoved` events emitted earlier this tick and apply any
/// hazards present at the destinations. Run *after* `execute_tasks`
/// in the schedule so movement events from this tick are visible.
pub fn check_hazards(world: &mut World) {
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

    let hazards: Vec<(Pos, Hazard)> = {
        let mut q = world.query::<(&Position, &Hazard)>();
        q.iter(world).map(|(p, h)| (p.0, h.clone())).collect()
    };
    if hazards.is_empty() {
        return;
    }

    for (mover, dest) in moves {
        for (h_pos, hazard) in &hazards {
            if dest != *h_pos {
                continue;
            }
            apply_hazard(world, mover, hazard);
        }
    }
}

fn apply_hazard(world: &mut World, entity: Entity, hazard: &Hazard) {
    match hazard {
        Hazard::Slippery {
            slip_chance,
            prone_ticks,
            label,
        } => {
            let slipped = world.resource_mut::<Rng>().chance(*slip_chance);
            if !slipped {
                return;
            }
            if let Some(mut q) = world.get_mut::<TaskQueue>(entity) {
                q.push_front(Task::Wait(*prone_ticks));
            }
            let tick = world.resource::<Clock>().tick;
            world.resource_mut::<EventLog>().push(
                tick,
                Event::Slipped {
                    entity,
                    hazard: label.clone(),
                    prone_ticks: *prone_ticks,
                },
            );
        }
    }
}
