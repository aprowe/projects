//! Items, physical traits, inventory, and clothing.
//!
//! An item is any entity that carries the `Item` marker plus a name and
//! whichever physical-trait components are relevant. New traits are
//! added by defining a new `Component` and inserting it on the item; no
//! engine changes are required.
//!
//! Inventory and clothing are expressed as components on the *holder*:
//! `Inventory(Vec<Entity>)` lists items being carried, `Wearing` maps
//! body slots to the items currently equipped there. Helpers in this
//! module keep the two views consistent and emit log events.

use std::collections::HashMap;

use bevy_ecs::prelude::{Component, Entity, World};

use crate::components::Position;
use crate::log::{Event, EventLog};
use crate::time::Clock;
use crate::world::{MaterialId, Pos};

// ─── item core ──────────────────────────────────────────────────────────────

/// Marker: this entity is an item, not a creature or environmental
/// feature.
#[derive(Component, Copy, Clone, Debug)]
pub struct Item;

/// Human-readable item name, e.g. "crowbar", "wool shirt".
#[derive(Component, Clone, Debug)]
pub struct ItemName(pub String);

impl ItemName {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

/// What material the item is primarily made of. Optional — items can
/// also have multiple layered materials in future.
#[derive(Component, Copy, Clone, Debug)]
pub struct ItemMaterial(pub MaterialId);

// ─── physical traits ────────────────────────────────────────────────────────

/// Mass in kilograms. Add separate components later if we need to model
/// inertia, momentum, or volume.
#[derive(Component, Copy, Clone, Debug)]
pub struct Mass(pub f32);

/// Current temperature in degrees Celsius. Equilibrates with the
/// surroundings and the carrier over time (system not yet written).
#[derive(Component, Copy, Clone, Debug)]
pub struct Temperature(pub f32);

/// Thermal conductivity, W/(m·K). Wood ≈ 0.13, wool ≈ 0.04, steel ≈ 50,
/// copper ≈ 400.
#[derive(Component, Copy, Clone, Debug)]
pub struct ThermalConductivity(pub f32);

/// Electrical conductivity, S/m. Spans ~30 orders of magnitude across
/// real materials, so use it as a relative number rather than enforcing
/// strict units.
#[derive(Component, Copy, Clone, Debug)]
pub struct ElectricalConductivity(pub f32);

/// Dice rolled when the item lands a hit. Standard D&D notation:
/// `count`d`sides` + `bonus`. Default fist (no DamageDice on item)
/// is 1d4 + 0 in `combat::resolve_attack`.
#[derive(Component, Copy, Clone, Debug)]
pub struct DamageDice {
    pub count: u8,
    pub sides: u8,
    pub bonus: i32,
}

impl DamageDice {
    pub const fn new(count: u8, sides: u8) -> Self {
        Self {
            count,
            sides,
            bonus: 0,
        }
    }
    pub const fn with_bonus(count: u8, sides: u8, bonus: i32) -> Self {
        Self { count, sides, bonus }
    }
}

/// Bonus to the wearer's Armor Class when this item is worn (or the
/// flat AC when used as a shield, etc). Sum across all worn items.
#[derive(Component, Copy, Clone, Debug)]
pub struct ArmorBonus(pub i32);

/// How the item feels to touch.
#[derive(Component, Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Texture {
    Smooth,
    Rough,
    Coarse,
    Soft,
    Sharp,
    Slick,
    Sticky,
    Furry,
    Polished,
    Bumpy,
}

// ─── carrying & wearing ─────────────────────────────────────────────────────

/// Where on a body an item is worn or held.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum BodySlot {
    Head,
    Torso,
    Legs,
    Feet,
    Hands,
    MainHand,
    OffHand,
    Back,
}

impl BodySlot {
    pub fn label(self) -> &'static str {
        match self {
            BodySlot::Head => "head",
            BodySlot::Torso => "torso",
            BodySlot::Legs => "legs",
            BodySlot::Feet => "feet",
            BodySlot::Hands => "hands",
            BodySlot::MainHand => "main hand",
            BodySlot::OffHand => "off hand",
            BodySlot::Back => "back",
        }
    }
}

/// Tag on an item declaring it can be equipped in the given body slot.
#[derive(Component, Copy, Clone, Debug)]
pub struct Wearable(pub BodySlot);

/// Marks a weapon as two-handed; equipping clears the off-hand. The
/// equip helper checks this and refuses if the off-hand is busy.
#[derive(Component, Copy, Clone, Debug)]
pub struct TwoHanded;

/// Tag: this weapon shoots projectiles. `range` is the maximum
/// chebyshev tile distance at which `Task::Shoot` will resolve.
/// Damage is still driven by `DamageDice` on the weapon.
#[derive(Component, Copy, Clone, Debug)]
pub struct RangedWeapon {
    pub range: i32,
    pub ammo_kind: AmmoKind,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum AmmoKind {
    Pistol9mm,
    Shotgun12g,
    Rifle308,
    BowArrow,
    CrossbowBolt,
    ThrownDagger,
}

impl AmmoKind {
    pub fn label(self) -> &'static str {
        match self {
            AmmoKind::Pistol9mm => "9mm",
            AmmoKind::Shotgun12g => "12-gauge shell",
            AmmoKind::Rifle308 => ".308 cartridge",
            AmmoKind::BowArrow => "arrow",
            AmmoKind::CrossbowBolt => "crossbow bolt",
            AmmoKind::ThrownDagger => "dagger",
        }
    }
}

/// One round of ammunition. Despawned when consumed.
#[derive(Component, Copy, Clone, Debug)]
pub struct Ammo(pub AmmoKind);

/// Unstructured carry: items in pockets, satchels, hands without a slot.
#[derive(Component, Default, Debug)]
pub struct Inventory(pub Vec<Entity>);

impl Inventory {
    pub fn contains(&self, item: Entity) -> bool {
        self.0.contains(&item)
    }
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.0.iter().copied()
    }
}

/// Equipped clothing or held items, keyed by body slot.
#[derive(Component, Default, Debug)]
pub struct Wearing(pub HashMap<BodySlot, Entity>);

impl Wearing {
    pub fn get(&self, slot: BodySlot) -> Option<Entity> {
        self.0.get(&slot).copied()
    }
    pub fn iter(&self) -> impl Iterator<Item = (BodySlot, Entity)> + '_ {
        self.0.iter().map(|(&s, &e)| (s, e))
    }
}

// ─── helpers ────────────────────────────────────────────────────────────────

/// Place `item` into `holder`'s `Inventory`, creating the inventory if
/// necessary, and emit an `ItemTaken` event. Also strips the item's
/// `Position` so it is no longer "on the floor" — `drop_item` puts
/// the position back when the holder lets it go.
pub fn give_item(world: &mut World, holder: Entity, item: Entity) {
    {
        let mut entity_mut = world.entity_mut(holder);
        if !entity_mut.contains::<Inventory>() {
            entity_mut.insert(Inventory::default());
        }
        let mut inv = entity_mut.get_mut::<Inventory>().expect("just inserted");
        if !inv.0.contains(&item) {
            inv.0.push(item);
        }
    }
    world.entity_mut(item).remove::<Position>();
    let tick = world.resource::<Clock>().tick;
    world
        .resource_mut::<EventLog>()
        .push(tick, Event::ItemTaken { taker: holder, item });
}

/// Equip `item` in its `Wearable` slot on `wearer`. If something is
/// already in that slot, it is moved into the wearer's `Inventory`.
/// Returns the slot the item ended up in, or `None` if the item has no
/// `Wearable` component.
pub fn equip_item(world: &mut World, wearer: Entity, item: Entity) -> Option<BodySlot> {
    let slot = world.get::<Wearable>(item)?.0;
    let two_handed = world.get::<TwoHanded>(item).is_some();

    let mut entity_mut = world.entity_mut(wearer);
    if !entity_mut.contains::<Wearing>() {
        entity_mut.insert(Wearing::default());
    }
    let (displaced, displaced_offhand) = {
        let mut wearing = entity_mut.get_mut::<Wearing>().expect("just inserted");
        let prev = wearing.0.insert(slot, item);
        let off = if two_handed && slot == BodySlot::MainHand {
            wearing.0.remove(&BodySlot::OffHand)
        } else {
            None
        };
        (prev, off)
    };
    if let Some(prev) = displaced {
        give_item(world, wearer, prev);
    }
    if let Some(off) = displaced_offhand {
        give_item(world, wearer, off);
    }
    let tick = world.resource::<Clock>().tick;
    world.resource_mut::<EventLog>().push(
        tick,
        Event::ItemEquipped {
            wearer,
            item,
            slot,
        },
    );
    Some(slot)
}

/// Remove whatever is in `slot` on `wearer`, moving it into their
/// inventory. Returns the entity that was unequipped, if any.
pub fn unequip_item(world: &mut World, wearer: Entity, slot: BodySlot) -> Option<Entity> {
    let item = {
        let mut wearing = world.get_mut::<Wearing>(wearer)?;
        wearing.0.remove(&slot)?
    };
    give_item(world, wearer, item);
    let tick = world.resource::<Clock>().tick;
    world.resource_mut::<EventLog>().push(
        tick,
        Event::ItemUnequipped {
            wearer,
            item,
            slot,
        },
    );
    Some(item)
}

/// Drop `item` from `holder`'s inventory at `holder`'s position. The
/// item gains a `Position` component so it sits on the floor.
pub fn drop_item(world: &mut World, holder: Entity, item: Entity) {
    let dropped_at = world
        .get::<Position>(holder)
        .map(|p| p.0)
        .unwrap_or_default();

    if let Some(mut inv) = world.get_mut::<Inventory>(holder) {
        inv.0.retain(|&e| e != item);
    }
    if let Some(mut wearing) = world.get_mut::<Wearing>(holder) {
        wearing.0.retain(|_, &mut e| e != item);
    }
    world.entity_mut(item).insert(Position(dropped_at));
    let tick = world.resource::<Clock>().tick;
    world.resource_mut::<EventLog>().push(
        tick,
        Event::ItemDropped {
            dropper: holder,
            item,
            at: dropped_at,
        },
    );
}

/// Convenience: build a vocabulary string describing the held weapons
/// of an entity (for narration). Looks at MainHand and OffHand.
pub fn held_weapons_summary(world: &World, holder: Entity) -> String {
    let Some(wearing) = world.get::<Wearing>(holder) else {
        return String::new();
    };
    let mut parts = Vec::new();
    for slot in [BodySlot::MainHand, BodySlot::OffHand] {
        if let Some(item) = wearing.get(slot) {
            if let Some(name) = world.get::<ItemName>(item) {
                parts.push(format!("{} ({})", name.0, slot.label()));
            }
        }
    }
    parts.join(", ")
}

/// Resolve item position for rendering: items have a `Position`
/// component when they are on the floor, otherwise they are carried by
/// some entity. This returns the position they should be drawn at, or
/// `None` if no holder has a `Position` either.
pub fn item_world_position(world: &World, item: Entity) -> Option<Pos> {
    if let Some(p) = world.get::<Position>(item) {
        return Some(p.0);
    }
    None
}
