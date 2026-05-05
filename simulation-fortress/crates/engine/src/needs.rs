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

use bevy_ecs::prelude::{Component, Query};

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
