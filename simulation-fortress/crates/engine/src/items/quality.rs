//! Quality, design style, and currency value.
//!
//! These three axes are orthogonal to material (`ItemMaterial`):
//!
//! - **Quality** is *how well-made* something is. Crude / Cheap /
//!   Standard / Fine / Masterwork / Antique / Heirloom. It scales
//!   the item's combat damage, armor bonus, and currency value.
//! - **Style** is *what aesthetic it belongs to*. Modern, Victorian,
//!   ArtDeco, Rustic, etc. Mostly narrative, but Style + Antique is
//!   what auctioneers pay extra for.
//! - **Value** is the going price in currency, baked from a
//!   template's `base_value` × `Quality.multiplier()` × any style
//!   bonuses. Burglars prioritize loot by value.
//!
//! All three are simple `Component` wrappers attached at spawn time
//! by `spawn_item_template` / `spawn_furniture_template`. Scenarios
//! can also insert them by hand for one-off named treasures.
//!
//! The `narrate_label` helper joins these with `Kind` / `ItemName`
//! to produce phrases like "antique Victorian oil portrait" or
//! "masterwork steel longsword".

use bevy_ecs::prelude::Component;

#[derive(Component, Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub enum Quality {
    /// Hand-made, broken-fixed, makeshift. Combat penalty.
    Crude,
    /// Mass-produced, fragile, generic.
    Cheap,
    #[default]
    Standard,
    /// Premium consumer grade — shows craftsmanship.
    Fine,
    /// Artisan, peak craft. Combat bonus.
    Masterwork,
    /// Old + valuable. Mostly value bonus, no combat boost.
    Antique,
    /// Family heirloom — irreplaceable, top value tier.
    Heirloom,
}

impl Quality {
    pub fn label(self) -> &'static str {
        match self {
            Quality::Crude => "crude",
            Quality::Cheap => "cheap",
            Quality::Standard => "",
            Quality::Fine => "fine",
            Quality::Masterwork => "masterwork",
            Quality::Antique => "antique",
            Quality::Heirloom => "heirloom",
        }
    }

    /// Multiplier applied to currency value when computing the final
    /// `Value` from a template's base.
    pub fn value_multiplier(self) -> f32 {
        match self {
            Quality::Crude => 0.25,
            Quality::Cheap => 0.6,
            Quality::Standard => 1.0,
            Quality::Fine => 2.0,
            Quality::Masterwork => 4.0,
            Quality::Antique => 6.0,
            Quality::Heirloom => 12.0,
        }
    }

    /// Flat add-on to weapon damage / armor AC. Zero for Standard;
    /// negative for Crude/Cheap; positive for Fine and above. Older
    /// pieces (Antique, Heirloom) don't add combat bonus by default
    /// — they'd be too valuable to risk swinging.
    pub fn combat_bonus(self) -> i32 {
        match self {
            Quality::Crude => -1,
            Quality::Cheap => 0,
            Quality::Standard => 0,
            Quality::Fine => 1,
            Quality::Masterwork => 2,
            Quality::Antique => 0,
            Quality::Heirloom => 0,
        }
    }
}

#[derive(Component, Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum Style {
    #[default]
    Standard,
    Modern,
    Contemporary,
    Victorian,
    ArtDeco,
    Rustic,
    Industrial,
    Minimalist,
    Traditional,
    Bohemian,
    Gothic,
    MidCentury,
}

impl Style {
    pub fn label(self) -> &'static str {
        match self {
            Style::Standard => "",
            Style::Modern => "modern",
            Style::Contemporary => "contemporary",
            Style::Victorian => "Victorian",
            Style::ArtDeco => "Art Deco",
            Style::Rustic => "rustic",
            Style::Industrial => "industrial",
            Style::Minimalist => "minimalist",
            Style::Traditional => "traditional",
            Style::Bohemian => "bohemian",
            Style::Gothic => "Gothic",
            Style::MidCentury => "mid-century",
        }
    }

    /// Style + `Quality::Antique`/`Heirloom` synergy: antique
    /// Victorian and Art Deco pieces carry an extra premium.
    pub fn antique_premium(self) -> f32 {
        match self {
            Style::Victorian | Style::ArtDeco | Style::Gothic => 1.5,
            Style::MidCentury | Style::Traditional => 1.25,
            _ => 1.0,
        }
    }
}

/// Currency value of an item or piece of furniture. Burglars
/// prioritize known loot by this value (descending). 0 means
/// "essentially worthless" — common cookware, plain books, etc.
#[derive(Component, Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct Value(pub u32);

/// Override paint color (RGB 0..=255). When present on an entity,
/// renderers prefer this over the underlying `Material.color`. Use
/// for "the wardrobe is painted teal" or "the front door is glossy
/// red". Material color remains the inherent appearance.
#[derive(Component, Copy, Clone, Debug, PartialEq, Eq)]
pub struct Paint(pub [u8; 3]);

impl Paint {
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Paint([r, g, b])
    }
}

impl Value {
    pub fn from_template(base: u32, quality: Quality, style: Style) -> Self {
        let antique_bonus = if matches!(quality, Quality::Antique | Quality::Heirloom) {
            style.antique_premium()
        } else {
            1.0
        };
        let v = (base as f32) * quality.value_multiplier() * antique_bonus;
        Value(v.round() as u32)
    }
}

/// Compose a narrative label like "antique Victorian wardrobe" or
/// "masterwork steel longsword". Returns the bare base name when no
/// modifiers apply.
pub fn narrate_label(base: &str, quality: Option<Quality>, style: Option<Style>) -> String {
    let q = quality.map(|q| q.label()).filter(|s| !s.is_empty());
    let s = style.map(|s| s.label()).filter(|s| !s.is_empty());
    match (q, s) {
        (Some(q), Some(s)) => format!("{q} {s} {base}"),
        (Some(q), None) => format!("{q} {base}"),
        (None, Some(s)) => format!("{s} {base}"),
        (None, None) => base.to_string(),
    }
}
