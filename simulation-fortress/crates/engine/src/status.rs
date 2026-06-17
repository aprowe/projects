//! Status effects: temporary conditions on creatures.
//!
//! `StatusEffects` is a small Vec of `StatusEffect` entries. Each
//! has a kind, an integer remaining-ticks counter, and an intensity.
//! The `tick_status_effects` system decrements counters once per
//! tick, removes expired entries, and applies per-tick consequences:
//!
//! - `Poisoned`: -1 HP per tick (× intensity).
//! - `OnFire`: -2 HP per tick + smoke; spreads with low chance to
//!   adjacent flammable tiles (handled by scenarios).
//! - `Asphyxiating`: -3 HP per tick.
//! - `Bleeding`: -1 HP per tick; ends when bandaged.
//! - `Stunned`, `Sleeping`, `Drunk`, `Prone`: behavioral, not damage —
//!   read by other systems (no-attack while stunned, etc.).
//! - `Concussed`: similar to drunk + reduced perception.
//!
//! Scenarios apply status by inserting `StatusEffects` (or pushing
//! to an existing one) and the engine ticks them down each frame.

use bevy_ecs::prelude::{Component, Entity, World};

use crate::components::Health;
use crate::log::{Event, EventLog};
use crate::time::Clock;

#[derive(Component, Default, Clone, Debug)]
pub struct StatusEffects(pub Vec<StatusEffect>);

#[derive(Clone, Debug)]
pub struct StatusEffect {
    pub kind: StatusKind,
    pub remaining: u32,
    /// 1 = mild, 3 = severe. Multiplies HP-drain effects.
    pub intensity: u8,
}

impl StatusEffect {
    pub fn new(kind: StatusKind, ticks: u32, intensity: u8) -> Self {
        Self {
            kind,
            remaining: ticks,
            intensity: intensity.clamp(1, 3),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum StatusKind {
    Poisoned,
    OnFire,
    Asphyxiating,
    Bleeding,
    Stunned,
    Sleeping,
    Drunk,
    Prone,
    Concussed,
    Blinded,
    Deafened,
}

impl StatusKind {
    pub fn label(self) -> &'static str {
        match self {
            StatusKind::Poisoned => "poisoned",
            StatusKind::OnFire => "on fire",
            StatusKind::Asphyxiating => "asphyxiating",
            StatusKind::Bleeding => "bleeding",
            StatusKind::Stunned => "stunned",
            StatusKind::Sleeping => "asleep",
            StatusKind::Drunk => "drunk",
            StatusKind::Prone => "prone",
            StatusKind::Concussed => "concussed",
            StatusKind::Blinded => "blinded",
            StatusKind::Deafened => "deafened",
        }
    }

    /// Per-tick HP drain (positive = damage). Multiplied by
    /// intensity at apply-time.
    pub fn hp_drain(self) -> i32 {
        match self {
            StatusKind::Poisoned => 1,
            StatusKind::OnFire => 2,
            StatusKind::Asphyxiating => 3,
            StatusKind::Bleeding => 1,
            _ => 0,
        }
    }

    /// Whether this status prevents the actor from acting (the task
    /// queue still drains but actions are no-ops).
    pub fn incapacitates(self) -> bool {
        matches!(
            self,
            StatusKind::Stunned | StatusKind::Sleeping | StatusKind::Prone
        )
    }
}

/// Convenience: insert or refresh a status on a target.
pub fn apply_status(world: &mut World, target: Entity, kind: StatusKind, ticks: u32, intensity: u8) {
    let mut entity = world.entity_mut(target);
    if !entity.contains::<StatusEffects>() {
        entity.insert(StatusEffects::default());
    }
    let mut effects = entity.get_mut::<StatusEffects>().expect("just inserted");
    if let Some(existing) = effects.0.iter_mut().find(|e| e.kind == kind) {
        existing.remaining = existing.remaining.max(ticks);
        existing.intensity = existing.intensity.max(intensity).clamp(1, 3);
    } else {
        effects.0.push(StatusEffect::new(kind, ticks, intensity));
    }
}

/// Engine system: drain HP for damaging effects, decrement counters,
/// remove expired entries, emit events for crossings.
pub fn tick_status_effects(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let entities: Vec<Entity> = {
        let mut q = world.query_filtered::<Entity, bevy_ecs::query::With<StatusEffects>>();
        q.iter(world).collect()
    };
    for entity in entities {
        let drained: Vec<(StatusKind, i32)> = {
            let effects = match world.get::<StatusEffects>(entity) {
                Some(e) => e,
                None => continue,
            };
            effects
                .0
                .iter()
                .map(|e| (e.kind, e.kind.hp_drain() * e.intensity as i32))
                .filter(|(_, d)| *d > 0)
                .collect()
        };
        for (kind, drain) in drained {
            let killed = {
                let mut h = match world.get_mut::<Health>(entity) {
                    Some(h) => h,
                    None => continue,
                };
                h.current = (h.current - drain).max(0);
                h.current == 0
            };
            let label = label_for(world, entity);
            world.resource_mut::<EventLog>().push(
                tick,
                Event::Note(format!(
                    "{label} suffers {drain} dmg from being {}.",
                    kind.label()
                )),
            );
            if killed {
                world.resource_mut::<EventLog>().push(
                    tick,
                    Event::EntityKilled {
                        entity,
                        by: None,
                    },
                );
            }
        }
        if let Some(mut effects) = world.get_mut::<StatusEffects>(entity) {
            for e in effects.0.iter_mut() {
                e.remaining = e.remaining.saturating_sub(1);
            }
            effects.0.retain(|e| e.remaining > 0);
        }
    }
}

fn label_for(world: &World, entity: Entity) -> String {
    match world.get::<crate::components::Kind>(entity) {
        Some(k) => format!("{}#{}", k.0, entity.index()),
        None => format!("entity#{}", entity.index()),
    }
}
