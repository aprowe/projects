//! Runtime event injection: a typed `Action` enum that mutates the
//! ECS world, plus an `apply_action` function that interprets one.
//!
//! The Action enum is `serde::Deserialize` so external callers (CLI
//! REPL, future LLM front-end, scripts) can drive the simulation by
//! emitting JSON. Every variant maps onto an existing engine helper —
//! this module is a thin, well-typed surface, not new behavior.
//!
//! Entities are referenced by either `Kind` name (`"intruder"`) or
//! raw entity index (`{"index": 75}`); the resolver walks the ECS to
//! find a match. Materials are referenced by registry name
//! (`"wood"`).

use bevy_ecs::prelude::{Entity, World};
use serde::Deserialize;

use crate::actions::{fill_region_logged, note as note_helper, set_voxel_logged, spawn_creature};
use crate::anatomy::{
    apply_body_plan, dragon_body_plan, humanoid_body_plan, quadruped_body_plan,
};
use crate::combat::resolve_attack;
use crate::components::{Kind, Position};
use crate::items::{
    equip_item as engine_equip, give_item, BodySlot, Item, ItemMaterial, ItemName, Mass,
    Temperature, Texture, Wearable,
};
use crate::physics::Coating;
use crate::tasks::{Goal, Task, TaskQueue};
use crate::world::{MaterialId, Pos, Voxel, VoxelWorld};

/// One mutation against the simulation world.
#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "action")]
pub enum Action {
    /// Push a free-form note into the event log.
    Note { text: String },
    /// Replace one voxel.
    SetVoxel { at: PosLike, tile: TileSpec },
    /// Fill an axis-aligned region with a tile.
    FillRegion {
        min: PosLike,
        max: PosLike,
        tile: TileSpec,
    },
    /// Spawn a creature with the given body plan, optionally tagged
    /// with a faction.
    SpawnCreature {
        name: String,
        at: PosLike,
        #[serde(default)]
        faction: Option<String>,
        #[serde(default = "default_health")]
        health: i32,
        #[serde(default = "default_body_plan")]
        body_plan: String,
    },
    /// Spawn an item with the given physical traits. If `at` is set
    /// it lands on the floor; if `equip_on` is set it goes straight
    /// onto a creature's body slot (overriding `at`).
    SpawnItem {
        name: String,
        #[serde(default)]
        at: Option<PosLike>,
        #[serde(default = "default_mass")]
        mass: f32,
        #[serde(default = "default_temperature")]
        temperature: f32,
        #[serde(default)]
        texture: Option<TextureName>,
        #[serde(default)]
        wearable: Option<BodySlotName>,
        #[serde(default)]
        material: Option<String>,
        #[serde(default)]
        equip_on: Option<EntityRef>,
        #[serde(default)]
        give_to: Option<EntityRef>,
    },
    /// Coat a tile with the given material — a puddle, slick, blood
    /// splatter, dust patch. The footing-check system reads the
    /// material's `friction` to decide whether passing creatures
    /// slip; flammability/conductivity are wired through whenever
    /// those systems exist.
    Coat {
        at: PosLike,
        material: String,
        #[serde(default = "default_volume")]
        volume: f32,
    },
    /// Resolve a single attack: attacker swings their main-hand
    /// weapon (or fist) at target, picking a body part by hit weight.
    Attack {
        attacker: EntityRef,
        target: EntityRef,
    },
    /// Push a task onto an actor's queue.
    QueueTask { actor: EntityRef, task: TaskSpec },
    /// Equip an item that's already in the wearer's inventory or
    /// floating around, on its `Wearable` slot.
    Equip { wearer: EntityRef, item: EntityRef },
    /// Give an item directly into a holder's inventory.
    Give { holder: EntityRef, item: EntityRef },
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct PosLike {
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub z: i32,
}

impl PosLike {
    pub fn to_pos(self) -> Pos {
        Pos::new(self.x, self.y, self.z)
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct TileSpec {
    pub tile: TileKindName,
    pub material: String,
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum TileKindName {
    Empty,
    Floor,
    Wall,
    Ramp,
}

impl TileKindName {
    fn voxel(self, material: MaterialId) -> Voxel {
        match self {
            TileKindName::Empty => Voxel::empty(),
            TileKindName::Floor => Voxel::floor(material),
            TileKindName::Wall => Voxel::wall(material),
            TileKindName::Ramp => Voxel::ramp(material),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum EntityRef {
    /// Match by `Kind` name (first matching alive entity).
    ByKind(String),
    /// Match by raw `Entity` index.
    Index { index: u32 },
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum TextureName {
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

impl TextureName {
    fn texture(self) -> Texture {
        match self {
            TextureName::Smooth => Texture::Smooth,
            TextureName::Rough => Texture::Rough,
            TextureName::Coarse => Texture::Coarse,
            TextureName::Soft => Texture::Soft,
            TextureName::Sharp => Texture::Sharp,
            TextureName::Slick => Texture::Slick,
            TextureName::Sticky => Texture::Sticky,
            TextureName::Furry => Texture::Furry,
            TextureName::Polished => Texture::Polished,
            TextureName::Bumpy => Texture::Bumpy,
        }
    }
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum BodySlotName {
    Head,
    Torso,
    Legs,
    Feet,
    Hands,
    MainHand,
    OffHand,
    Back,
}

impl BodySlotName {
    fn slot(self) -> BodySlot {
        match self {
            BodySlotName::Head => BodySlot::Head,
            BodySlotName::Torso => BodySlot::Torso,
            BodySlotName::Legs => BodySlot::Legs,
            BodySlotName::Feet => BodySlot::Feet,
            BodySlotName::Hands => BodySlot::Hands,
            BodySlotName::MainHand => BodySlot::MainHand,
            BodySlotName::OffHand => BodySlot::OffHand,
            BodySlotName::Back => BodySlot::Back,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum TaskSpec {
    MoveTo { at: PosLike },
    Attack { target: EntityRef },
    Wait { ticks: u32 },
    UseEntity { target: EntityRef },
    PickUp { item: EntityRef },
    Equip { item: EntityRef },
}

fn default_health() -> i32 {
    100
}
fn default_mass() -> f32 {
    1.0
}
fn default_temperature() -> f32 {
    20.0
}
fn default_body_plan() -> String {
    "humanoid".into()
}
fn default_volume() -> f32 {
    1.0
}

/// Apply one action to the world. On success returns a short
/// description of what happened (suitable for echoing to a REPL).
pub fn apply_action(world: &mut World, action: &Action) -> Result<String, String> {
    match action {
        Action::Note { text } => {
            note_helper(world, text.clone());
            Ok(format!("noted: {text}"))
        }
        Action::SetVoxel { at, tile } => {
            let mat = resolve_material(world, &tile.material)?;
            set_voxel_logged(world, at.to_pos(), tile.tile.voxel(mat));
            Ok(format!("voxel set at ({}, {}, {})", at.x, at.y, at.z))
        }
        Action::FillRegion { min, max, tile } => {
            let mat = resolve_material(world, &tile.material)?;
            fill_region_logged(world, min.to_pos(), max.to_pos(), tile.tile.voxel(mat));
            Ok(format!(
                "filled region ({}, {}, {})..({}, {}, {})",
                min.x, min.y, min.z, max.x, max.y, max.z
            ))
        }
        Action::SpawnCreature {
            name,
            at,
            faction,
            health,
            body_plan,
        } => {
            let entity =
                spawn_creature(world, name.clone(), at.to_pos(), *health, faction.as_deref());
            let plan = match body_plan.as_str() {
                "humanoid" => humanoid_body_plan(),
                "quadruped" => quadruped_body_plan(),
                "dragon" => dragon_body_plan(),
                other => return Err(format!("unknown body plan: {other}")),
            };
            apply_body_plan(world, entity, &plan);
            world
                .entity_mut(entity)
                .insert(TaskQueue::default())
                .insert(Goal::default());
            Ok(format!(
                "spawned {} (#{}) with {} body",
                name,
                entity.index(),
                body_plan
            ))
        }
        Action::SpawnItem {
            name,
            at,
            mass,
            temperature,
            texture,
            wearable,
            material,
            equip_on,
            give_to,
        } => {
            let material_id = if let Some(mat_name) = material {
                Some(resolve_material(world, mat_name)?)
            } else {
                None
            };
            let id = {
                let mut e = world.spawn((
                    Item,
                    ItemName::new(name.clone()),
                    Mass(*mass),
                    Temperature(*temperature),
                ));
                if let Some(t) = texture {
                    e.insert(t.texture());
                }
                if let Some(slot) = wearable {
                    e.insert(Wearable(slot.slot()));
                }
                if let Some(p) = at {
                    e.insert(Position(p.to_pos()));
                }
                if let Some(mat) = material_id {
                    e.insert(ItemMaterial(mat));
                }
                e.id()
            };
            if let Some(holder_ref) = equip_on {
                let holder = resolve_entity(world, holder_ref)?;
                engine_equip(world, holder, id);
            } else if let Some(holder_ref) = give_to {
                let holder = resolve_entity(world, holder_ref)?;
                give_item(world, holder, id);
            }
            Ok(format!("spawned item {} (#{})", name, id.index()))
        }
        Action::Coat {
            at,
            material,
            volume,
        } => {
            let mat = resolve_material(world, material)?;
            let mat_name = world
                .resource::<VoxelWorld>()
                .material(mat)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| material.clone());
            let kind_label = format!("{mat_name}_spill");
            world.spawn((
                Position(at.to_pos()),
                Kind(kind_label.clone()),
                Coating {
                    material: mat,
                    volume: *volume,
                },
            ));
            Ok(format!(
                "coated ({}, {}, {}) with {} (vol {})",
                at.x, at.y, at.z, mat_name, volume
            ))
        }
        Action::Attack { attacker, target } => {
            let a = resolve_entity(world, attacker)?;
            let t = resolve_entity(world, target)?;
            resolve_attack(world, a, t);
            Ok(format!(
                "#{} attacks #{}",
                a.index(),
                t.index()
            ))
        }
        Action::QueueTask { actor, task } => {
            let actor_e = resolve_entity(world, actor)?;
            let task = build_task(world, task)?;
            let label = task.label();
            if !world.entity(actor_e).contains::<TaskQueue>() {
                world.entity_mut(actor_e).insert(TaskQueue::default());
            }
            if let Some(mut q) = world.get_mut::<TaskQueue>(actor_e) {
                q.push(task);
            }
            Ok(format!("queued {} on #{}", label, actor_e.index()))
        }
        Action::Equip { wearer, item } => {
            let w = resolve_entity(world, wearer)?;
            let i = resolve_entity(world, item)?;
            engine_equip(world, w, i);
            Ok(format!("#{} equipped #{}", w.index(), i.index()))
        }
        Action::Give { holder, item } => {
            let h = resolve_entity(world, holder)?;
            let i = resolve_entity(world, item)?;
            give_item(world, h, i);
            Ok(format!("#{} given to #{}", i.index(), h.index()))
        }
    }
}

fn build_task(world: &mut World, spec: &TaskSpec) -> Result<Task, String> {
    Ok(match spec {
        TaskSpec::MoveTo { at } => Task::MoveTo(at.to_pos()),
        TaskSpec::Attack { target } => Task::Attack(resolve_entity(world, target)?),
        TaskSpec::Wait { ticks } => Task::Wait(*ticks),
        TaskSpec::UseEntity { target } => Task::UseEntity(resolve_entity(world, target)?),
        TaskSpec::PickUp { item } => Task::PickUp(resolve_entity(world, item)?),
        TaskSpec::Equip { item } => Task::Equip(resolve_entity(world, item)?),
    })
}

fn resolve_material(world: &World, name: &str) -> Result<MaterialId, String> {
    world
        .resource::<VoxelWorld>()
        .material_id(name)
        .ok_or_else(|| format!("unknown material: {name}"))
}

fn resolve_entity(world: &mut World, eref: &EntityRef) -> Result<Entity, String> {
    match eref {
        EntityRef::Index { index } => {
            let target = *index;
            let mut q = world.query::<Entity>();
            q.iter(world)
                .find(|e| (e.to_bits() & 0xFFFF_FFFF) as u32 == target)
                .ok_or_else(|| format!("no entity with index {target}"))
        }
        EntityRef::ByKind(name) => {
            let mut q = world.query::<(Entity, &Kind)>();
            q.iter(world)
                .find(|(_, k)| k.0 == *name)
                .map(|(e, _)| e)
                .ok_or_else(|| format!("no entity with kind '{name}'"))
        }
    }
}

/// Parse a JSON string as either one Action or an array of them, then
/// apply each in order. Returns a vector of outcome messages.
pub fn apply_json(world: &mut World, json: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("parse error: {e}"))?;
    let actions: Vec<Action> = if value.is_array() {
        serde_json::from_value(value).map_err(|e| format!("parse error: {e}"))?
    } else {
        vec![serde_json::from_value::<Action>(value).map_err(|e| format!("parse error: {e}"))?]
    };
    let mut out = Vec::with_capacity(actions.len());
    for action in actions {
        out.push(apply_action(world, &action)?);
    }
    Ok(out)
}
