//! Structured event log for the simulation.
//!
//! The `EventLog` is a bevy ECS `Resource` that lives on the simulation
//! world. Engine helpers and scenario systems push typed events onto it;
//! renderers read from it to produce narration and post-run summaries.

use bevy_ecs::prelude::{Entity, Resource, World};

use crate::anatomy::{BodyPartKind, PartStatus};
use crate::components::Kind;
use crate::items::{BodySlot, ItemName};
use crate::sound::SoundKind;
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
    /// Emitted when a creature with `Fear` crosses the "terrified"
    /// threshold for the first time during a combat exchange.
    Terrified {
        entity: Entity,
    },
    /// A sound emitted at a position. Read by `update_hearing` to
    /// populate `Perceived` components on listeners. Footsteps and
    /// ordinary speech are intentionally noisy to support
    /// stealth/perception scenarios; the prose narrator filters
    /// these out.
    SoundEmitted {
        source: Option<Entity>,
        position: Pos,
        kind: SoundKind,
        intensity: f32,
    },
    /// A swing that didn't land. D&D-style: attack roll didn't meet AC.
    AttackMissed {
        attacker: Entity,
        target: Entity,
        attack_roll: i32,
        target_ac: i32,
        weapon: String,
    },
    /// Natural 20 — emitted alongside the resulting `EntityAttacked`
    /// + `BodyPartWounded`.
    CriticalHit {
        attacker: Entity,
        target: Entity,
        weapon: String,
    },
    /// A creature attempted a manipulation/strength check on an
    /// object (door, lock, lid) and the result was determined.
    AbilityCheck {
        actor: Entity,
        kind: String,        // "manipulation", "strength", ...
        target: String,      // "the front door", "the dresser drawer"
        roll: i32,
        dc: i32,
        success: bool,
        impossible: bool,
    },
    /// A door changed state (opened, slammed, lockpicked, kicked in).
    DoorStateChanged {
        door: Entity,
        new_state: String,
        cause: String,
    },
    /// A scripted dialog line played. `listener = None` means the
    /// speaker is talking to themselves / out loud.
    Spoke {
        speaker: Entity,
        listener: Option<Entity>,
        text: String,
    },
    /// A question posed to a specific listener.
    Asked {
        speaker: Entity,
        listener: Entity,
        question: String,
    },
    /// A deceptive line. `believability` is in [0, 1]; less believable
    /// lines bump observer suspicion.
    Lied {
        speaker: Entity,
        listener: Entity,
        text: String,
        believability: f32,
    },
    /// An observer's suspicion of a target changed because of a
    /// disguise mismatch, knowledge cross-check, or shaky story.
    Observed {
        observer: Entity,
        target: Entity,
        suspicion: f32,
        reason: String,
    },
    /// An observer's suspicion crossed the alarm threshold. They are
    /// no longer fooled and will act on it.
    AlarmRaised {
        observer: Entity,
        target: Entity,
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
            "{} slips on {} and goes down (prone for {} ticks).",
            label(*entity, world),
            hazard,
            prone_ticks,
        ),
        Event::Terrified { entity } => {
            format!("{} is wide-eyed with terror.", label(*entity, world))
        }
        Event::AttackMissed {
            attacker,
            target,
            attack_roll,
            target_ac,
            weapon,
        } => format!(
            "{} swings the {} at {} — rolled {} vs AC {}, miss.",
            label(*attacker, world),
            weapon,
            label(*target, world),
            attack_roll,
            target_ac,
        ),
        Event::CriticalHit {
            attacker,
            target,
            weapon,
        } => format!(
            "Natural 20! {} lands a critical blow on {} with the {}.",
            label(*attacker, world),
            label(*target, world),
            weapon,
        ),
        Event::AbilityCheck {
            actor,
            kind,
            target,
            roll,
            dc,
            success,
            impossible,
        } => {
            if *impossible {
                format!(
                    "{} tries to {} {} but can't — no working hands.",
                    label(*actor, world),
                    kind,
                    target,
                )
            } else if *success {
                format!(
                    "{} {} {} (rolled {} vs DC {} — success).",
                    label(*actor, world),
                    kind,
                    target,
                    roll,
                    dc,
                )
            } else {
                format!(
                    "{} fumbles trying to {} {} (rolled {} vs DC {} — failure).",
                    label(*actor, world),
                    kind,
                    target,
                    roll,
                    dc,
                )
            }
        }
        Event::DoorStateChanged { door: _, new_state, cause } => {
            format!("A door is now {new_state} ({cause}).")
        }
        Event::Spoke { speaker, listener, text } => match listener {
            Some(l) => format!(
                "{} says to {}: \"{}\"",
                label(*speaker, world),
                label(*l, world),
                text,
            ),
            None => format!("{} mutters: \"{}\"", label(*speaker, world), text),
        },
        Event::Asked { speaker, listener, question } => format!(
            "{} asks {}: \"{}\"",
            label(*speaker, world),
            label(*listener, world),
            question,
        ),
        Event::Lied { speaker, listener, text, believability } => format!(
            "{} tells {} (lie, believability {:.2}): \"{}\"",
            label(*speaker, world),
            label(*listener, world),
            believability,
            text,
        ),
        Event::Observed { observer, target, suspicion, reason } => format!(
            "{} eyes {} ({}, suspicion {:.2} — {}).",
            label(*observer, world),
            label(*target, world),
            crate::dialog::Suspicion::label(*suspicion),
            suspicion,
            reason,
        ),
        Event::AlarmRaised { observer, target } => format!(
            "{} raises the alarm — {} is exposed!",
            label(*observer, world),
            label(*target, world),
        ),
        Event::SoundEmitted {
            kind,
            position,
            source: _,
            intensity: _,
        } => {
            if !kind.is_narrated() {
                return String::new();
            }
            match kind {
                SoundKind::Scream => format!(
                    "A scream pierces the air from ({}, {}, {}).",
                    position.x, position.y, position.z
                ),
                SoundKind::Bang => format!(
                    "A loud bang echoes from ({}, {}, {}).",
                    position.x, position.y, position.z
                ),
                _ => String::new(),
            }
        }
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
