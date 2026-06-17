//! Furniture, fixtures, and room dressing.
//!
//! Where `items.rs` covers things you carry and wear, this module
//! covers things that *live* in a room: couches, beds, wardrobes,
//! TVs, windows, sliding glass doors, paintings, rugs.
//!
//! Furniture is just an entity with a `Position` and a `Furniture`
//! marker. Optional state components let it carry richer behavior:
//!
//! - `Powered` — appliances and lights with on/off + a power source
//!   (mains, battery, gas, fire). The `furniture_emit_system` uses
//!   `Powered` to push ambient sound (TV chatter, fridge hum) and
//!   ambient light into the world.
//! - `WindowState` — open / closed / cracked / shattered. Closed
//!   windows block sight (eventually); shattered ones spill broken
//!   glass on the floor.
//! - `Container` — wardrobes, dressers, fridges that hold items.
//!   Looting an item means moving it from the container into the
//!   actor's inventory.
//! - `Painting` — wall art, valuable.
//! - `Rug` — floor covering, primarily aesthetic but contributes
//!   friction overrides.
//! - `LightSource` — emits illumination at a configurable lumen.
//!
//! The `furniture_emit_system` runs once per tick after
//! `update_hearing` to push appliance noise into the perception layer
//! so guards can "hear" the TV from the next room.

use std::collections::VecDeque;

use bevy_ecs::prelude::{Component, Entity, World};

use crate::components::Position;
use crate::log::{Event, EventLog};
use crate::sound::SoundKind;
use crate::time::Clock;

/// Marker: this entity is furniture / fixture, not a creature or item
/// that gets carried. The `kind` is a small enum classifying common
/// archetypes so renderers and planners can treat them generically.
#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
pub struct Furniture(pub FurnitureKind);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FurnitureKind {
    Seating,        // couches, armchairs, dining chairs
    Bed,            // single, double, king
    Storage,        // dresser, wardrobe, bookshelf, nightstand
    Table,          // coffee, dining, side, desk, kitchen island
    Appliance,      // TV, fridge, stove, microwave, washer/dryer
    Plumbing,       // toilet, sink, bathtub, shower
    Lighting,       // lamps, chandeliers, sconces
    WallArt,        // paintings, photos, mirrors
    FloorCovering,  // rugs, carpets-as-entities
    Window,         // glass windows
    Door,           // mostly decorative — real doors live in door.rs
    Decor,          // plants, vases, knickknacks
    Structure,      // columns, banisters, railings, stair entities
    Outdoor,        // grill, patio set, pool, mailbox
}

impl FurnitureKind {
    pub fn label(self) -> &'static str {
        match self {
            FurnitureKind::Seating => "seating",
            FurnitureKind::Bed => "bed",
            FurnitureKind::Storage => "storage",
            FurnitureKind::Table => "table",
            FurnitureKind::Appliance => "appliance",
            FurnitureKind::Plumbing => "plumbing",
            FurnitureKind::Lighting => "lighting",
            FurnitureKind::WallArt => "wall art",
            FurnitureKind::FloorCovering => "floor covering",
            FurnitureKind::Window => "window",
            FurnitureKind::Door => "door",
            FurnitureKind::Decor => "decor",
            FurnitureKind::Structure => "structure",
            FurnitureKind::Outdoor => "outdoor",
        }
    }

    /// Does this furniture type block walking onto its tile? Most
    /// furniture is a soft obstacle (you can step over a coffee table
    /// in the simulation), but beds/couches/walls do block.
    pub fn blocks_tile(self) -> bool {
        matches!(
            self,
            FurnitureKind::Seating
                | FurnitureKind::Bed
                | FurnitureKind::Storage
                | FurnitureKind::Plumbing
                | FurnitureKind::Window
                | FurnitureKind::Structure
                | FurnitureKind::Appliance
        )
    }
}

/// What powers an appliance or light.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerSource {
    Mains,
    Battery,
    Gas,
    Fire,
}

impl PowerSource {
    pub fn label(self) -> &'static str {
        match self {
            PowerSource::Mains => "mains",
            PowerSource::Battery => "battery",
            PowerSource::Gas => "gas",
            PowerSource::Fire => "fire",
        }
    }
}

/// Appliance / light state. `on` is the live state; flipping it via
/// `Toggle` toggles. `ambient_sound` and `ambient_loudness` describe
/// the noise this device makes while on; the emit system pushes a
/// `SoundEmitted` event each tick.
#[derive(Component, Clone, Debug)]
pub struct Powered {
    pub on: bool,
    pub source: PowerSource,
    pub ambient_sound: Option<SoundKind>,
    pub ambient_loudness: f32,
    /// Heat emitted in degrees-celsius equivalent per tick when on.
    /// Stoves +5, fireplaces +10, fridges -1 (consumed by the room).
    pub heat_per_tick: f32,
    pub label: String,
}

impl Powered {
    pub fn off(label: impl Into<String>, source: PowerSource) -> Self {
        Self {
            on: false,
            source,
            ambient_sound: None,
            ambient_loudness: 0.0,
            heat_per_tick: 0.0,
            label: label.into(),
        }
    }

    pub fn on(label: impl Into<String>, source: PowerSource) -> Self {
        Self {
            on: true,
            source,
            ambient_sound: None,
            ambient_loudness: 0.0,
            heat_per_tick: 0.0,
            label: label.into(),
        }
    }

    pub fn with_ambient(mut self, kind: SoundKind, loudness: f32) -> Self {
        self.ambient_sound = Some(kind);
        self.ambient_loudness = loudness;
        self
    }

    pub fn with_heat(mut self, heat_per_tick: f32) -> Self {
        self.heat_per_tick = heat_per_tick;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowState {
    Closed,
    Open,
    Cracked,
    Shattered,
}

impl WindowState {
    pub fn label(self) -> &'static str {
        match self {
            WindowState::Closed => "closed",
            WindowState::Open => "open",
            WindowState::Cracked => "cracked",
            WindowState::Shattered => "shattered",
        }
    }
}

/// A glass or screen pane. Closed/cracked windows block movement but
/// not sight; open or shattered panes are passable.
#[derive(Component, Clone, Debug)]
pub struct Window {
    pub state: WindowState,
    pub label: String,
}

impl Window {
    pub fn closed(label: impl Into<String>) -> Self {
        Self {
            state: WindowState::Closed,
            label: label.into(),
        }
    }
    pub fn open(label: impl Into<String>) -> Self {
        Self {
            state: WindowState::Open,
            label: label.into(),
        }
    }
}

/// A container of items. `lockable` adds a DC for opening. The items
/// inside aren't on the floor — they don't have a `Position` until
/// the container is opened and they're either dropped or transferred.
#[derive(Component, Default, Debug)]
pub struct Container {
    pub items: Vec<Entity>,
    pub locked: bool,
    pub lock_dc: i32,
    pub open: bool,
}

impl Container {
    pub fn unlocked() -> Self {
        Self {
            items: Vec::new(),
            locked: false,
            lock_dc: 0,
            open: false,
        }
    }

    pub fn locked(lock_dc: i32) -> Self {
        Self {
            items: Vec::new(),
            locked: true,
            lock_dc,
            open: false,
        }
    }

    pub fn with_items(mut self, items: impl IntoIterator<Item = Entity>) -> Self {
        self.items.extend(items);
        self
    }
}

/// Wall art. `value_currency` is a soft "how much would this fetch
/// at auction" number — burglars use it to prioritize loot.
#[derive(Component, Clone, Debug)]
pub struct Painting {
    pub artist: String,
    pub title: String,
    pub value_currency: u32,
}

impl Painting {
    pub fn new(artist: impl Into<String>, title: impl Into<String>, value: u32) -> Self {
        Self {
            artist: artist.into(),
            title: title.into(),
            value_currency: value,
        }
    }
}

/// Floor covering — overrides the friction of the underlying voxel.
#[derive(Component, Clone, Debug)]
pub struct Rug {
    pub label: String,
    pub friction: f32,
}

impl Rug {
    pub fn new(label: impl Into<String>, friction: f32) -> Self {
        Self {
            label: label.into(),
            friction,
        }
    }
}

/// A light source. `lumens` 0 = off. Coupled with `Powered` if the
/// light has on/off behavior.
#[derive(Component, Clone, Copy, Debug)]
pub struct LightSource {
    pub lumens: f32,
}

impl LightSource {
    pub fn warm() -> Self {
        Self { lumens: 800.0 }
    }
    pub fn dim() -> Self {
        Self { lumens: 200.0 }
    }
    pub fn bright() -> Self {
        Self { lumens: 1500.0 }
    }
}

/// Marker: a stair tile (so renderers can draw `>`/`<` and
/// pathfinding can later route between floors).
#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stair {
    Up,
    Down,
}

/// Marker: this furniture can be vaulted/climbed over instead of
/// blocking the way. The pathfinder treats the tile as walkable
/// (no wall stamp on spawn) and `physics::footing_check` rolls a
/// DEX check when an actor steps onto a Climbable tile — failure
/// applies `StatusKind::Prone` for a tick.
///
/// `difficulty` is the DC (default 10): low couches and footstools
/// 8, twin beds 10, heavy dressers 14, kitchen counters 12.
#[derive(Component, Copy, Clone, Debug)]
pub struct Climbable {
    pub difficulty: i32,
}

impl Climbable {
    pub fn easy() -> Self { Self { difficulty: 8 } }
    pub fn normal() -> Self { Self { difficulty: 10 } }
    pub fn hard() -> Self { Self { difficulty: 14 } }
}

/// Marker: this furniture / item can be picked up and hauled
/// somewhere else. Mass + STR checks decide whether the lift
/// succeeds and how much it slows the haulier.
///
/// `min_strength` is the rolled-strength threshold (STR + d20)
/// required to lift solo. Heavier pieces need cooperative haulers
/// (`Task::AssistHaul`).
#[derive(Component, Copy, Clone, Debug)]
pub struct Haulable {
    pub min_strength: i32,
    /// Soft cap: items that need at least this many haulers to
    /// move smoothly. 1 = solo, 2 = sofa-class, 3 = piano-class.
    pub haulers_needed: u8,
}

impl Haulable {
    pub fn light() -> Self { Self { min_strength: 8, haulers_needed: 1 } }
    pub fn medium() -> Self { Self { min_strength: 12, haulers_needed: 1 } }
    pub fn heavy() -> Self { Self { min_strength: 16, haulers_needed: 2 } }
    pub fn massive() -> Self { Self { min_strength: 20, haulers_needed: 3 } }
}

/// Live state: this entity is currently being carried by `actor`.
/// The `tick_carrying` system keeps the carried item's `Position`
/// in sync with the carrier's. While this is set, the entity
/// doesn't stamp a wall in the voxel world (it's a "soft prop"
/// being toted around).
#[derive(Component, Copy, Clone, Debug)]
pub struct Carried {
    pub by: Entity,
}

/// Per-tick: every `Carried` entity gets its `Position` snapped to
/// its carrier's. When the carrier dies (no longer has Position),
/// the carried entity is dropped where it stood.
pub fn tick_carrying(world: &mut World) {
    let pairs: Vec<(Entity, Entity)> = {
        let mut q = world.query::<(Entity, &Carried)>();
        q.iter(world).map(|(e, c)| (e, c.by)).collect()
    };
    for (item, carrier) in pairs {
        let new_pos = world.get::<Position>(carrier).map(|p| p.0);
        match new_pos {
            Some(p) => {
                if let Some(mut pos) = world.get_mut::<Position>(item) {
                    pos.0 = p;
                } else {
                    world.entity_mut(item).insert(Position(p));
                }
            }
            None => {
                // Carrier vanished — drop in place by removing Carried.
                world.entity_mut(item).remove::<Carried>();
            }
        }
    }
}

/// Per-tick: every `Powered` entity that is `on` and has an
/// `ambient_sound` pushes a `SoundEmitted` event from its position.
/// This makes "the TV is on in the next room" audible to entities
/// with `Hearing`. Read by `update_hearing` later in the schedule.
pub fn furniture_emit_system(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let mut emissions: VecDeque<Event> = VecDeque::new();
    {
        let mut q = world.query::<(Entity, &Position, &Powered)>();
        for (entity, pos, powered) in q.iter(world) {
            if !powered.on {
                continue;
            }
            if let Some(kind) = powered.ambient_sound.as_ref() {
                emissions.push_back(Event::SoundEmitted {
                    source: Some(entity),
                    position: pos.0,
                    kind: kind.clone(),
                    intensity: powered.ambient_loudness,
                });
            }
        }
    }
    if emissions.is_empty() {
        return;
    }
    let mut log = world.resource_mut::<EventLog>();
    for e in emissions {
        log.push(tick, e);
    }
}
