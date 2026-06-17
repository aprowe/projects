//! Generic creature components used by every scenario.
//!
//! Scenarios can also define their own components for marker traits
//! (`Intruder`, `Conscript`) or scenario-specific data; these here are
//! the lingua franca that engine systems and renderers rely on.

use std::collections::HashMap;

use bevy_ecs::prelude::Component;

use crate::world::Pos;

/// Human-readable kind tag, e.g. `"resident_0"`, `"intruder"`, `"deer"`.
#[derive(Component, Clone, Debug)]
pub struct Kind(pub String);

impl Kind {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

/// Voxel-aligned position. Sub-voxel motion can be added later.
#[derive(Component, Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Position(pub Pos);

/// Hit points. An entity is considered alive while `current > 0`.
#[derive(Component, Copy, Clone, Debug)]
pub struct Health {
    pub current: i32,
    pub max: i32,
}

impl Health {
    pub fn new(max: i32) -> Self {
        Self { current: max, max }
    }

    pub fn is_alive(&self) -> bool {
        self.current > 0
    }
}

/// Which side the entity belongs to. Free-form string today; will grow
/// into a registry once §12 of SYSTEMS.md is implemented.
#[derive(Component, Clone, Debug)]
pub struct Faction(pub String);

impl Faction {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

/// Open-ended per-scenario fields. An escape hatch until repeated keys
/// here graduate to first-class components.
#[derive(Component, Clone, Default, Debug)]
pub struct ExtraData(pub HashMap<String, String>);
