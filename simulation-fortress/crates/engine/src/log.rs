//! Structured event log for the simulation.
//!
//! The `EventLog` is a bevy ECS `Resource` that lives on the simulation
//! world. Engine helpers and scenario systems push typed events onto it;
//! renderers read from it to produce narration and post-run summaries.

use bevy_ecs::prelude::{Entity, Resource, World};

use crate::anatomy::{BodyPartKind, PartStatus};
use crate::components::Kind;
use crate::items::{BodySlot, ItemName};
use crate::time::Tick;
use crate::world::{MaterialId, Pos};

#[derive(Clone, Debug)]
pub enum Event {
    /// Free-form prose injected by a scenario.
    Note(String),
    EntitySpawned {
        entity: Entity,
        kind: String,
        faction: Option<String>,
        at: Pos,
    },
    EntityMoved {
        entity: Entity,
        from: Pos,
        to: Pos,
    },
    EntityAttacked {
        attacker: Option<Entity>,
        target: Entity,
        damage: i32,
        remaining_health: i32,
    },
    EntityKilled {
        entity: Entity,
        by: Option<Entity>,
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
    ItemTaken {
        taker: Entity,
        item: Entity,
    },
    ItemEquipped {
        wearer: Entity,
        item: Entity,
        slot: BodySlot,
    },
    ItemUnequipped {
        wearer: Entity,
        item: Entity,
        slot: BodySlot,
    },
    ItemDropped {
        dropper: Entity,
        item: Entity,
        at: Pos,
    },
    BodyPartWounded {
        entity: Entity,
        part: BodyPartKind,
        damage: i32,
        status: PartStatus,
        weapon: String,
    },
    BodyPartDestroyed {
        entity: Entity,
        part: BodyPartKind,
        status: PartStatus,
        weapon: String,
    },
    EntityUsed {
        user: Entity,
        target: Entity,
    },
    TaskFailed {
        entity: Entity,
        reason: &'static str,
    },
    Slipped {
        entity: Entity,
        hazard: String,
        prone_ticks: u32,
    },
}

#[derive(Resource, Default)]
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

/// Render a single event as a sentence using the bevy world for entity
/// labels (looks up `Kind` components).
pub fn narrate(event: &Event, world: &World) -> String {
    match event {
        Event::Note(msg) => msg.clone(),
        Event::EntitySpawned {
            entity: _,
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
        Event::EntityMoved { entity, from, to } => {
            let l = label(*entity, world);
            let dir = direction(*from, *to);
            format!("{l} steps {dir} to ({}, {}, {}).", to.x, to.y, to.z)
        }
        Event::EntityAttacked {
            attacker,
            target,
            damage,
            remaining_health,
        } => {
            let attacker_label = attacker
                .map(|a| label(a, world))
                .unwrap_or_else(|| "Something unseen".into());
            let target_label = label(*target, world);
            format!(
                "{attacker_label} strikes {target_label} for {damage} damage (target now at {remaining_health} hp)."
            )
        }
        Event::EntityKilled { entity, by } => {
            let target_label = label(*entity, world);
            match by {
                Some(b) => {
                    let by_label = label(*b, world);
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
        Event::ItemTaken { taker, item } => {
            format!(
                "{} picks up {}.",
                label(*taker, world),
                item_label(*item, world)
            )
        }
        Event::ItemEquipped {
            wearer,
            item,
            slot,
        } => format!(
            "{} equips {} on the {}.",
            label(*wearer, world),
            item_label(*item, world),
            slot.label()
        ),
        Event::ItemUnequipped {
            wearer,
            item,
            slot,
        } => format!(
            "{} removes {} from the {}.",
            label(*wearer, world),
            item_label(*item, world),
            slot.label()
        ),
        Event::ItemDropped { dropper, item, at } => format!(
            "{} drops {} at ({}, {}, {}).",
            label(*dropper, world),
            item_label(*item, world),
            at.x,
            at.y,
            at.z
        ),
        Event::BodyPartWounded {
            entity,
            part,
            damage,
            status,
            weapon,
        } => format!(
            "{}'s {} is {} by the {} ({} dmg).",
            label(*entity, world),
            part.label(),
            status.label(),
            weapon,
            damage,
        ),
        Event::EntityUsed { user, target } => format!(
            "{} interacts with {}.",
            label(*user, world),
            label(*target, world),
        ),
        Event::TaskFailed { entity, reason } => format!(
            "{}'s task fails: {reason}.",
            label(*entity, world),
        ),
        Event::Slipped {
            entity,
            hazard,
            prone_ticks,
        } => format!(
            "{} slips on the {} and crashes to the floor (prone for {} ticks).",
            label(*entity, world),
            hazard,
            prone_ticks,
        ),
        Event::BodyPartDestroyed {
            entity,
            part,
            status,
            weapon,
        } => match status {
            PartStatus::Severed => format!(
                "{}'s {} is sheared off by the {}!",
                label(*entity, world),
                part.label(),
                weapon,
            ),
            PartStatus::Crushed => format!(
                "{}'s {} is crushed beyond use by the {}.",
                label(*entity, world),
                part.label(),
                weapon,
            ),
            other => format!(
                "{}'s {} is {} by the {}.",
                label(*entity, world),
                part.label(),
                other.label(),
                weapon,
            ),
        },
    }
}

fn item_label(item: Entity, world: &World) -> String {
    match world.get::<ItemName>(item) {
        Some(n) => format!("a {}", n.0),
        None => format!("item#{}", item.index()),
    }
}

fn label(entity: Entity, world: &World) -> String {
    match world.get::<Kind>(entity) {
        Some(k) => format!("{}#{}", k.0, entity.index()),
        None => format!("entity#{}", entity.index()),
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
