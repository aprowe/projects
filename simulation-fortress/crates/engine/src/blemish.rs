//! Surface details: dents, burns, scratches, finishes, engravings.
//!
//! The `Blemishes` component is a small Vec of `Blemish` structs
//! attached to items and furniture. Each blemish has a `kind`
//! (dent / burn / scratch / water stain / chip / engraving / patina /
//! repair) plus an optional location string ("on the left handle",
//! "near the foot") and a severity in 0..=3.
//!
//! Blemishes nudge `Value`:
//! - Damage blemishes (dent, burn, scratch, chip, water stain) drop
//!   the price by `0.05 * severity` per blemish.
//! - Engravings, patinas, and "signs of age" actually *raise* value
//!   on antique pieces (provenance is provenance). Repairs are
//!   neutral but mentioned in narration.
//!
//! `Finish` is a separate component for surface treatment — matte,
//! glossy, distressed, polished, weathered, lacquered. It's mostly
//! aesthetic but `Distressed` doesn't stack with damage blemishes
//! (the distressing IS the damage).
//!
//! Renderers and narrators read these to emit phrases like
//! "antique walnut dresser, dented at the foot and scratched on the
//! top, with a scorched ring from a candle".

use bevy_ecs::prelude::Component;

#[derive(Component, Clone, Debug, Default)]
pub struct Blemishes(pub Vec<Blemish>);

#[derive(Clone, Debug)]
pub struct Blemish {
    pub kind: BlemishKind,
    /// Free-form location; e.g. "on the left handle", "across the
    /// torso", "behind the lock".
    pub location: String,
    /// 0 = barely noticeable, 3 = unmistakable.
    pub severity: u8,
}

impl Blemish {
    pub fn new(kind: BlemishKind, location: impl Into<String>, severity: u8) -> Self {
        Self {
            kind,
            location: location.into(),
            severity: severity.min(3),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum BlemishKind {
    Dent,
    BurnMark,
    Scratch,
    WaterStain,
    Chip,
    /// Engraved initials / monogram / commemorative inscription.
    Engraving,
    /// Buildup of age/oxidation that connoisseurs prize.
    Patina,
    /// Visible mend — glued ceramic, sutured leather, soldered metal.
    Repair,
    /// Bullet hole or stab mark.
    Hole,
    /// Bloodstain.
    BloodStain,
    /// Chewed by an animal.
    Chewed,
    /// Faded by sun exposure.
    SunFaded,
}

impl BlemishKind {
    pub fn label(self) -> &'static str {
        match self {
            BlemishKind::Dent => "dent",
            BlemishKind::BurnMark => "burn mark",
            BlemishKind::Scratch => "scratch",
            BlemishKind::WaterStain => "water stain",
            BlemishKind::Chip => "chip",
            BlemishKind::Engraving => "engraved inscription",
            BlemishKind::Patina => "patina",
            BlemishKind::Repair => "old repair",
            BlemishKind::Hole => "hole",
            BlemishKind::BloodStain => "bloodstain",
            BlemishKind::Chewed => "chew marks",
            BlemishKind::SunFaded => "sun-faded patch",
        }
    }

    /// Multiplier applied to base `Value`. Damage drops the price;
    /// patinas and engravings can raise it on antiques.
    pub fn value_multiplier(self) -> f32 {
        match self {
            BlemishKind::Dent => 0.95,
            BlemishKind::BurnMark => 0.85,
            BlemishKind::Scratch => 0.95,
            BlemishKind::WaterStain => 0.90,
            BlemishKind::Chip => 0.92,
            BlemishKind::Hole => 0.70,
            BlemishKind::BloodStain => 0.60,
            BlemishKind::Chewed => 0.80,
            BlemishKind::SunFaded => 0.85,
            BlemishKind::Engraving => 1.10,
            BlemishKind::Patina => 1.05,
            BlemishKind::Repair => 0.95,
        }
    }
}

#[derive(Component, Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum Finish {
    #[default]
    Standard,
    Matte,
    Glossy,
    Polished,
    Brushed,
    Lacquered,
    Distressed,
    Weathered,
    HandRubbed,
    Galvanized,
    Painted,
    Whitewashed,
}

impl Finish {
    pub fn label(self) -> &'static str {
        match self {
            Finish::Standard => "",
            Finish::Matte => "matte",
            Finish::Glossy => "glossy",
            Finish::Polished => "polished",
            Finish::Brushed => "brushed",
            Finish::Lacquered => "lacquered",
            Finish::Distressed => "distressed",
            Finish::Weathered => "weathered",
            Finish::HandRubbed => "hand-rubbed",
            Finish::Galvanized => "galvanized",
            Finish::Painted => "painted",
            Finish::Whitewashed => "whitewashed",
        }
    }
}

/// Compose the descriptive postscript for an item or piece of
/// furniture: "[finish], [blemishes joined by ' and ']". Returns
/// the empty string when there's nothing to describe.
pub fn describe_blemishes(finish: Option<Finish>, blemishes: Option<&Blemishes>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(f) = finish {
        let l = f.label();
        if !l.is_empty() {
            parts.push(l.to_string());
        }
    }
    if let Some(b) = blemishes {
        for blem in &b.0 {
            let intensity = match blem.severity {
                0 => "faint",
                1 => "small",
                2 => "noticeable",
                _ => "deep",
            };
            let loc = if blem.location.is_empty() {
                String::new()
            } else {
                format!(" {}", blem.location)
            };
            parts.push(format!("{intensity} {}{loc}", blem.kind.label()));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        parts.join(", ")
    }
}

/// Adjusted value: base × Π blemish multipliers.
pub fn adjusted_value(base: u32, blemishes: Option<&Blemishes>) -> u32 {
    let mut v = base as f32;
    if let Some(b) = blemishes {
        for blem in &b.0 {
            v *= blem.kind.value_multiplier();
        }
    }
    v.round() as u32
}
