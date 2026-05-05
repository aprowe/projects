//! Sound emission and hearing.
//!
//! Sound is the engine's way of letting *unrelated* entities react to
//! events that happened across the map: a scream alerts neighbors, a
//! gunshot summons zombies, footsteps give a sneaking thief away.
//!
//! Two halves:
//!
//! - **Emission.** Engine systems watch the existing event log and
//!   add `Event::SoundEmitted { source, position, kind, intensity }`.
//!   `emit_combat_sounds` derives screams + blow-impacts from
//!   `EntityAttacked`. `emit_movement_sounds` derives footsteps from
//!   `EntityMoved`, scaling with the actor's `Locomotion`.
//!
//! - **Hearing.** `update_hearing` walks every entity with a
//!   `Hearing` component, finds this tick's `SoundEmitted` events
//!   within its range (modulated by sound intensity), and writes a
//!   fresh `Perceived { heard: Vec<HeardSound> }` component on the
//!   listener. Scenarios consume `Perceived.heard` in their planners.
//!
//! Run order in a scenario schedule:
//!
//! ```text
//! ... planners, execute_tasks, footing_check, retaliation_system ...
//! emit_combat_sounds,
//! emit_movement_sounds,
//! update_hearing,
//! ... scenario reactions that read `Perceived` ...
//! ```

use bevy_ecs::prelude::{Component, Entity, World};

use crate::components::Position;
use crate::log::{Event, EventLog};
use crate::physics::Locomotion;
use crate::time::Clock;
use crate::world::Pos;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SoundKind {
    /// Cry of pain or terror — emitted by combat victims and
    /// panicking civilians.
    Scream,
    /// A weapon striking flesh or material — emitted by attackers.
    Blow,
    /// Movement. Quiet for sneaking, loud for running.
    Footstep,
    /// Conversation, command, ordinary speech.
    Speech,
    /// Loud — gunshot, explosion, breaking glass.
    Bang,
    /// Catch-all for scenario-specific sounds.
    Other(String),
}

impl SoundKind {
    pub fn label(&self) -> &str {
        match self {
            SoundKind::Scream => "scream",
            SoundKind::Blow => "blow",
            SoundKind::Footstep => "footstep",
            SoundKind::Speech => "speech",
            SoundKind::Bang => "bang",
            SoundKind::Other(s) => s.as_str(),
        }
    }

    /// Whether the prose narrator should include this sound in the
    /// per-tick paragraph. Footsteps and ordinary speech are too noisy.
    pub fn is_narrated(&self) -> bool {
        matches!(self, SoundKind::Scream | SoundKind::Bang)
    }
}

/// How well the entity hears. `range` is the maximum tile distance
/// at which a unit-intensity sound is perceived. Intensity attenuates
/// the effective range linearly.
#[derive(Component, Copy, Clone, Debug)]
pub struct Hearing {
    pub range: i32,
}

impl Hearing {
    pub fn keen() -> Self {
        Self { range: 24 }
    }
    pub fn normal() -> Self {
        Self { range: 14 }
    }
    pub fn dull() -> Self {
        Self { range: 6 }
    }
}

/// How well the entity sees. Voxel walls block line of sight.
#[derive(Component, Copy, Clone, Debug)]
pub struct Sight {
    pub range: i32,
}

impl Sight {
    pub fn keen() -> Self {
        Self { range: 24 }
    }
    pub fn normal() -> Self {
        Self { range: 14 }
    }
    pub fn dim() -> Self {
        Self { range: 6 }
    }
}

/// How well the entity smells. Picks up odor from `Coating`
/// materials via the materials' `smell_intensity` field.
#[derive(Component, Copy, Clone, Debug)]
pub struct Smell {
    pub range: i32,
}

impl Smell {
    pub fn keen() -> Self {
        Self { range: 30 }
    }
    pub fn normal() -> Self {
        Self { range: 10 }
    }
    pub fn dull() -> Self {
        Self { range: 3 }
    }
}

#[derive(Clone, Debug)]
pub struct SeenEntity {
    pub entity: Entity,
    pub position: Pos,
    pub distance: i32,
}

#[derive(Clone, Debug)]
pub struct SmelledOdor {
    pub origin: Pos,
    pub material_name: String,
    pub apparent_intensity: f32,
    pub distance: i32,
}

#[derive(Clone, Debug)]
pub struct HeardSound {
    pub source: Option<Entity>,
    pub origin: Pos,
    pub kind: SoundKind,
    /// Distance-attenuated intensity in `[0, 1]`. 1.0 = right next to
    /// you, dropping to 0 at the edge of audibility.
    pub apparent_intensity: f32,
    pub distance: i32,
}

#[derive(Component, Default, Debug)]
pub struct Perceived {
    pub heard: Vec<HeardSound>,
    pub seen: Vec<SeenEntity>,
    pub smelled: Vec<SmelledOdor>,
}

impl Perceived {
    pub fn loudest_violent(&self) -> Option<&HeardSound> {
        self.heard
            .iter()
            .filter(|s| matches!(s.kind, SoundKind::Scream | SoundKind::Blow | SoundKind::Bang))
            .max_by(|a, b| {
                a.apparent_intensity
                    .partial_cmp(&b.apparent_intensity)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    pub fn has_violent(&self) -> bool {
        self.loudest_violent().is_some()
    }

    pub fn nearest_seen(&self) -> Option<&SeenEntity> {
        self.seen.iter().min_by_key(|s| s.distance)
    }

    /// The strongest odor currently sensed.
    pub fn strongest_smell(&self) -> Option<&SmelledOdor> {
        self.smelled.iter().max_by(|a, b| {
            a.apparent_intensity
                .partial_cmp(&b.apparent_intensity)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

// ─── emission systems ──────────────────────────────────────────────────────

/// Emit Scream + Blow sounds whenever combat lands a hit.
pub fn emit_combat_sounds(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let attacks: Vec<(Option<Entity>, Entity, i32)> = world
        .resource::<EventLog>()
        .events_at(tick)
        .filter_map(|e| match e {
            Event::EntityAttacked {
                attacker,
                target,
                damage,
                ..
            } => Some((*attacker, *target, *damage)),
            _ => None,
        })
        .collect();

    for (attacker, target, damage) in attacks {
        // Victim's scream — intensity rises with damage.
        if let Some(target_pos) = world.get::<Position>(target).map(|p| p.0) {
            let intensity = ((damage as f32 / 30.0) + 0.2).clamp(0.2, 1.0);
            world.resource_mut::<EventLog>().push(
                tick,
                Event::SoundEmitted {
                    source: Some(target),
                    position: target_pos,
                    kind: SoundKind::Scream,
                    intensity,
                },
            );
        }
        // Attacker's blow — flatter intensity, depends on weapon mass
        // (deferred — for now use a fixed mid-loudness blow).
        if let Some(att) = attacker {
            if let Some(att_pos) = world.get::<Position>(att).map(|p| p.0) {
                world.resource_mut::<EventLog>().push(
                    tick,
                    Event::SoundEmitted {
                        source: Some(att),
                        position: att_pos,
                        kind: SoundKind::Blow,
                        intensity: 0.5,
                    },
                );
            }
        }
    }
}

/// Emit Footstep sounds from this tick's `EntityMoved` events,
/// scaled by the mover's `Locomotion`.
pub fn emit_movement_sounds(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let moves: Vec<(Entity, Pos)> = world
        .resource::<EventLog>()
        .events_at(tick)
        .filter_map(|e| match e {
            Event::EntityMoved { entity, to, .. } => Some((*entity, *to)),
            _ => None,
        })
        .collect();

    for (entity, pos) in moves {
        let intensity = movement_loudness(world, entity);
        if intensity == 0.0 {
            continue;
        }
        world.resource_mut::<EventLog>().push(
            tick,
            Event::SoundEmitted {
                source: Some(entity),
                position: pos,
                kind: SoundKind::Footstep,
                intensity,
            },
        );
    }
}

fn movement_loudness(world: &World, entity: Entity) -> f32 {
    let loco = world.get::<Locomotion>(entity).copied().unwrap_or_default();
    match loco {
        Locomotion::Sneaking => 0.0,
        Locomotion::Crawling => 0.05,
        Locomotion::Walking => 0.15,
        Locomotion::Running => 0.45,
        Locomotion::Standing => 0.0,
    }
}

// ─── perception ────────────────────────────────────────────────────────────

/// Walk every entity with `Hearing` and populate a fresh
/// `Perceived { heard }` component from this tick's `SoundEmitted`
/// events. The previous tick's heard list is replaced wholesale.
pub fn update_hearing(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let sounds: Vec<(Option<Entity>, Pos, SoundKind, f32)> = world
        .resource::<EventLog>()
        .events_at(tick)
        .filter_map(|e| match e {
            Event::SoundEmitted {
                source,
                position,
                kind,
                intensity,
            } => Some((*source, *position, kind.clone(), *intensity)),
            _ => None,
        })
        .collect();

    let listeners: Vec<(Entity, Pos, i32)> = {
        let mut q = world.query::<(Entity, &Position, &Hearing)>();
        q.iter(world).map(|(e, p, h)| (e, p.0, h.range)).collect()
    };

    let mut updates: Vec<(Entity, Vec<HeardSound>)> = Vec::with_capacity(listeners.len());
    for (listener, listener_pos, range) in listeners {
        let mut heard: Vec<HeardSound> = Vec::new();
        for (source, origin, kind, intensity) in &sounds {
            if Some(listener) == *source {
                continue; // don't hear yourself
            }
            let dist = listener_pos.chebyshev(*origin);
            let effective_range = (range as f32 * intensity).max(1.0);
            if dist as f32 > effective_range {
                continue;
            }
            let apparent =
                (1.0 - (dist as f32 / effective_range)).clamp(0.0, 1.0) * intensity;
            heard.push(HeardSound {
                source: *source,
                origin: *origin,
                kind: kind.clone(),
                apparent_intensity: apparent,
                distance: dist,
            });
        }
        updates.push((listener, heard));
    }

    for (listener, heard) in updates {
        let exists = world.get::<Perceived>(listener).is_some();
        if exists {
            if let Some(mut p) = world.get_mut::<Perceived>(listener) {
                p.heard = heard;
            }
        } else if !heard.is_empty() {
            world.entity_mut(listener).insert(Perceived {
                heard,
                seen: Vec::new(),
                smelled: Vec::new(),
            });
        }
    }
}

/// Walk every entity with `Sight` and write a fresh
/// `Perceived.seen` list — every other living creature within range
/// whose tile has a clear voxel line of sight from the observer.
pub fn update_sight(world: &mut World) {
    let observers: Vec<(Entity, Pos, i32)> = {
        let mut q = world.query::<(Entity, &Position, &Sight)>();
        q.iter(world).map(|(e, p, s)| (e, p.0, s.range)).collect()
    };
    let candidates: Vec<(Entity, Pos)> = {
        let mut q = world
            .query_filtered::<(Entity, &Position), bevy_ecs::query::With<crate::components::Health>>();
        q.iter(world).map(|(e, p)| (e, p.0)).collect()
    };

    let mut updates: Vec<(Entity, Vec<SeenEntity>)> = Vec::with_capacity(observers.len());
    for (observer, obs_pos, range) in observers {
        let mut seen: Vec<SeenEntity> = Vec::new();
        for (target, t_pos) in &candidates {
            if observer == *target {
                continue;
            }
            let dist = obs_pos.chebyshev(*t_pos);
            if dist > range {
                continue;
            }
            if line_of_sight_blocked(world, obs_pos, *t_pos) {
                continue;
            }
            seen.push(SeenEntity {
                entity: *target,
                position: *t_pos,
                distance: dist,
            });
        }
        seen.sort_by_key(|s| s.distance);
        updates.push((observer, seen));
    }

    for (observer, seen) in updates {
        let exists = world.get::<Perceived>(observer).is_some();
        if exists {
            if let Some(mut p) = world.get_mut::<Perceived>(observer) {
                p.seen = seen;
            }
        } else if !seen.is_empty() {
            world.entity_mut(observer).insert(Perceived {
                heard: Vec::new(),
                seen,
                smelled: Vec::new(),
            });
        }
    }
}

/// Walk every entity with `Smell` and write a fresh
/// `Perceived.smelled` list. Smell sources are `Coating` entities;
/// a coating's odor strength is its material's `smell_intensity`.
/// Apparent intensity attenuates with distance: a sniffer at
/// distance d from a strength-s coating perceives s * (1 - d/(R*s)).
pub fn update_smell(world: &mut World) {
    use crate::physics::Coating;
    use crate::world::VoxelWorld;

    let sniffers: Vec<(Entity, Pos, i32)> = {
        let mut q = world.query::<(Entity, &Position, &Smell)>();
        q.iter(world).map(|(e, p, s)| (e, p.0, s.range)).collect()
    };
    if sniffers.is_empty() {
        return;
    }

    // Snapshot every coating: position + material info, with the
    // coating's current `volume` already folded into the source
    // intensity so a near-evaporated puddle smells weaker.
    let coatings: Vec<(Pos, String, f32)> = {
        let mut q = world.query::<(&Position, &Coating)>();
        let vw = world.resource::<VoxelWorld>();
        q.iter(world)
            .filter_map(|(p, c)| {
                vw.material(c.material).and_then(|mat| {
                    let base = mat.smell_intensity;
                    let effective = base * c.volume.clamp(0.0, 1.0);
                    if effective <= 0.0 {
                        None
                    } else {
                        Some((p.0, mat.name.clone(), effective))
                    }
                })
            })
            .collect()
    };

    let mut updates: Vec<(Entity, Vec<SmelledOdor>)> = Vec::with_capacity(sniffers.len());
    for (sniffer, sniff_pos, range) in sniffers {
        let mut smelled: Vec<SmelledOdor> = Vec::new();
        for (origin, material_name, source_intensity) in &coatings {
            let dist = sniff_pos.chebyshev(*origin);
            let effective_range = (range as f32 * *source_intensity).max(1.0);
            if dist as f32 > effective_range {
                continue;
            }
            let apparent = (1.0 - (dist as f32 / effective_range)).clamp(0.0, 1.0)
                * *source_intensity;
            smelled.push(SmelledOdor {
                origin: *origin,
                material_name: material_name.clone(),
                apparent_intensity: apparent,
                distance: dist,
            });
        }
        smelled.sort_by(|a, b| {
            b.apparent_intensity
                .partial_cmp(&a.apparent_intensity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        updates.push((sniffer, smelled));
    }

    for (sniffer, smelled) in updates {
        let exists = world.get::<Perceived>(sniffer).is_some();
        if exists {
            if let Some(mut p) = world.get_mut::<Perceived>(sniffer) {
                p.smelled = smelled;
            }
        } else if !smelled.is_empty() {
            world.entity_mut(sniffer).insert(Perceived {
                heard: Vec::new(),
                seen: Vec::new(),
                smelled,
            });
        }
    }
}

fn line_of_sight_blocked(world: &World, a: Pos, b: Pos) -> bool {
    use crate::world::VoxelWorld;
    let voxel_world = world.resource::<VoxelWorld>();
    let dx = (b.x - a.x) as f32;
    let dy = (b.y - a.y) as f32;
    let dz = (b.z - a.z) as f32;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    if dist <= 1.5 {
        return false;
    }
    let steps = (dist * 2.0).ceil() as i32;
    for i in 1..steps {
        let t = i as f32 / steps as f32;
        let x = (a.x as f32 + dx * t).round() as i32;
        let y = (a.y as f32 + dy * t).round() as i32;
        let z = (a.z as f32 + dz * t).round() as i32;
        let p = Pos::new(x, y, z);
        if p == a || p == b {
            continue;
        }
        if voxel_world.is_solid(p) {
            return true;
        }
    }
    false
}

/// Convenience for scenarios: emit a scream from `entity`'s position.
/// Used when a panicking civilian yells.
pub fn emit_scream(world: &mut World, entity: Entity, intensity: f32) {
    let pos = match world.get::<Position>(entity).map(|p| p.0) {
        Some(p) => p,
        None => return,
    };
    let tick = world.resource::<Clock>().tick;
    world.resource_mut::<EventLog>().push(
        tick,
        Event::SoundEmitted {
            source: Some(entity),
            position: pos,
            kind: SoundKind::Scream,
            intensity: intensity.clamp(0.0, 1.0),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::EventLog;
    use crate::time::Clock;

    fn fresh_world() -> World {
        let mut world = World::new();
        world.insert_resource(Clock::default());
        world.insert_resource(EventLog::default());
        world
    }

    #[test]
    fn loud_sound_close_by_is_audible() {
        let mut world = fresh_world();
        let listener = world
            .spawn((Position(Pos::new(0, 0, 0)), Hearing::normal()))
            .id();
        // Simulate a sound emitted at distance 3.
        world.resource_mut::<EventLog>().push(
            0,
            Event::SoundEmitted {
                source: None,
                position: Pos::new(3, 0, 0),
                kind: SoundKind::Scream,
                intensity: 1.0,
            },
        );
        update_hearing(&mut world);
        let p = world.get::<Perceived>(listener).expect("Perceived");
        assert_eq!(p.heard.len(), 1);
        assert_eq!(p.heard[0].kind, SoundKind::Scream);
        assert_eq!(p.heard[0].distance, 3);
    }

    #[test]
    fn quiet_sound_far_away_is_not_heard() {
        let mut world = fresh_world();
        let listener = world
            .spawn((Position(Pos::new(0, 0, 0)), Hearing::normal()))
            .id();
        world.resource_mut::<EventLog>().push(
            0,
            Event::SoundEmitted {
                source: None,
                position: Pos::new(20, 0, 0),
                kind: SoundKind::Footstep,
                intensity: 0.1, // quiet
            },
        );
        update_hearing(&mut world);
        let p = world.get::<Perceived>(listener);
        // Either Perceived is absent or its heard list is empty.
        assert!(p.map(|p| p.heard.is_empty()).unwrap_or(true));
    }

    #[test]
    fn entity_does_not_hear_itself() {
        let mut world = fresh_world();
        let me = world
            .spawn((Position(Pos::new(0, 0, 0)), Hearing::normal()))
            .id();
        world.resource_mut::<EventLog>().push(
            0,
            Event::SoundEmitted {
                source: Some(me),
                position: Pos::new(0, 0, 0),
                kind: SoundKind::Scream,
                intensity: 1.0,
            },
        );
        update_hearing(&mut world);
        let p = world.get::<Perceived>(me);
        assert!(p.map(|p| p.heard.is_empty()).unwrap_or(true));
    }
}
