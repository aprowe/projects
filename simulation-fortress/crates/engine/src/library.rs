//! A discoverable catalog of templates: materials, items, body plans,
//! and roles. Lives on the bevy world as a `Resource`. Scenarios
//! (and REPL / LLM front-ends) can:
//!
//! 1. **Search** the library by substring (`Library::search("knife")`).
//! 2. **List** entries by category.
//! 3. **Spawn** an entity by template name (`spawn_item_template`,
//!    `spawn_role_template`).
//! 4. **Auto-register** materials on demand — `ensure_material` adds
//!    a library material to the active `VoxelWorld` the first time
//!    it's referenced.
//!
//! This is the "what's already in the toolbox" surface that an LLM
//! co-author needs in order to compose new scenarios without
//! reinventing primitives.

use std::collections::HashMap;

use bevy_ecs::prelude::{Entity, Resource, World};

use crate::actions::spawn_creature;
use crate::anatomy::{
    apply_body_plan, dragon_body_plan, humanoid_body_plan, quadruped_body_plan, BodyPlan,
};
use crate::components::Position;
use crate::items::{
    equip_item as engine_equip, give_item, ArmorBonus, BodySlot, DamageDice,
    ElectricalConductivity, Item, ItemMaterial, ItemName, Mass, Temperature, Texture,
    ThermalConductivity, Wearable,
};
use crate::stats::Stats;
use crate::tasks::{Goal, TaskQueue};
use crate::world::{Material, MaterialId, Pos, VoxelWorld};

#[derive(Clone, Debug)]
pub struct ItemTemplate {
    pub mass: f32,
    pub temperature: f32,
    pub thermal_conductivity: Option<f32>,
    pub electrical_conductivity: Option<f32>,
    pub texture: Option<Texture>,
    pub wearable: Option<BodySlot>,
    /// Library material key (e.g. `"steel"`, `"leather"`). When the
    /// template is spawned the engine auto-registers the referenced
    /// material in the active `VoxelWorld` if not already present,
    /// then attaches `ItemMaterial(id)` to the spawned entity.
    pub material: Option<String>,
    /// Damage dice when the item is used as a weapon. None = no
    /// `DamageDice` component (combat falls back to fist 1d3).
    pub damage_dice: Option<DamageDice>,
    /// AC bonus when worn. None = no `ArmorBonus` component.
    pub armor_bonus: Option<i32>,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct RoleTemplate {
    /// Default `Kind` string for the spawned creature. Scenarios can
    /// override on spawn.
    pub kind: String,
    /// Library body-plan key (`"humanoid"`, `"quadruped"`, `"dragon"`).
    pub body_plan: String,
    pub health: i32,
    pub faction: Option<String>,
    /// Library item-template keys to auto-equip on spawn.
    pub equipment: Vec<String>,
    pub description: String,
    /// D&D-style ability scores. Defaults to `Stats::citizen()` when
    /// `None`.
    pub stats: Option<Stats>,
}

#[derive(Resource)]
pub struct Library {
    pub materials: HashMap<String, Material>,
    pub items: HashMap<String, ItemTemplate>,
    pub body_plans: HashMap<String, BodyPlan>,
    pub roles: HashMap<String, RoleTemplate>,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Debug)]
pub enum LibraryHit {
    Material(String),
    Item(String),
    BodyPlan(String),
    Role(String),
}

impl LibraryHit {
    pub fn category(&self) -> &'static str {
        match self {
            LibraryHit::Material(_) => "material",
            LibraryHit::Item(_) => "item",
            LibraryHit::BodyPlan(_) => "body plan",
            LibraryHit::Role(_) => "role",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            LibraryHit::Material(n)
            | LibraryHit::Item(n)
            | LibraryHit::BodyPlan(n)
            | LibraryHit::Role(n) => n.as_str(),
        }
    }
}

impl Default for Library {
    fn default() -> Self {
        let mut lib = Self {
            materials: HashMap::new(),
            items: HashMap::new(),
            body_plans: HashMap::new(),
            roles: HashMap::new(),
        };
        populate_defaults(&mut lib);
        lib
    }
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn empty() -> Self {
        Self {
            materials: HashMap::new(),
            items: HashMap::new(),
            body_plans: HashMap::new(),
            roles: HashMap::new(),
        }
    }

    /// Substring search across every category.
    pub fn search(&self, query: &str) -> Vec<LibraryHit> {
        let q = query.to_lowercase();
        let mut hits: Vec<LibraryHit> = self
            .materials
            .keys()
            .filter(|k| k.to_lowercase().contains(&q))
            .map(|k| LibraryHit::Material(k.clone()))
            .chain(
                self.items
                    .keys()
                    .filter(|k| k.to_lowercase().contains(&q))
                    .map(|k| LibraryHit::Item(k.clone())),
            )
            .chain(
                self.body_plans
                    .keys()
                    .filter(|k| k.to_lowercase().contains(&q))
                    .map(|k| LibraryHit::BodyPlan(k.clone())),
            )
            .chain(
                self.roles
                    .keys()
                    .filter(|k| k.to_lowercase().contains(&q))
                    .map(|k| LibraryHit::Role(k.clone())),
            )
            .collect();
        hits.sort();
        hits
    }

    pub fn category_names(&self) -> Vec<(&'static str, usize)> {
        vec![
            ("materials", self.materials.len()),
            ("items", self.items.len()),
            ("body_plans", self.body_plans.len()),
            ("roles", self.roles.len()),
        ]
    }

    pub fn list_materials(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.materials.keys().map(String::as_str).collect();
        v.sort();
        v
    }

    pub fn list_items(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.items.keys().map(String::as_str).collect();
        v.sort();
        v
    }

    pub fn list_body_plans(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.body_plans.keys().map(String::as_str).collect();
        v.sort();
        v
    }

    pub fn list_roles(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.roles.keys().map(String::as_str).collect();
        v.sort();
        v
    }
}

// ─── ensure / spawn helpers ────────────────────────────────────────────────

/// Look up a material in the active `VoxelWorld`; if not registered,
/// fall through to the library and register it on demand. Returns
/// the resulting `MaterialId`.
pub fn ensure_material(world: &mut World, name: &str) -> Result<MaterialId, String> {
    if let Some(id) = world.resource::<VoxelWorld>().material_id(name) {
        return Ok(id);
    }
    let template = {
        let lib = world.resource::<Library>();
        lib.materials.get(name).cloned()
    };
    match template {
        Some(t) => Ok(world.resource_mut::<VoxelWorld>().register_material(t)),
        None => Err(format!("no material in library: {name}")),
    }
}

/// Options the caller can layer over an item template at spawn time.
#[derive(Default)]
pub struct ItemSpawnOpts {
    pub at: Option<Pos>,
    pub equip_on: Option<Entity>,
    pub give_to: Option<Entity>,
    pub override_label: Option<String>,
}

/// Spawn an item from a library template. The library entry's key is
/// used as the `ItemName` unless `override_label` is supplied. If
/// `equip_on` or `give_to` is set, the item lands on that holder
/// (equip beats give); otherwise `at` decides whether it sits on the
/// floor.
pub fn spawn_item_template(
    world: &mut World,
    template_name: &str,
    opts: ItemSpawnOpts,
) -> Result<Entity, String> {
    let template = {
        let lib = world.resource::<Library>();
        lib.items
            .get(template_name)
            .cloned()
            .ok_or_else(|| format!("no item template: {template_name}"))?
    };

    let material_id = match &template.material {
        Some(m) => Some(ensure_material(world, m)?),
        None => None,
    };

    let label = opts.override_label.unwrap_or_else(|| template_name.to_string());

    let id = {
        let mut e = world.spawn((
            Item,
            ItemName::new(label),
            Mass(template.mass),
            Temperature(template.temperature),
        ));
        if let Some(t) = template.texture {
            e.insert(t);
        }
        if let Some(slot) = template.wearable {
            e.insert(Wearable(slot));
        }
        if let Some(tc) = template.thermal_conductivity {
            e.insert(ThermalConductivity(tc));
        }
        if let Some(ec) = template.electrical_conductivity {
            e.insert(ElectricalConductivity(ec));
        }
        if let Some(mat) = material_id {
            e.insert(ItemMaterial(mat));
        }
        if let Some(dice) = template.damage_dice {
            e.insert(dice);
        }
        if let Some(bonus) = template.armor_bonus {
            e.insert(ArmorBonus(bonus));
        }
        if opts.equip_on.is_none() && opts.give_to.is_none() {
            if let Some(p) = opts.at {
                e.insert(Position(p));
            }
        }
        e.id()
    };

    if let Some(holder) = opts.equip_on {
        engine_equip(world, holder, id);
    } else if let Some(holder) = opts.give_to {
        give_item(world, holder, id);
    }

    Ok(id)
}

#[derive(Default)]
pub struct RoleSpawnOpts {
    pub at: Pos,
    pub kind_label: Option<String>,
    pub faction_override: Option<String>,
    pub health_override: Option<i32>,
}

/// Spawn a creature from a role template: pulls health, faction, body
/// plan, and equipment list out of the library, applies them.
pub fn spawn_role_template(
    world: &mut World,
    role_name: &str,
    opts: RoleSpawnOpts,
) -> Result<Entity, String> {
    let template = {
        let lib = world.resource::<Library>();
        lib.roles
            .get(role_name)
            .cloned()
            .ok_or_else(|| format!("no role template: {role_name}"))?
    };

    let kind = opts.kind_label.unwrap_or_else(|| template.kind.clone());
    let faction = opts
        .faction_override
        .as_deref()
        .or(template.faction.as_deref());
    let health = opts.health_override.unwrap_or(template.health);

    let entity = spawn_creature(world, kind, opts.at, health, faction);

    let plan = {
        let lib = world.resource::<Library>();
        lib.body_plans
            .get(&template.body_plan)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "role '{role_name}' references missing body plan '{}'",
                    template.body_plan
                )
            })?
    };
    apply_body_plan(world, entity, &plan);

    let stats = template.stats.unwrap_or_else(Stats::citizen);
    world
        .entity_mut(entity)
        .insert(TaskQueue::default())
        .insert(Goal::default())
        .insert(stats);

    for item_name in &template.equipment {
        let item = spawn_item_template(world, item_name, ItemSpawnOpts::default())?;
        engine_equip(world, entity, item);
    }

    Ok(entity)
}

// ─── default population ────────────────────────────────────────────────────

fn populate_defaults(lib: &mut Library) {
    populate_materials(lib);
    populate_body_plans(lib);
    populate_items(lib);
    populate_roles(lib);
}

fn populate_materials(lib: &mut Library) {
    let mat = |name: &str,
               solid,
               density,
               flammable,
               friction,
               smell_intensity,
               volatility|
     -> Material {
        Material {
            name: name.into(),
            solid,
            density,
            flammable,
            friction,
            smell_intensity,
            volatility,
        }
    };
    let entries = [
        // structural
        mat("wood",   true, 0.7, true,  0.55, 0.05, 0.0),
        mat("stone",  true, 2.5, false, 0.70, 0.00, 0.0),
        mat("brick",  true, 1.9, false, 0.65, 0.00, 0.0),
        mat("steel",  true, 7.8, false, 0.50, 0.00, 0.0),
        mat("iron",   true, 7.2, false, 0.50, 0.05, 0.0),
        mat("glass",  true, 2.5, false, 0.40, 0.00, 0.0),
        // soft / wearable
        mat("leather", false, 0.9, true, 0.85, 0.10, 0.0),
        mat("rubber",  false, 1.2, true, 0.95, 0.05, 0.0),
        mat("wool",    false, 0.3, true, 0.70, 0.05, 0.0),
        mat("cotton",  false, 0.4, true, 0.70, 0.00, 0.0),
        // ground / vegetation
        mat("grass",  false, 0.1, true,  0.80, 0.05, 0.0),
        mat("soil",   false, 1.5, false, 0.70, 0.05, 0.0),
        mat("sand",   false, 1.6, false, 0.60, 0.00, 0.0),
        mat("dirt",   false, 1.4, false, 0.75, 0.05, 0.0),
        // fluids / coatings — volatility drives smell decay
        mat("water",  false, 1.00, false, 0.40, 0.00, 0.40),
        mat("oil",    false, 0.90, true,  0.05, 0.30, 0.05),
        mat("blood",  false, 1.05, false, 0.25, 0.60, 0.10),
        mat("ice",    true,  0.90, false, 0.10, 0.00, 0.20),
        mat("mud",    false, 1.50, false, 0.50, 0.10, 0.05),
        mat("urine",  false, 1.02, false, 0.30, 0.85, 0.20),
        mat("vomit",  false, 1.00, false, 0.35, 0.70, 0.15),
        // food
        mat("mashed_potato", false, 1.0, true, 0.30, 0.20, 0.10),
        mat("ketchup",       false, 1.1, false, 0.35, 0.30, 0.05),
        mat("oatmeal",       false, 0.9, true, 0.40, 0.15, 0.10),
    ];
    for m in entries {
        lib.materials.insert(m.name.clone(), m);
    }
}

fn populate_body_plans(lib: &mut Library) {
    lib.body_plans.insert("humanoid".into(), humanoid_body_plan());
    lib.body_plans.insert("quadruped".into(), quadruped_body_plan());
    lib.body_plans.insert("dragon".into(), dragon_body_plan());
}

fn populate_items(lib: &mut Library) {
    use BodySlot::*;

    fn weapon(
        mass: f32,
        texture: Texture,
        material: &str,
        dice: DamageDice,
        desc: &str,
    ) -> ItemTemplate {
        ItemTemplate {
            mass,
            temperature: 20.0,
            thermal_conductivity: None,
            electrical_conductivity: None,
            texture: Some(texture),
            wearable: Some(MainHand),
            material: Some(material.into()),
            damage_dice: Some(dice),
            armor_bonus: None,
            description: desc.into(),
        }
    }

    fn clothing(
        mass: f32,
        texture: Texture,
        slot: BodySlot,
        material: &str,
        ac: i32,
        desc: &str,
    ) -> ItemTemplate {
        ItemTemplate {
            mass,
            temperature: 28.0,
            thermal_conductivity: None,
            electrical_conductivity: None,
            texture: Some(texture),
            wearable: Some(slot),
            material: Some(material.into()),
            damage_dice: None,
            armor_bonus: if ac == 0 { None } else { Some(ac) },
            description: desc.into(),
        }
    }

    fn misc(
        mass: f32,
        texture: Option<Texture>,
        material: Option<&str>,
        desc: &str,
    ) -> ItemTemplate {
        ItemTemplate {
            mass,
            temperature: 20.0,
            thermal_conductivity: None,
            electrical_conductivity: None,
            texture,
            wearable: None,
            material: material.map(|s| s.into()),
            damage_dice: None,
            armor_bonus: None,
            description: desc.into(),
        }
    }

    let entries: &[(&str, ItemTemplate)] = &[
        // weapons — damage dice mirror D&D-ish baselines
        ("steel crowbar",
            weapon(2.5, Texture::Polished, "steel",
                DamageDice::with_bonus(1, 8, 1),
                "heavy steel pry bar — bludgeon, can punch through doors")),
        ("kitchen knife",
            weapon(0.3, Texture::Sharp, "steel",
                DamageDice::new(1, 4),
                "sharp narrow blade — clean cuts")),
        ("hunting knife",
            weapon(0.5, Texture::Sharp, "steel",
                DamageDice::with_bonus(1, 6, 1),
                "fixed-blade hunting knife")),
        ("baseball bat",
            weapon(1.0, Texture::Polished, "wood",
                DamageDice::new(1, 6),
                "wooden club, balanced for swinging")),
        ("wooden club",
            weapon(1.5, Texture::Rough, "wood",
                DamageDice::with_bonus(1, 6, 1),
                "rough-cut bludgeon")),
        ("brass candlestick",
            weapon(1.2, Texture::Polished, "iron",
                DamageDice::new(1, 6),
                "heavy ornamental candlestick — surprisingly nasty")),
        ("fire poker",
            weapon(1.5, Texture::Polished, "iron",
                DamageDice::with_bonus(1, 6, 1),
                "wrought-iron poker from the fireplace")),
        ("frying pan",
            weapon(1.2, Texture::Smooth, "iron",
                DamageDice::with_bonus(1, 6, 1),
                "cast-iron skillet, both ends serviceable")),
        ("brick",
            misc(2.5, Some(Texture::Coarse), Some("brick"),
                "throwable masonry block")),

        // food (throwable in food-fight scenario)
        ("plate of mashed potato",
            misc(0.4, Some(Texture::Sticky), Some("mashed_potato"),
                "messy lunch projectile")),
        ("bottle of ketchup",
            misc(0.6, Some(Texture::Slick), Some("ketchup"),
                "sealed sauce, splatters on impact")),

        // clothing / armor — AC bonuses are very mild
        ("wool shirt",        clothing(0.3, Texture::Soft, Torso, "wool",   0,  "warm long-sleeved shirt")),
        ("cotton t-shirt",    clothing(0.2, Texture::Soft, Torso, "cotton", 0,  "light tee")),
        ("leather jacket",    clothing(1.2, Texture::Rough, Torso, "leather", 1,  "heavy outer layer, mild armor")),
        ("hoodie",            clothing(0.5, Texture::Soft, Torso, "cotton", 0,  "hooded sweatshirt")),
        ("kevlar vest",       clothing(2.5, Texture::Rough, Torso, "leather", 3,  "ballistic vest — substantial AC bonus")),
        ("leather boots",     clothing(0.9, Texture::Rough, Feet,  "leather", 0,  "ankle-high boots")),
        ("rubber boots",      clothing(0.9, Texture::Rough, Feet,  "rubber",  0,  "high-grip rubber boots")),
        ("hardhat",           clothing(0.4, Texture::Smooth, Head, "rubber",  1,  "construction safety helmet")),
        ("backpack",          clothing(0.5, Texture::Rough, Back,  "cotton",  0,  "shoulder pack with straps")),
    ];

    for (name, tmpl) in entries {
        lib.items.insert((*name).into(), tmpl.clone());
    }
}

fn populate_roles(lib: &mut Library) {
    let role = |kind: &str,
                body_plan: &str,
                health: i32,
                faction: Option<&str>,
                equipment: &[&str],
                stats: Option<Stats>,
                description: &str|
     -> RoleTemplate {
        RoleTemplate {
            kind: kind.into(),
            body_plan: body_plan.into(),
            health,
            faction: faction.map(|s| s.into()),
            equipment: equipment.iter().map(|s| (*s).to_string()).collect(),
            stats,
            description: description.into(),
        }
    };

    let entries: &[(&str, RoleTemplate)] = &[
        ("civilian",      role("civilian", "humanoid", 60, Some("civilian"), &["cotton t-shirt", "leather boots"], Some(Stats::citizen()), "ordinary unarmed person")),
        ("guard",         role("guard", "humanoid", 120, Some("guard"), &["leather jacket", "leather boots", "wooden club"], Some(Stats::brute()), "armed authority figure")),
        ("thief",         role("thief", "humanoid", 80, Some("thief"), &["hoodie", "rubber boots", "kitchen knife"], Some(Stats::rogue()), "light-footed and dangerous up close")),
        ("soldier",       role("soldier", "humanoid", 100, Some("soldier"), &["leather jacket", "leather boots", "steel crowbar"], Some(Stats::brute()), "trained combatant")),
        ("farmer",        role("farmer", "humanoid", 100, Some("farm"), &["cotton t-shirt", "leather boots"], Some(Stats::citizen()), "field worker")),
        ("zombie",        role("zombie", "humanoid", 60, Some("undead"), &[], Some(Stats::brute()), "shambling, hungry")),
        ("dog",           role("dog", "quadruped", 80, Some("feral"), &[], Some(Stats::rogue()), "fast quadruped with bite attack")),
        ("dragon",        role("dragon", "dragon", 500, Some("dragon"), &[], Some(Stats { str_: 22, dex: 12, con: 20, int: 16, wis: 14, cha: 17 }), "large flying reptile")),
        // family / scenario-specific
        ("father",        role("father", "humanoid", 100, Some("family"), &["cotton t-shirt", "leather boots"], Some(Stats { str_: 14, dex: 11, con: 13, int: 11, wis: 11, cha: 11 }), "family head — moderate STR")),
        ("mother",        role("mother", "humanoid", 80, Some("family"), &["wool shirt", "leather boots"], Some(Stats { str_: 11, dex: 13, con: 12, int: 13, wis: 13, cha: 12 }), "family head — average DEX")),
        ("teenager",      role("teen", "humanoid", 60, Some("family"), &["hoodie", "rubber boots"], Some(Stats { str_: 10, dex: 14, con: 11, int: 12, wis: 9, cha: 12 }), "older child, quick on feet")),
        ("child",         role("child", "humanoid", 30, Some("family"), &["cotton t-shirt", "rubber boots"], Some(Stats::child()), "small kid")),
        ("elder",         role("elder", "humanoid", 50, Some("family"), &["wool shirt", "leather boots"], Some(Stats::elder()), "frail grandparent")),
        ("burglar",       role("burglar", "humanoid", 90, Some("invader"), &["hoodie", "rubber boots", "fire poker"], Some(Stats::rogue()), "lockpicker, light-fingered")),
        ("brute",         role("brute", "humanoid", 130, Some("invader"), &["leather jacket", "leather boots", "steel crowbar"], Some(Stats::brute()), "the muscle")),
        ("market_shopper", role("shopper", "humanoid", 60, Some("crowd"), &["cotton t-shirt", "leather boots"], None, "for the Indian-market thief scenario — wandering background")),
        ("freshman",      role("freshman", "humanoid", 60, Some("freshman"), &["hoodie", "rubber boots"], None, "for the cafeteria food-fight scenario")),
        ("senior",        role("senior", "humanoid", 70, Some("senior"), &["leather jacket", "leather boots"], None, "for the cafeteria food-fight scenario")),
    ];

    for (name, tmpl) in entries {
        lib.roles.insert((*name).into(), tmpl.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_populate_every_category() {
        let lib = Library::default();
        assert!(lib.materials.len() >= 10);
        assert!(lib.items.len() >= 8);
        assert!(lib.body_plans.len() >= 3);
        assert!(lib.roles.len() >= 5);
    }

    #[test]
    fn search_finds_matches_across_categories() {
        let lib = Library::default();
        let hits = lib.search("leather");
        assert!(hits.iter().any(|h| matches!(h, LibraryHit::Material(_))));
        assert!(hits.iter().any(|h| matches!(h, LibraryHit::Item(_))));
    }

    #[test]
    fn search_is_case_insensitive() {
        let lib = Library::default();
        let lower = lib.search("oil");
        let upper = lib.search("OIL");
        assert_eq!(lower, upper);
        assert!(!lower.is_empty());
    }
}
