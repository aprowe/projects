//! Structured event log for the simulation.
//!
//! Every interesting thing that happens — entities spawning, moving, taking
//! damage, dying; voxels changing; free-form scenario notes — is pushed onto
//! the `EventLog` as a typed `Event`. Renderers and post-run analysis read
//! from the log; the `LogRenderer` turns events into prose narration.

use crate::entity::{EntityId, EntityStore};
use crate::time::Tick;
use crate::world::{MaterialId, Pos};

#[derive(Clone, Debug)]
pub enum Event {
    /// Free-form prose injected by a scenario.
    Note(String),
    EntitySpawned {
        id: EntityId,
        kind: String,
        faction: Option<String>,
        at: Pos,
    },
    EntityMoved {
        id: EntityId,
        from: Pos,
        to: Pos,
    },
    EntityAttacked {
        attacker: Option<EntityId>,
        target: EntityId,
        damage: i32,
        remaining_health: i32,
    },
    EntityKilled {
        id: EntityId,
        by: Option<EntityId>,
    },
    VoxelChanged {
        at: Pos,
        material: MaterialId,
    },
    VoxelRegionFilled {
        min: Pos,
        max: Pos,
        material: MaterialId,
    },
}

#[derive(Default)]
pub struct EventLog {
    events: Vec<(Tick, Event)>,
}

impl EventLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, tick: Tick, event: Event) {
        self.events.push((tick, event));
    }

    pub fn events_at(&self, tick: Tick) -> impl Iterator<Item = &Event> {
        self.events
            .iter()
            .filter(move |(t, _)| *t == tick)
            .map(|(_, e)| e)
    }

    pub fn all(&self) -> &[(Tick, Event)] {
        &self.events
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Render a single event as a sentence using entity state for labels.
pub fn narrate(event: &Event, entities: &EntityStore) -> String {
    match event {
        Event::Note(msg) => msg.clone(),
        Event::EntitySpawned {
            id: _,
            kind,
            faction,
            at,
        } => {
            let faction = faction
                .as_deref()
                .map(|f| format!(" ({f})"))
                .unwrap_or_default();
            format!(
                "A {kind}{faction} appears at ({}, {}, {}).",
                at.x, at.y, at.z
            )
        }
        Event::EntityMoved { id, from, to } => {
            let label = label(*id, entities);
            let dir = direction(*from, *to);
            format!("{label} steps {dir} to ({}, {}, {}).", to.x, to.y, to.z)
        }
        Event::EntityAttacked {
            attacker,
            target,
            damage,
            remaining_health,
        } => {
            let attacker_label = attacker
                .map(|a| label(a, entities))
                .unwrap_or_else(|| "Something unseen".into());
            let target_label = label(*target, entities);
            format!(
                "{attacker_label} strikes {target_label} for {damage} damage (target now at {remaining_health} hp)."
            )
        }
        Event::EntityKilled { id, by } => {
            let target_label = label(*id, entities);
            match by {
                Some(b) => {
                    let by_label = label(*b, entities);
                    format!("{target_label} collapses, killed by {by_label}.")
                }
                None => format!("{target_label} dies."),
            }
        }
        Event::VoxelChanged { at, material } => format!(
            "The voxel at ({}, {}, {}) becomes material #{material}.",
            at.x, at.y, at.z
        ),
        Event::VoxelRegionFilled { min, max, material } => format!(
            "Voxels from ({}, {}, {}) to ({}, {}, {}) are filled with material #{material}.",
            min.x, min.y, min.z, max.x, max.y, max.z
        ),
    }
}

fn label(id: EntityId, entities: &EntityStore) -> String {
    match entities.get(id) {
        Some(e) => format!("{}#{}", e.kind, e.id.0),
        None => format!("entity#{}", id.0),
    }
}

fn direction(from: Pos, to: Pos) -> &'static str {
    let dx = (to.x - from.x).signum();
    let dy = (to.y - from.y).signum();
    let dz = (to.z - from.z).signum();
    match (dx, dy, dz) {
        (0, 0, 0) => "in place",
        (0, -1, 0) => "north",
        (0, 1, 0) => "south",
        (1, 0, 0) => "east",
        (-1, 0, 0) => "west",
        (1, -1, 0) => "northeast",
        (-1, -1, 0) => "northwest",
        (1, 1, 0) => "southeast",
        (-1, 1, 0) => "southwest",
        (_, _, 1) => "upward",
        (_, _, -1) => "downward",
        _ => "onward",
    }
}
