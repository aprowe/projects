//! Dialog, identity, knowledge, and suspicion.
//!
//! Lays the groundwork for stealth/social-engineering scenarios in
//! the spirit of Hitman: an actor can wear a `Disguise`, an NPC can
//! `Observe` them and check the disguise against its own `Knowledge`
//! (who's allowed where, what a real pilot's badge looks like), and
//! mismatches accumulate `Suspicion` until the cover blows.
//!
//! Dialog itself is intentionally small: one-shot `Utterance` events
//! and a `Conversation` task that drains a queued script line per
//! tick. NPCs react to lines via systems registered by the scenario
//! — the engine doesn't try to "understand" speech.
//!
//! The whole module is opt-in: a scenario adds `Knowledge`,
//! `Disguise`, `Suspicion`, etc. only on the entities that need them.
//! Nothing here runs unless the scenario inserts the components and
//! schedules the systems.

use std::collections::HashMap;
use std::collections::VecDeque;

use bevy_ecs::prelude::{Component, Entity, World};

use crate::components::Position;
use crate::log::{Event, EventLog};
use crate::time::Clock;

// ─── identity ───────────────────────────────────────────────────────────────

/// The "true" identity an entity carries in its bones — the name and
/// role it actually has, regardless of how it currently presents.
/// Distinct from the free-form `Kind` tag (which is a debug label).
#[derive(Component, Clone, Debug)]
pub struct Identity {
    pub name: String,
    pub role: String,
    pub clearance: Clearance,
}

impl Identity {
    pub fn new(name: impl Into<String>, role: impl Into<String>, clearance: Clearance) -> Self {
        Self {
            name: name.into(),
            role: role.into(),
            clearance,
        }
    }
}

/// Tiered access. NPCs check `presented_clearance >= zone_required`
/// when deciding whether to challenge an entity that crossed a
/// checkpoint. Layered, so `Pilot` implies `Crew` implies `Public`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Clearance {
    Public,
    Crew,
    Pilot,
    Restricted,
}

impl Clearance {
    pub fn label(self) -> &'static str {
        match self {
            Clearance::Public => "public",
            Clearance::Crew => "crew",
            Clearance::Pilot => "pilot",
            Clearance::Restricted => "restricted",
        }
    }
}

/// What an entity is currently *presenting* as. A spy walks in with
/// their own `Identity` plus a `Disguise` claiming "Captain Hayes,
/// pilot". Observers see the `Disguise` first; if its clearance
/// matches the zone they're guarding, they wave the spy through.
///
/// The "quality" field (0.0–1.0) is how convincing the costume is.
/// 1.0 is a real uniform with a real badge; 0.6 is a reasonable
/// approximation; 0.2 is a hi-vis vest from a hardware store. Used
/// as a modifier on observation rolls.
#[derive(Component, Clone, Debug)]
pub struct Disguise {
    pub presented_name: String,
    pub presented_role: String,
    pub presented_clearance: Clearance,
    pub quality: f32,
}

impl Disguise {
    pub fn new(
        name: impl Into<String>,
        role: impl Into<String>,
        clearance: Clearance,
        quality: f32,
    ) -> Self {
        Self {
            presented_name: name.into(),
            presented_role: role.into(),
            presented_clearance: clearance,
            quality: quality.clamp(0.0, 1.0),
        }
    }
}

// ─── knowledge ─────────────────────────────────────────────────────────────

/// What a particular NPC knows. This is per-entity — different NPCs
/// know different things. The TSA agent guarding gate B7 knows the
/// flight manifest for B7 but not for B12; a pilot knows other
/// pilots' faces but not the cleaning staff's schedule.
///
/// Stored as a small key/value bag so scenarios can extend it
/// without engine changes. Common keys (by convention):
///
/// - `"manifest:gate_b7"` → comma-separated names allowed at B7
/// - `"face:captain_hayes"` → "balding, 50s, glasses" (a description
///   the NPC can compare to what they actually see)
/// - `"badge_color:pilot"` → "navy_blue" (what a real pilot badge
///   looks like in this airport's uniform code)
/// - `"protocol:ask_origin_city"` → "yes" (NPC's small-talk pattern)
#[derive(Component, Default, Clone, Debug)]
pub struct Knowledge(pub HashMap<String, String>);

impl Knowledge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.0.insert(key.into(), value.into());
        self
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|s| s.as_str())
    }

    pub fn knows(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0.insert(key.into(), value.into());
    }
}

// ─── suspicion ──────────────────────────────────────────────────────────────

/// How suspicious one entity is of others. Per-target rather than a
/// flat scalar — guard A might trust the player while guard B has
/// already started watching them. Values are 0.0 (oblivious) → 1.0
/// (sounding the alarm).
///
/// Scenarios decide thresholds. By convention:
///
/// - `0.00 .. 0.25` — calm. Default greetings.
/// - `0.25 .. 0.50` — curious. Will make small talk to probe.
/// - `0.50 .. 0.80` — suspicious. Asks for ID, watches the spy
///   move.
/// - `0.80 ..` — alarmed. Calls security, blocks the door.
#[derive(Component, Default, Clone, Debug)]
pub struct Suspicion(pub HashMap<Entity, f32>);

impl Suspicion {
    pub fn level(&self, target: Entity) -> f32 {
        self.0.get(&target).copied().unwrap_or(0.0)
    }

    pub fn raise(&mut self, target: Entity, delta: f32) -> f32 {
        let v = (self.level(target) + delta).clamp(0.0, 1.0);
        self.0.insert(target, v);
        v
    }

    pub fn lower(&mut self, target: Entity, delta: f32) -> f32 {
        let v = (self.level(target) - delta).clamp(0.0, 1.0);
        self.0.insert(target, v);
        v
    }

    pub fn label(level: f32) -> &'static str {
        if level < 0.25 {
            "calm"
        } else if level < 0.50 {
            "curious"
        } else if level < 0.80 {
            "suspicious"
        } else {
            "alarmed"
        }
    }
}

/// Marker: this NPC is currently raising the alarm — they've spotted
/// the spy and are no longer fooled. Scenarios can set planners to
/// pursue / call backup when this is present.
#[derive(Component, Copy, Clone, Debug)]
pub struct Alarmed;

// ─── observation ────────────────────────────────────────────────────────────

/// Tag: this entity actively scans nearby creatures for suspicious
/// signs. The `observation_system` runs once per tick and, for each
/// `Observer` adjacent to (or within `range`) other entities, pushes
/// `Event::Observed` describing what the observer sees, and bumps
/// `Suspicion` if the disguise + clearance combo doesn't match the
/// observer's local `Knowledge`.
///
/// `expected_clearance`: the minimum clearance level allowed in the
/// zone this observer is guarding. Anyone presenting less than this
/// gets challenged immediately.
///
/// `guarded_y_min`: candidates with `pos.y < guarded_y_min` are
/// considered "still on the public side" and are not inspected. A
/// 24/7 zone with no public side is `i32::MIN`.
#[derive(Component, Clone, Debug)]
pub struct Observer {
    pub range: i32,
    pub expected_clearance: Clearance,
    pub zone_label: String,
    pub guarded_y_min: i32,
}

impl Observer {
    pub fn new(range: i32, expected_clearance: Clearance, zone_label: impl Into<String>) -> Self {
        Self {
            range,
            expected_clearance,
            zone_label: zone_label.into(),
            guarded_y_min: i32::MIN,
        }
    }

    /// Restrict inspections to candidates whose `pos.y` is at or past
    /// the given line (south, in the airport scenario's coordinate
    /// frame). Useful for one-way checkpoints.
    pub fn with_guarded_y(mut self, y_min: i32) -> Self {
        self.guarded_y_min = y_min;
        self
    }
}

/// Engine system: each tick, every `Observer` looks at every entity
/// within `range` and updates its own `Suspicion` of them based on
/// presented vs. required clearance, and on `Knowledge` mismatches.
///
/// "Presented identity" = `Disguise` if one is worn, otherwise the
/// entity's own `Identity`. Entities without an `Identity` are
/// invisible to social observation (they're props, not people).
///
/// Bumps:
/// - `+0.30` if the presented clearance is below the zone threshold
///   (a stranger in a restricted zone).
/// - `+0.20 * (1.0 - quality)` for cheap disguises (the cleaner the
///   uniform, the less the bump). Identity-as-presentation has
///   quality 1.0 — a real badge is unimpeachable.
/// - `+0.30` if the observer's `Knowledge` has a `"face:<presented_name>"`
///   entry — meaning the observer personally knows the real person —
///   and the candidate's true `Identity.name` doesn't match.
/// - `-0.05` if the candidate's clearance is at or above the
///   observer's threshold and nothing else is wrong (the disguise is
///   working; suspicion drifts down).
///
/// Once suspicion crosses 0.80, inserts `Alarmed` on the observer
/// and emits `Event::AlarmRaised`.
pub fn observation_system(world: &mut World) {
    type ObsRow = (Entity, crate::world::Pos, i32, Clearance, String, i32);
    let observers: Vec<ObsRow> = {
        let mut q = world.query::<(Entity, &Position, &Observer)>();
        q.iter(world)
            .map(|(e, p, o)| {
                (
                    e,
                    p.0,
                    o.range,
                    o.expected_clearance,
                    o.zone_label.clone(),
                    o.guarded_y_min,
                )
            })
            .collect()
    };
    type CandRow = (Entity, crate::world::Pos, String, Clearance, f32, String);
    let candidates: Vec<CandRow> = {
        let mut q = world.query::<(Entity, &Position, Option<&Disguise>, &Identity)>();
        q.iter(world)
            .map(|(e, p, d, id)| {
                let (name, clearance, quality) = match d {
                    Some(d) => (
                        d.presented_name.clone(),
                        d.presented_clearance,
                        d.quality,
                    ),
                    None => (id.name.clone(), id.clearance, 1.0),
                };
                (e, p.0, name, clearance, quality, id.name.clone())
            })
            .collect()
    };

    for (observer, observer_pos, range, expected, zone_label, guarded_y_min) in observers {
        if world.get::<Alarmed>(observer).is_some() {
            continue;
        }
        for (candidate, cand_pos, presented_name, presented_clearance, quality, true_name)
            in &candidates
        {
            if *candidate == observer {
                continue;
            }
            // Don't suspect other guards going about their business.
            if world.get::<Observer>(*candidate).is_some() {
                continue;
            }
            if cand_pos.y < guarded_y_min {
                continue;
            }
            let dx = (cand_pos.x - observer_pos.x).abs();
            let dy = (cand_pos.y - observer_pos.y).abs();
            let dz = (cand_pos.z - observer_pos.z).abs();
            let cheb = dx.max(dy).max(dz);
            if cheb > range {
                continue;
            }

            let knows_face = world
                .get::<Knowledge>(observer)
                .map(|k| k.knows(&format!("face:{presented_name}")))
                .unwrap_or(false);

            let mut delta = 0.0_f32;
            let mut reason = String::new();
            if *presented_clearance < expected {
                delta += 0.30;
                reason = format!(
                    "{} clearance in {zone_label}",
                    presented_clearance.label()
                );
            }
            delta += 0.20 * (1.0 - quality);
            if knows_face && *presented_name != *true_name {
                delta += 0.30;
                if reason.is_empty() {
                    reason = format!("face doesn't match \"{presented_name}\"");
                } else {
                    reason = format!("{reason}; face mismatch");
                }
            }
            if delta == 0.0 && *presented_clearance >= expected && !knows_face {
                delta -= 0.05;
            }
            if delta == 0.0 {
                continue;
            }

            if world.get::<Suspicion>(observer).is_none() {
                world.entity_mut(observer).insert(Suspicion::default());
            }
            let new_level = {
                let mut sus = world
                    .get_mut::<Suspicion>(observer)
                    .expect("inserted above");
                if delta >= 0.0 {
                    sus.raise(*candidate, delta)
                } else {
                    sus.lower(*candidate, -delta)
                }
            };

            if !reason.is_empty() {
                push_event(
                    world,
                    Event::Observed {
                        observer,
                        target: *candidate,
                        suspicion: new_level,
                        reason,
                    },
                );
            }

            if new_level >= 0.80 && world.get::<Alarmed>(observer).is_none() {
                world.entity_mut(observer).insert(Alarmed);
                push_event(
                    world,
                    Event::AlarmRaised {
                        observer,
                        target: *candidate,
                    },
                );
                break;
            }
        }
    }
}

// ─── conversation ───────────────────────────────────────────────────────────

/// One scripted line. The dialog scheduler emits this as a single
/// `Utterance` event and advances. Use `Speak` for plain dialog,
/// `Lie` to flag the line as deceptive (so observation systems can
/// score it), or `Ask` to mark it as an information request — NPC
/// listeners observe the question and may respond next tick by
/// queueing their own conversation.
#[derive(Clone, Debug)]
pub enum DialogLine {
    Speak {
        listener: Option<Entity>,
        text: String,
    },
    Ask {
        listener: Entity,
        question: String,
    },
    Lie {
        listener: Entity,
        text: String,
        believability: f32,
    },
    /// Pause for `ticks` before saying the next line. Useful for
    /// pacing — a beat of awkward silence.
    Pause(u32),
}

impl DialogLine {
    pub fn label(&self) -> String {
        match self {
            DialogLine::Speak { text, .. } => format!("Speak({text:?})"),
            DialogLine::Ask { question, .. } => format!("Ask({question:?})"),
            DialogLine::Lie { text, believability, .. } => {
                format!("Lie({text:?}, p={believability:.2})")
            }
            DialogLine::Pause(t) => format!("Pause({t})"),
        }
    }
}

/// A scripted multi-line conversation an actor wants to say. The
/// `dialog_system` drains one entry per tick and turns it into a
/// `Spoke` / `Lied` / `Asked` event. When empty, the component is
/// removed.
#[derive(Component, Default, Debug)]
pub struct Conversation(pub VecDeque<DialogLine>);

impl Conversation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, line: DialogLine) -> &mut Self {
        self.0.push_back(line);
        self
    }

    pub fn extend(&mut self, lines: impl IntoIterator<Item = DialogLine>) -> &mut Self {
        self.0.extend(lines);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Engine system: drains one `DialogLine` per actor per tick.
/// Emits `Event::Spoke`, `Event::Lied`, or `Event::Asked`. Pauses
/// rewrite themselves with a decremented counter.
///
/// Lies bump observer suspicion if the listener happens to be an
/// `Observer` and `believability < 1.0`. The believability is
/// applied as `0.20 * (1.0 - believability)` — same formula as
/// disguise quality, so a totally convincing lie is free and a
/// terrible one shows up immediately on the suspicion meter.
pub fn dialog_system(world: &mut World) {
    let speakers: Vec<Entity> = {
        let mut q = world.query_filtered::<Entity, bevy_ecs::query::With<Conversation>>();
        q.iter(world).collect()
    };

    for speaker in speakers {
        let next = {
            let mut conv = match world.get_mut::<Conversation>(speaker) {
                Some(c) => c,
                None => continue,
            };
            conv.0.pop_front()
        };
        let Some(line) = next else {
            world.entity_mut(speaker).remove::<Conversation>();
            continue;
        };

        match line {
            DialogLine::Speak { listener, text } => {
                push_event(
                    world,
                    Event::Spoke {
                        speaker,
                        listener,
                        text,
                    },
                );
            }
            DialogLine::Ask { listener, question } => {
                push_event(
                    world,
                    Event::Asked {
                        speaker,
                        listener,
                        question,
                    },
                );
            }
            DialogLine::Lie {
                listener,
                text,
                believability,
            } => {
                push_event(
                    world,
                    Event::Lied {
                        speaker,
                        listener,
                        text,
                        believability,
                    },
                );
                if world.get::<Observer>(listener).is_some() {
                    let bump = 0.20 * (1.0 - believability.clamp(0.0, 1.0));
                    if bump > 0.0 {
                        if world.get::<Suspicion>(listener).is_none() {
                            world.entity_mut(listener).insert(Suspicion::default());
                        }
                        let mut sus = world
                            .get_mut::<Suspicion>(listener)
                            .expect("inserted above");
                        let new_level = sus.raise(speaker, bump);
                        push_event(
                            world,
                            Event::Observed {
                                observer: listener,
                                target: speaker,
                                suspicion: new_level,
                                reason: "shaky story".into(),
                            },
                        );
                    }
                }
            }
            DialogLine::Pause(t) => {
                if t > 1 {
                    if let Some(mut conv) = world.get_mut::<Conversation>(speaker) {
                        conv.0.push_front(DialogLine::Pause(t - 1));
                    }
                }
            }
        }

        if let Some(conv) = world.get::<Conversation>(speaker) {
            if conv.0.is_empty() {
                world.entity_mut(speaker).remove::<Conversation>();
            }
        }
    }
}

fn push_event(world: &mut World, event: Event) {
    let tick = world.resource::<Clock>().tick;
    world.resource_mut::<EventLog>().push(tick, event);
}
