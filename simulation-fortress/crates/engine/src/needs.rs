//! Needs, drives, and mood.
//!
//! Each need is its own component on a creature so scenarios can opt
//! in to whichever ones make sense (a zombie has no need for sleep; a
//! soldier doesn't need food in a 30-minute skirmish). The `tick_needs`
//! system drifts every need toward its natural direction each tick;
//! scenarios call `feed`, `rest`, `frighten` etc. to push values back
//! the other way.
//!
//! Mood is derived from the other needs at the end of each tick by
//! `derive_mood`. The default scoring mixes hunger, fear, and energy;
//! scenarios can write their own mood system if they want a different
//! formula.

use bevy_ecs::prelude::{Component, Entity, Query, World};

use crate::log::{Event, EventLog};
use crate::time::Clock;

/// Hunger drifts upward toward 1.0; eating drops it back toward 0.
#[derive(Component, Copy, Clone, Debug)]
pub struct Hunger {
    pub current: f32,
    pub rate: f32,
}

impl Hunger {
    pub fn new(rate: f32) -> Self {
        Self {
            current: 0.0,
            rate,
        }
    }

    pub fn satiated() -> Self {
        Self::new(0.01)
    }

    pub fn is_hungry(&self) -> bool {
        self.current >= 0.6
    }

    pub fn is_starving(&self) -> bool {
        self.current >= 0.95
    }

    pub fn feed(&mut self, amount: f32) {
        self.current = (self.current - amount).max(0.0);
    }
}

/// Energy drifts downward toward 0.0; sleeping refills it.
#[derive(Component, Copy, Clone, Debug)]
pub struct Energy {
    pub current: f32,
    pub drain_rate: f32,
}

impl Energy {
    pub fn new(drain_rate: f32) -> Self {
        Self {
            current: 1.0,
            drain_rate,
        }
    }

    pub fn rested() -> Self {
        Self::new(0.005)
    }

    pub fn is_tired(&self) -> bool {
        self.current <= 0.4
    }

    pub fn is_exhausted(&self) -> bool {
        self.current <= 0.05
    }

    pub fn rest(&mut self, amount: f32) {
        self.current = (self.current + amount).min(1.0);
    }
}

/// Fear decays toward 0 each tick; it's pushed up by `frighten`.
#[derive(Component, Copy, Clone, Debug)]
pub struct Fear {
    pub current: f32,
    pub decay_rate: f32,
}

impl Fear {
    pub fn new(decay_rate: f32) -> Self {
        Self {
            current: 0.0,
            decay_rate,
        }
    }

    pub fn calm() -> Self {
        Self::new(0.01)
    }

    pub fn is_afraid(&self) -> bool {
        self.current >= 0.5
    }

    pub fn is_terrified(&self) -> bool {
        self.current >= 0.85
    }

    pub fn frighten(&mut self, amount: f32) {
        self.current = (self.current + amount).min(1.0);
    }
}

/// Mood, in `[-1.0, 1.0]`. Computed by `derive_mood` from the other
/// needs each tick; scenarios can also nudge it directly.
#[derive(Component, Copy, Clone, Debug)]
pub struct Mood {
    pub current: f32,
}

impl Default for Mood {
    fn default() -> Self {
        Self { current: 0.0 }
    }
}

impl Mood {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn label(self) -> &'static str {
        if self.current >= 0.6 {
            "elated"
        } else if self.current >= 0.2 {
            "content"
        } else if self.current > -0.2 {
            "neutral"
        } else if self.current > -0.6 {
            "unhappy"
        } else {
            "miserable"
        }
    }
}

/// Drift each need toward its natural direction. Run once per tick,
/// before scenario planners that read needs to decide goals.
pub fn tick_needs(
    mut hunger_q: Query<&mut Hunger>,
    mut energy_q: Query<&mut Energy>,
    mut fear_q: Query<&mut Fear>,
) {
    for mut h in &mut hunger_q {
        h.current = (h.current + h.rate).min(1.0);
    }
    for mut e in &mut energy_q {
        e.current = (e.current - e.drain_rate).max(0.0);
    }
    for mut f in &mut fear_q {
        f.current = (f.current - f.decay_rate).max(0.0);
    }
}

/// Anything edible. Attached to a food item, a feeding trough, a
/// kitchen, a coffee station — whatever a creature can `UseEntity`
/// to satisfy hunger. The `eat_on_use` system reduces the user's
/// `Hunger` by `satiation` and bumps `Energy` by `refreshment`.
#[derive(Component, Copy, Clone, Debug)]
pub struct Edible {
    /// Hunger reduction in `[0.0, 1.0]`. 1.0 fully sates; 0.4 a snack.
    pub satiation: f32,
    /// Energy bump in `[0.0, 1.0]`. 0 = food only; 0.4 = coffee.
    pub refreshment: f32,
}

impl Edible {
    pub fn meal() -> Self {
        Self {
            satiation: 1.0,
            refreshment: 0.0,
        }
    }

    pub fn snack(satiation: f32) -> Self {
        Self {
            satiation,
            refreshment: 0.0,
        }
    }

    pub fn coffee() -> Self {
        Self {
            satiation: 0.6,
            refreshment: 0.4,
        }
    }
}

/// When `Event::EntityUsed { user, target }` fires and `target` has
/// an `Edible` component, drop `user`'s `Hunger` and bump their
/// `Energy` accordingly. Run after `execute_tasks` (which emits
/// `EntityUsed`) and after the engine's narration systems.
pub fn eat_on_use(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let used: Vec<(Entity, Entity)> = world
        .resource::<EventLog>()
        .events_at(tick)
        .filter_map(|e| match e {
            Event::EntityUsed { user, target } => Some((*user, *target)),
            _ => None,
        })
        .collect();

    for (user, target) in used {
        let edible = match world.get::<Edible>(target) {
            Some(e) => *e,
            None => continue,
        };
        if let Some(mut h) = world.get_mut::<Hunger>(user) {
            h.feed(edible.satiation);
        }
        if edible.refreshment > 0.0 {
            if let Some(mut e) = world.get_mut::<Energy>(user) {
                e.rest(edible.refreshment);
            }
        }
    }
}

/// When a creature with `Fear` is wounded in combat, spike their
/// fear. If the spike crosses the "terrified" threshold, emit
/// `Event::Terrified` once so the narrator can flag it.
pub fn fear_from_combat(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let victims: Vec<Entity> = world
        .resource::<EventLog>()
        .events_at(tick)
        .filter_map(|e| match e {
            Event::BodyPartWounded { entity, .. } => Some(*entity),
            _ => None,
        })
        .collect();

    let mut newly_terrified: Vec<Entity> = Vec::new();
    for victim in victims {
        if let Some(mut fear) = world.get_mut::<Fear>(victim) {
            let was_terrified = fear.is_terrified();
            fear.frighten(0.35);
            if !was_terrified && fear.is_terrified() {
                newly_terrified.push(victim);
            }
        }
    }
    for v in newly_terrified {
        world
            .resource_mut::<EventLog>()
            .push(tick, Event::Terrified { entity: v });
    }
}

/// Recompute mood from the other needs. Default formula: hunger and
/// fear pull mood down, energy above-half pulls it up, below-half
/// drags it down.
#[allow(clippy::type_complexity)]
pub fn derive_mood(
    mut q: Query<(
        &mut Mood,
        Option<&Hunger>,
        Option<&Fear>,
        Option<&Energy>,
    )>,
) {
    for (mut mood, hunger, fear, energy) in &mut q {
        let mut score = 0.0_f32;
        if let Some(h) = hunger {
            score -= h.current * 0.5;
        }
        if let Some(f) = fear {
            score -= f.current * 0.7;
        }
        if let Some(e) = energy {
            score += (e.current - 0.5) * 0.4;
        }
        mood.current = score.clamp(-1.0, 1.0);
    }
}
