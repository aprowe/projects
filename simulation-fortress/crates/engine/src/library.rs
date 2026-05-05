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
use crate::components::{Kind, Position};
use crate::furniture::{
    Container, Furniture, FurnitureKind, LightSource, Painting, PowerSource, Powered, Rug,
    Window,
};
use crate::items::{
    equip_item as engine_equip, give_item, ArmorBonus, BodySlot, DamageDice,
    ElectricalConductivity, Item, ItemMaterial, ItemName, Mass, Temperature, Texture,
    ThermalConductivity, Wearable,
};
use crate::quality::{Quality, Style, Value};
use crate::sound::SoundKind;
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
    /// Craftsmanship tier. Scales combat bonus + value multiplier.
    /// Defaults to `Quality::Standard` if `None`.
    pub quality: Option<Quality>,
    /// Aesthetic style. Mostly narrative, with an antique premium.
    pub style: Option<Style>,
    /// Currency value before quality/style multipliers. Final
    /// `Value` is computed at spawn via `Value::from_template`.
    /// 0 = worthless (everyday objects).
    pub base_value: u32,
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

/// A piece of furniture in the catalog. Each template knows its
/// kind, its glyph (for ASCII renderers), the material it's made of,
/// its footprint in voxel tiles (a king bed is 2x3, a couch is 3x1,
/// a pool is 4x4), and optional state specs (powered/container/
/// window/painting/rug/light) that the spawn helper stamps onto the
/// anchor entity.
#[derive(Clone, Debug)]
pub struct FurnitureTemplate {
    pub kind: FurnitureKind,
    pub glyph: char,
    pub material: Option<String>,
    /// Footprint, in tiles, as `(width_x, depth_y)`. `(1, 1)` is the
    /// default — a single-tile entity. The spawn helper places one
    /// entity per tile (so the renderer sees the glyph repeated)
    /// and stamps state components onto the anchor (top-left) tile.
    pub size: (u8, u8),
    pub powered: Option<PoweredSpec>,
    pub container: Option<ContainerSpec>,
    pub window: Option<WindowSpec>,
    pub painting: Option<PaintingSpec>,
    pub rug: Option<RugSpec>,
    pub light_lumens: Option<f32>,
    /// Craftsmanship tier. Defaults to `Quality::Standard`.
    pub quality: Option<Quality>,
    /// Aesthetic style. `Standard` = no modifier in narration.
    pub style: Option<Style>,
    /// Currency value before quality/style multipliers (0 = trivial).
    pub base_value: u32,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct PoweredSpec {
    pub on: bool,
    pub source: PowerSource,
    pub ambient: Option<(SoundKind, f32)>,
    pub heat_per_tick: f32,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct ContainerSpec {
    pub locked: bool,
    pub lock_dc: i32,
}

#[derive(Clone, Debug)]
pub struct WindowSpec {
    pub closed: bool,
}

#[derive(Clone, Debug)]
pub struct PaintingSpec {
    pub artist: String,
    pub title: String,
    pub value: u32,
}

#[derive(Clone, Debug)]
pub struct RugSpec {
    pub friction: f32,
}

#[derive(Resource)]
pub struct Library {
    pub materials: HashMap<String, Material>,
    pub items: HashMap<String, ItemTemplate>,
    pub body_plans: HashMap<String, BodyPlan>,
    pub roles: HashMap<String, RoleTemplate>,
    pub furniture: HashMap<String, FurnitureTemplate>,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Debug)]
pub enum LibraryHit {
    Material(String),
    Item(String),
    BodyPlan(String),
    Role(String),
    Furniture(String),
}

impl LibraryHit {
    pub fn category(&self) -> &'static str {
        match self {
            LibraryHit::Material(_) => "material",
            LibraryHit::Item(_) => "item",
            LibraryHit::BodyPlan(_) => "body plan",
            LibraryHit::Role(_) => "role",
            LibraryHit::Furniture(_) => "furniture",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            LibraryHit::Material(n)
            | LibraryHit::Item(n)
            | LibraryHit::BodyPlan(n)
            | LibraryHit::Role(n)
            | LibraryHit::Furniture(n) => n.as_str(),
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
            furniture: HashMap::new(),
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
            furniture: HashMap::new(),
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
            .chain(
                self.furniture
                    .keys()
                    .filter(|k| k.to_lowercase().contains(&q))
                    .map(|k| LibraryHit::Furniture(k.clone())),
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
            ("furniture", self.furniture.len()),
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

    pub fn list_furniture(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.furniture.keys().map(String::as_str).collect();
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
        let quality = template.quality.unwrap_or(Quality::Standard);
        let style = template.style.unwrap_or(Style::Standard);
        if quality != Quality::Standard {
            e.insert(quality);
        }
        if style != Style::Standard {
            e.insert(style);
        }
        if template.base_value > 0 {
            e.insert(Value::from_template(template.base_value, quality, style));
        }
        let combat_bonus = quality.combat_bonus();
        if let Some(mut dice) = template.damage_dice {
            // Quality flat-bonuses the dice (Crude -1, Masterwork +2, etc.)
            dice.bonus = (dice.bonus as i32 + combat_bonus).max(-3) as i32;
            e.insert(dice);
        }
        if let Some(bonus) = template.armor_bonus {
            e.insert(ArmorBonus((bonus + combat_bonus).max(0)));
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

/// Options the caller can layer over a furniture template at spawn.
#[derive(Default)]
pub struct FurnitureSpawnOpts {
    pub at: Pos,
    pub kind_label: Option<String>,
}

/// Spawn a furniture entity from a template. Multi-tile templates
/// (a 3x1 sofa, a 2x3 king bed, a 5x3 pool) stamp one entity per
/// tile so the renderer paints the glyph across the full footprint.
/// State components (`Powered`, `Container`, `Window`, `Painting`,
/// `Rug`, `LightSource`) live ONLY on the anchor entity (top-left
/// of the footprint) — that's the "interaction tile" planners walk
/// up to. The returned `Entity` is the anchor.
///
/// Blocking furniture (`FurnitureKind::blocks_tile()`) also stamps
/// a wall voxel of its `material` at every footprint tile, so the
/// pathfinder treats the whole sofa as impassable instead of letting
/// actors walk through it.
pub fn spawn_furniture_template(
    world: &mut World,
    template_name: &str,
    opts: FurnitureSpawnOpts,
) -> Result<Entity, String> {
    let template = {
        let lib = world.resource::<Library>();
        lib.furniture
            .get(template_name)
            .cloned()
            .ok_or_else(|| format!("no furniture template: {template_name}"))?
    };

    let material_id = match &template.material {
        Some(m) => Some(ensure_material(world, m)?),
        None => None,
    };

    let label = opts.kind_label.unwrap_or_else(|| template_name.to_string());
    let (w, d) = template.size;
    let w = w.max(1) as i32;
    let d = d.max(1) as i32;

    // Spawn the anchor first.
    let anchor_pos = opts.at;
    let id = world
        .spawn((
            Position(anchor_pos),
            Kind(label.clone()),
            Furniture(template.kind),
        ))
        .id();
    if let Some(mat) = material_id {
        world.entity_mut(id).insert(ItemMaterial(mat));
    }

    // Spawn one extra entity per footprint tile beyond the anchor.
    // These get a Position + Kind + Furniture so the renderer paints
    // them, but no state components — the anchor owns those.
    for dx in 0..w {
        for dy in 0..d {
            if dx == 0 && dy == 0 {
                continue;
            }
            let tile = crate::world::Pos::new(anchor_pos.x + dx, anchor_pos.y + dy, anchor_pos.z);
            let tile_id = world
                .spawn((
                    Position(tile),
                    Kind(label.clone()),
                    Furniture(template.kind),
                ))
                .id();
            if let Some(mat) = material_id {
                world.entity_mut(tile_id).insert(ItemMaterial(mat));
            }
        }
    }

    // Optionally stamp wall voxels for the whole footprint so paths
    // route around the sofa instead of over it. Outdoor + decor +
    // floor-coverings + lighting + wall art don't block. Open
    // windows / sliding doors don't block either. Staircases get
    // special treatment — RampUp at z and Empty above (so an actor
    // can climb).
    let window_open = matches!(&template.window, Some(w) if !w.closed);
    let is_staircase = matches!(template_name, "staircase up" | "staircase down");
    if is_staircase {
        if let Some(mat) = material_id {
            let mut vw = world.resource_mut::<VoxelWorld>();
            let ramp = crate::world::Voxel::ramp(mat);
            for dx in 0..w {
                for dy in 0..d {
                    let tile = crate::world::Pos::new(
                        anchor_pos.x + dx,
                        anchor_pos.y + dy,
                        anchor_pos.z,
                    );
                    vw.set_voxel(tile, ramp);
                    // Air above so the actor can stand on top of the
                    // ramp at z+1 — supported by the RampUp.
                    let above = crate::world::Pos::new(
                        anchor_pos.x + dx,
                        anchor_pos.y + dy,
                        anchor_pos.z + 1,
                    );
                    vw.set_voxel(above, crate::world::Voxel::empty());
                }
            }
        }
    } else if template.kind.blocks_tile() && !window_open {
        if let Some(mat) = material_id {
            let mut vw = world.resource_mut::<VoxelWorld>();
            let voxel = crate::world::Voxel::wall(mat);
            for dx in 0..w {
                for dy in 0..d {
                    let tile = crate::world::Pos::new(
                        anchor_pos.x + dx,
                        anchor_pos.y + dy,
                        anchor_pos.z,
                    );
                    vw.set_voxel(tile, voxel);
                }
            }
        }
    }

    if let Some(p) = template.powered {
        let mut comp = if p.on {
            Powered::on(p.label, p.source)
        } else {
            Powered::off(p.label, p.source)
        };
        if let Some((kind, loud)) = p.ambient {
            comp = comp.with_ambient(kind, loud);
        }
        if p.heat_per_tick != 0.0 {
            comp = comp.with_heat(p.heat_per_tick);
        }
        world.entity_mut(id).insert(comp);
    }
    if let Some(c) = template.container {
        let comp = if c.locked {
            Container::locked(c.lock_dc)
        } else {
            Container::unlocked()
        };
        world.entity_mut(id).insert(comp);
    }
    if let Some(w) = template.window {
        let win = if w.closed {
            Window::closed(template_name)
        } else {
            Window::open(template_name)
        };
        world.entity_mut(id).insert(win);
    }
    if let Some(p) = template.painting {
        world
            .entity_mut(id)
            .insert(Painting::new(p.artist, p.title, p.value));
    }
    if let Some(r) = template.rug {
        world.entity_mut(id).insert(Rug::new(template_name, r.friction));
    }
    if let Some(lumens) = template.light_lumens {
        world.entity_mut(id).insert(LightSource { lumens });
    }

    let quality = template.quality.unwrap_or(Quality::Standard);
    let style = template.style.unwrap_or(Style::Standard);
    if quality != Quality::Standard {
        world.entity_mut(id).insert(quality);
    }
    if style != Style::Standard {
        world.entity_mut(id).insert(style);
    }
    if template.base_value > 0 {
        world
            .entity_mut(id)
            .insert(Value::from_template(template.base_value, quality, style));
    }
    Ok(id)
}

// ─── default population ────────────────────────────────────────────────────

fn populate_defaults(lib: &mut Library) {
    populate_materials(lib);
    populate_body_plans(lib);
    populate_items(lib);
    populate_roles(lib);
    populate_furniture(lib);
}

fn populate_materials(lib: &mut Library) {
    let mat = |name: &str,
               solid,
               density,
               flammable,
               friction,
               smell_intensity,
               volatility,
               color: [u8; 3]|
     -> Material {
        Material {
            name: name.into(),
            solid,
            density,
            flammable,
            friction,
            smell_intensity,
            volatility,
            color,
        }
    };
    let entries = [
        // structural — heavy
        mat("wood",     true, 0.7, true,  0.55, 0.05, 0.0,  [139,  90,  43]),
        mat("oak",      true, 0.8, true,  0.60, 0.05, 0.0,  [115,  74,  18]),
        mat("hardwood", true, 0.8, true,  0.60, 0.04, 0.0,  [120,  72,  36]),
        mat("plywood",  true, 0.5, true,  0.55, 0.02, 0.0,  [200, 160, 100]),
        mat("pine",     true, 0.5, true,  0.55, 0.06, 0.0,  [218, 170, 110]),
        mat("stone",    true, 2.5, false, 0.70, 0.00, 0.0,  [120, 120, 120]),
        mat("granite",  true, 2.7, false, 0.55, 0.00, 0.0,  [136, 124, 116]),
        mat("marble",   true, 2.7, false, 0.45, 0.00, 0.0,  [232, 228, 222]),
        mat("brick",    true, 1.9, false, 0.65, 0.00, 0.0,  [156,  74,  60]),
        mat("concrete", true, 2.4, false, 0.70, 0.00, 0.0,  [165, 165, 158]),
        mat("drywall",  true, 0.7, true,  0.60, 0.00, 0.0,  [228, 222, 210]),
        mat("plaster",  true, 0.9, false, 0.60, 0.00, 0.0,  [232, 222, 200]),
        mat("steel",    true, 7.8, false, 0.50, 0.00, 0.0,  [160, 160, 170]),
        mat("iron",     true, 7.2, false, 0.50, 0.05, 0.0,  [ 96,  96, 100]),
        mat("aluminum", true, 2.7, false, 0.55, 0.00, 0.0,  [200, 200, 205]),
        mat("glass",    true, 2.5, false, 0.40, 0.00, 0.0,  [180, 220, 230]),
        // surfaces — floor coverings
        mat("carpet",     false, 0.4, true, 0.85, 0.05, 0.0,  [160,  60,  60]),
        mat("tile",       true,  2.3, false, 0.45, 0.00, 0.0,  [200, 200, 195]),
        mat("linoleum",   false, 1.0, true,  0.55, 0.02, 0.0,  [200, 180, 130]),
        mat("hardwood_floor", true, 0.8, true, 0.55, 0.04, 0.0, [150,  90,  40]),
        mat("wallpaper",  false, 0.2, true,  0.65, 0.00, 0.0,  [220, 200, 175]),
        mat("paint",      false, 0.1, true,  0.60, 0.05, 0.0,  [240, 240, 240]),
        // soft / wearable
        mat("leather", false, 0.9, true, 0.85, 0.10, 0.0,  [110,  60,  30]),
        mat("rubber",  false, 1.2, true, 0.95, 0.05, 0.0,  [ 30,  30,  30]),
        mat("wool",    false, 0.3, true, 0.70, 0.05, 0.0,  [180, 170, 150]),
        mat("cotton",  false, 0.4, true, 0.70, 0.00, 0.0,  [240, 240, 230]),
        mat("silk",    false, 0.3, true, 0.50, 0.05, 0.0,  [240, 220, 200]),
        mat("velvet",  false, 0.4, true, 0.80, 0.05, 0.0,  [120,  20,  60]),
        mat("denim",   false, 0.5, true, 0.70, 0.05, 0.0,  [ 60, 100, 160]),
        // foam / fillings
        mat("foam",      false, 0.1, true, 0.80, 0.00, 0.0,  [240, 230, 200]),
        mat("polyester", false, 0.4, true, 0.75, 0.00, 0.0,  [200, 200, 200]),
        // ground / vegetation
        mat("grass",  false, 0.1, true,  0.80, 0.05, 0.0,  [ 80, 140,  60]),
        mat("soil",   false, 1.5, false, 0.70, 0.05, 0.0,  [ 90,  60,  40]),
        mat("sand",   false, 1.6, false, 0.60, 0.00, 0.0,  [220, 200, 150]),
        mat("dirt",   false, 1.4, false, 0.75, 0.05, 0.0,  [110,  78,  56]),
        mat("gravel", false, 1.7, false, 0.75, 0.00, 0.0,  [140, 130, 120]),
        mat("asphalt", true, 2.3, false, 0.65, 0.05, 0.0,  [ 50,  50,  55]),
        // fluids / coatings — volatility drives smell decay
        mat("water",  false, 1.00, false, 0.40, 0.00, 0.40,  [ 80, 130, 200]),
        mat("oil",    false, 0.90, true,  0.05, 0.30, 0.05,  [ 40,  35,  25]),
        mat("blood",  false, 1.05, false, 0.25, 0.60, 0.10,  [160,  20,  20]),
        mat("ice",    true,  0.90, false, 0.10, 0.00, 0.20,  [200, 230, 240]),
        mat("mud",    false, 1.50, false, 0.50, 0.10, 0.05,  [ 90,  60,  30]),
        mat("urine",  false, 1.02, false, 0.30, 0.85, 0.20,  [220, 200,  80]),
        mat("vomit",  false, 1.00, false, 0.35, 0.70, 0.15,  [180, 150,  90]),
        mat("wine",   false, 0.99, true,  0.30, 0.40, 0.20,  [120,  20,  30]),
        // food
        mat("mashed_potato", false, 1.0, true, 0.30, 0.20, 0.10,  [240, 220, 170]),
        mat("ketchup",       false, 1.1, false, 0.35, 0.30, 0.05,  [180,  30,  20]),
        mat("oatmeal",       false, 0.9, true, 0.40, 0.15, 0.10,  [200, 180, 140]),
        mat("flour",         false, 0.5, true, 0.55, 0.10, 0.05,  [240, 235, 220]),
        mat("sugar",         false, 0.8, true, 0.45, 0.05, 0.05,  [250, 250, 250]),
        mat("coffee",        false, 0.8, true, 0.40, 0.50, 0.10,  [ 80,  50,  30]),
        // canvas — for paintings
        mat("canvas",  false, 0.4, true, 0.65, 0.00, 0.0,  [220, 200, 170]),
        mat("paper",   false, 0.3, true, 0.65, 0.00, 0.0,  [240, 235, 220]),
        // ceramics / plastics / misc
        mat("porcelain", true,  2.4, false, 0.40, 0.00, 0.0,  [240, 240, 235]),
        mat("ceramic",   true,  2.0, false, 0.45, 0.00, 0.0,  [220, 200, 175]),
        mat("plastic",   false, 0.9, true,  0.55, 0.00, 0.0,  [200, 200, 200]),
        mat("wax",       false, 0.9, true,  0.50, 0.10, 0.05,  [240, 230, 210]),
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
            quality: None,
            style: None,
            base_value: 30,
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
            quality: None,
            style: None,
            base_value: 20,
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
            quality: None,
            style: None,
            base_value: 0,
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
            ItemTemplate {
                quality: Some(Quality::Antique),
                style: Some(Style::Victorian),
                base_value: 250,
                ..weapon(1.2, Texture::Polished, "iron",
                    DamageDice::new(1, 6),
                    "heavy ornamental candlestick — surprisingly nasty")
            }),
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
        ("kevlar vest",
            ItemTemplate {
                quality: Some(Quality::Fine),
                style: Some(Style::Modern),
                base_value: 400,
                ..clothing(2.5, Texture::Rough, Torso, "leather", 3,
                    "ballistic vest — substantial AC bonus")
            }),
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

fn populate_furniture(lib: &mut Library) {
    use FurnitureKind::*;

    // Helper closures that build a template with sensible defaults.
    fn base(kind: FurnitureKind, glyph: char, material: &str, desc: &str) -> FurnitureTemplate {
        FurnitureTemplate {
            kind,
            glyph,
            material: Some(material.into()),
            size: (1, 1),
            powered: None,
            container: None,
            window: None,
            painting: None,
            rug: None,
            light_lumens: None,
            quality: None,
            style: None,
            base_value: 0,
            description: desc.into(),
        }
    }
    fn sized(
        kind: FurnitureKind,
        glyph: char,
        material: &str,
        size: (u8, u8),
        desc: &str,
    ) -> FurnitureTemplate {
        FurnitureTemplate {
            size,
            ..base(kind, glyph, material, desc)
        }
    }
    fn pwr(label: &str, src: PowerSource, on: bool, ambient: Option<(SoundKind, f32)>, heat: f32)
        -> PoweredSpec
    {
        PoweredSpec {
            on,
            source: src,
            ambient,
            heat_per_tick: heat,
            label: label.into(),
        }
    }

    let entries: &[(&str, FurnitureTemplate)] = &[
        // ─── seating ──────────────────────────────────────────────
        ("sofa",         sized(Seating, 's', "velvet",  (3, 1), "long upholstered couch — 3 tiles wide")),
        ("loveseat",     sized(Seating, 's', "velvet",  (2, 1), "small couch for two")),
        ("armchair",     sized(Seating, 'a', "leather", (1, 1), "single-seat lounge chair")),
        ("dining chair", sized(Seating, 'h', "oak",     (1, 1), "wooden chair at the dining table")),
        ("bench",        sized(Seating, 'b', "oak",     (3, 1), "long bench in the foyer")),
        ("ottoman",      sized(Seating, 'o', "velvet",  (1, 1), "footrest")),
        ("recliner",     sized(Seating, 'r', "leather", (1, 1), "reclining lounge chair")),

        // ─── beds ─────────────────────────────────────────────────
        ("king bed",   sized(Bed, 'B', "oak",  (2, 3), "king-size four-poster")),
        ("queen bed",  sized(Bed, 'B', "oak",  (2, 3), "queen-size bed")),
        ("twin bed",   sized(Bed, 'b', "pine", (1, 2), "single twin")),
        ("crib",       sized(Bed, 'c', "pine", (1, 2), "infant crib")),

        // ─── storage ───────────────────────────────────────────────
        ("wardrobe",
            FurnitureTemplate {
                size: (2, 1),
                container: Some(ContainerSpec { locked: false, lock_dc: 0 }),
                ..base(Storage, 'W', "oak", "tall closet wardrobe")
            }),
        ("dresser",
            FurnitureTemplate {
                size: (2, 1),
                container: Some(ContainerSpec { locked: false, lock_dc: 0 }),
                ..base(Storage, 'D', "oak", "five-drawer dresser")
            }),
        ("locked dresser",
            FurnitureTemplate {
                size: (2, 1),
                container: Some(ContainerSpec { locked: true, lock_dc: 14 }),
                ..base(Storage, 'D', "oak", "dresser with a locked top drawer")
            }),
        ("nightstand",
            FurnitureTemplate {
                container: Some(ContainerSpec { locked: false, lock_dc: 0 }),
                ..base(Storage, 'n', "oak", "bedside nightstand with one drawer")
            }),
        ("bookshelf",
            FurnitureTemplate {
                size: (1, 2),
                container: Some(ContainerSpec { locked: false, lock_dc: 0 }),
                ..base(Storage, 'L', "oak", "tall bookshelf")
            }),
        ("china cabinet",
            FurnitureTemplate {
                size: (2, 1),
                container: Some(ContainerSpec { locked: false, lock_dc: 0 }),
                ..base(Storage, 'C', "oak", "glass-front china cabinet")
            }),
        ("safe",
            FurnitureTemplate {
                container: Some(ContainerSpec { locked: true, lock_dc: 22 }),
                quality: Some(Quality::Fine),
                style: Some(Style::Industrial),
                base_value: 800,
                ..base(Storage, 'S', "steel", "wall safe")
            }),
        ("filing cabinet",
            FurnitureTemplate {
                container: Some(ContainerSpec { locked: false, lock_dc: 0 }),
                ..base(Storage, 'f', "steel", "metal filing cabinet")
            }),

        // ─── tables ────────────────────────────────────────────────
        ("dining table",  sized(Table, 't', "oak",     (2, 4), "long dining table")),
        ("coffee table",  sized(Table, 'c', "oak",     (2, 1), "low living-room table")),
        ("desk",          sized(Table, 'd', "oak",     (2, 1), "writing desk")),
        ("kitchen island", sized(Table, 'i', "granite", (3, 1), "kitchen island with stone top")),
        ("side table",    sized(Table, 's', "oak",     (1, 1), "small side table")),

        // ─── appliances ────────────────────────────────────────────
        ("tv set",
            FurnitureTemplate {
                powered: Some(pwr("flatscreen TV", PowerSource::Mains, true,
                    Some((SoundKind::Other("tv chatter".into()), 0.45)), 0.0)),
                ..base(Appliance, 'T', "plastic", "wall-mounted flatscreen, currently on")
            }),
        ("tv off",
            FurnitureTemplate {
                powered: Some(pwr("flatscreen TV", PowerSource::Mains, false, None, 0.0)),
                ..base(Appliance, 'T', "plastic", "wall-mounted flatscreen, off")
            }),
        ("stereo",
            FurnitureTemplate {
                powered: Some(pwr("stereo", PowerSource::Mains, false,
                    Some((SoundKind::Other("music".into()), 0.40)), 0.0)),
                ..base(Appliance, 'r', "plastic", "vintage stereo with vinyl player")
            }),
        ("refrigerator",
            FurnitureTemplate {
                powered: Some(pwr("refrigerator", PowerSource::Mains, true,
                    Some((SoundKind::Other("hum".into()), 0.10)), -1.0)),
                container: Some(ContainerSpec { locked: false, lock_dc: 0 }),
                ..base(Appliance, 'F', "steel", "two-door fridge, humming")
            }),
        ("stove",
            FurnitureTemplate {
                powered: Some(pwr("gas stove", PowerSource::Gas, false, None, 5.0)),
                ..base(Appliance, 'O', "steel", "gas range stove")
            }),
        ("stove on",
            FurnitureTemplate {
                powered: Some(pwr("gas stove", PowerSource::Gas, true,
                    Some((SoundKind::Other("burner hiss".into()), 0.05)), 5.0)),
                ..base(Appliance, 'O', "steel", "stove with two burners on")
            }),
        ("microwave",
            FurnitureTemplate {
                powered: Some(pwr("microwave", PowerSource::Mains, false, None, 0.0)),
                ..base(Appliance, 'm', "steel", "countertop microwave")
            }),
        ("dishwasher",
            FurnitureTemplate {
                powered: Some(pwr("dishwasher", PowerSource::Mains, false, None, 0.0)),
                ..base(Appliance, 'w', "steel", "built-in dishwasher")
            }),
        ("washer",
            FurnitureTemplate {
                powered: Some(pwr("washer", PowerSource::Mains, false, None, 0.0)),
                ..base(Appliance, 'w', "steel", "front-loading washing machine")
            }),
        ("dryer",
            FurnitureTemplate {
                powered: Some(pwr("dryer", PowerSource::Mains, false, None, 0.0)),
                ..base(Appliance, 'y', "steel", "tumble dryer")
            }),
        ("fireplace",
            FurnitureTemplate {
                powered: Some(pwr("fireplace", PowerSource::Fire, true,
                    Some((SoundKind::Other("crackle".into()), 0.20)), 10.0)),
                light_lumens: Some(800.0),
                ..base(Appliance, '*', "brick", "stone fireplace, lit")
            }),
        ("fireplace cold",
            FurnitureTemplate {
                powered: Some(pwr("fireplace", PowerSource::Fire, false, None, 0.0)),
                ..base(Appliance, '*', "brick", "stone fireplace, ashes only")
            }),
        ("ceiling fan",
            FurnitureTemplate {
                powered: Some(pwr("ceiling fan", PowerSource::Mains, true,
                    Some((SoundKind::Other("whir".into()), 0.08)), 0.0)),
                ..base(Appliance, 'F', "aluminum", "three-blade ceiling fan")
            }),

        // ─── plumbing ──────────────────────────────────────────────
        ("toilet",        sized(Plumbing, 'u', "porcelain", (1, 1), "porcelain toilet")),
        ("bathroom sink", sized(Plumbing, 'k', "porcelain", (1, 1), "pedestal sink")),
        ("kitchen sink",  sized(Plumbing, 'K', "steel",     (2, 1), "stainless double-basin sink")),
        ("bathtub",       sized(Plumbing, 'U', "porcelain", (1, 2), "claw-foot tub")),
        ("shower",        sized(Plumbing, 'H', "tile",      (2, 2), "glass-walled shower")),

        // ─── lighting ──────────────────────────────────────────────
        ("table lamp",
            FurnitureTemplate {
                powered: Some(pwr("table lamp", PowerSource::Mains, true, None, 0.0)),
                light_lumens: Some(600.0),
                ..base(Lighting, 'l', "ceramic", "table lamp with linen shade")
            }),
        ("floor lamp",
            FurnitureTemplate {
                powered: Some(pwr("floor lamp", PowerSource::Mains, true, None, 0.0)),
                light_lumens: Some(800.0),
                ..base(Lighting, 'L', "iron", "tall arc floor lamp")
            }),
        ("chandelier",
            FurnitureTemplate {
                powered: Some(pwr("chandelier", PowerSource::Mains, true, None, 0.0)),
                light_lumens: Some(2400.0),
                ..base(Lighting, 'X', "iron", "crystal chandelier")
            }),
        ("sconce",
            FurnitureTemplate {
                powered: Some(pwr("sconce", PowerSource::Mains, true, None, 0.0)),
                light_lumens: Some(300.0),
                ..base(Lighting, 'i', "iron", "wall sconce")
            }),

        // ─── wall art ──────────────────────────────────────────────
        ("painting",
            FurnitureTemplate {
                painting: Some(PaintingSpec {
                    artist: "unknown".into(),
                    title: "untitled landscape".into(),
                    value: 200,
                }),
                ..base(WallArt, 'P', "canvas", "framed painting")
            }),
        ("oil portrait",
            FurnitureTemplate {
                painting: Some(PaintingSpec {
                    artist: "Alana Vermeer".into(),
                    title: "Portrait of Mrs. Vance".into(),
                    value: 4500,
                }),
                quality: Some(Quality::Antique),
                style: Some(Style::Victorian),
                base_value: 1500,
                ..base(WallArt, 'P', "canvas", "oil portrait, gilded frame")
            }),
        ("abstract canvas",
            FurnitureTemplate {
                painting: Some(PaintingSpec {
                    artist: "Min Park".into(),
                    title: "Red Sequence #4".into(),
                    value: 18000,
                }),
                quality: Some(Quality::Masterwork),
                style: Some(Style::Modern),
                base_value: 6000,
                ..base(WallArt, 'P', "canvas", "modern abstract — bold reds")
            }),
        ("photograph",
            FurnitureTemplate {
                painting: Some(PaintingSpec {
                    artist: "family".into(),
                    title: "wedding day".into(),
                    value: 0,
                }),
                ..base(WallArt, 'p', "paper", "framed family photo")
            }),
        ("mirror", base(WallArt, 'M', "glass", "wall mirror")),
        ("wall clock", base(WallArt, 'C', "wood", "round wall clock")),

        // ─── floor coverings ───────────────────────────────────────
        ("persian rug",
            FurnitureTemplate {
                rug: Some(RugSpec { friction: 0.85 }),
                ..base(FloorCovering, 'r', "wool", "patterned wool rug")
            }),
        ("kitchen mat",
            FurnitureTemplate {
                rug: Some(RugSpec { friction: 0.90 }),
                ..base(FloorCovering, 'm', "rubber", "anti-fatigue kitchen mat")
            }),
        ("bath mat",
            FurnitureTemplate {
                rug: Some(RugSpec { friction: 0.85 }),
                ..base(FloorCovering, 'm', "cotton", "bath mat")
            }),

        // ─── windows ───────────────────────────────────────────────
        ("window",
            FurnitureTemplate {
                window: Some(WindowSpec { closed: true }),
                ..base(Window, 'i', "glass", "window pane, closed")
            }),
        ("open window",
            FurnitureTemplate {
                window: Some(WindowSpec { closed: false }),
                ..base(Window, '/', "glass", "window pane, open")
            }),
        ("sliding glass door",
            FurnitureTemplate {
                window: Some(WindowSpec { closed: true }),
                ..base(Window, '|', "glass", "sliding glass door, closed")
            }),
        ("open sliding door",
            FurnitureTemplate {
                window: Some(WindowSpec { closed: false }),
                ..base(Window, '/', "glass", "sliding glass door, open")
            }),

        // ─── decor ─────────────────────────────────────────────────
        ("potted plant", base(Decor, '%', "plaster", "fern in a clay pot")),
        ("vase",         base(Decor, 'v', "porcelain", "ceramic vase, dried flowers")),
        ("books",        base(Decor, 'b', "paper", "stack of hardcovers")),
        ("candle",
            FurnitureTemplate {
                light_lumens: Some(80.0),
                ..base(Decor, 'c', "wax", "lit candle in a holder")
            }),

        // ─── structure ─────────────────────────────────────────────
        ("staircase up",   base(Structure, '>', "oak", "carpeted staircase to upper floor")),
        ("staircase down", base(Structure, '<', "oak", "staircase down to basement")),
        ("column",         base(Structure, '|', "marble", "decorative marble column")),
        ("railing",        base(Structure, '-', "oak", "wooden banister")),

        // ─── outdoor ───────────────────────────────────────────────
        ("grill",
            FurnitureTemplate {
                powered: Some(pwr("grill", PowerSource::Gas, false, None, 0.0)),
                ..base(Outdoor, 'g', "steel", "propane grill")
            }),
        ("patio chair", base(Outdoor, 'h', "aluminum", "weather-resistant chair")),
        ("patio table", base(Outdoor, 't', "aluminum", "round patio table")),
        ("hammock",      sized(Outdoor, '~', "cotton",   (3, 1), "rope hammock between two posts")),
        ("mailbox",      sized(Outdoor, 'M', "aluminum", (1, 1), "curbside mailbox on a post")),
        ("garden gnome", sized(Outdoor, 'g', "ceramic",  (1, 1), "smug little ceramic gnome")),
        ("pool",         sized(Outdoor, '~', "tile",     (5, 3), "in-ground swimming pool")),
        ("hot tub",      sized(Outdoor, '@', "tile",     (2, 2), "outdoor hot tub")),
    ];

    for (name, tmpl) in entries {
        lib.furniture.insert((*name).into(), tmpl.clone());
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
        assert!(lib.furniture.len() >= 20);
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
