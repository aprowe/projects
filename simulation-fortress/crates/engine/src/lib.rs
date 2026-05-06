//! Fortress engine: a voxel-based simulation core built on `bevy_ecs`.
//!
//! The engine provides primitives for Dwarf-Fortress-style simulations:
//! a chunked voxel world (`VoxelWorld`), creature components
//! (`Kind`, `Position`, `Health`, `Faction`, `ExtraData`), a tick clock,
//! a structured event log, A* pathfinding, and renderers (ASCII map +
//! prose narrator). Scenarios drive simulations by populating the bevy
//! ECS world during `setup` and registering systems on a `Schedule`.

pub mod actions;
pub mod anatomy;
/// Combat resolution + dice rolls (grouped subfolder: combat/{mod,dice}).
pub mod combat;
pub mod components;
pub mod dialog;
pub mod door;
pub mod fire;
pub mod furniture;
pub mod inject;
/// Items, blemishes, quality (grouped subfolder: items/{mod,blemish,quality}).
pub mod items;
pub mod library;
pub mod log;
pub mod needs;
pub mod npc_schedule;
pub mod pathfind;
pub mod physics;
pub mod render;
pub mod rng;
pub mod scenario;
pub mod simulation;
pub mod sound;
pub mod stats;
pub mod status;
pub mod tasks;
pub mod time;
pub mod world;

// Backward-compat top-level paths so internal `use crate::dice::…`
// and `use crate::blemish::…` etc. keep working after the move.
pub use combat::dice;
pub use items::blemish;
pub use items::quality;

pub use actions::{fill_region_logged, note, set_voxel_logged, spawn_creature};
pub use blemish::{
    adjusted_value, describe_blemishes, Blemish, BlemishKind, Blemishes, Finish,
};
pub use anatomy::{
    anatomy_alive, apply_body_plan, bird_body_plan, default_functions, dragon_body_plan,
    function_capacity, humanoid_body_plan, quadruped_body_plan, snake_body_plan,
    spawn_humanoid_body, BodyPart, BodyPartKind, BodyPartSpec, BodyPlan, Function, HitWeight,
    PartHealth, PartLabel, PartOf, PartStatus, ProvidesFunctions,
};
pub use combat::{armor_class, resolve_attack, AttackResult};
pub use dialog::{
    dialog_system, observation_system, Alarmed, Clearance, Conversation, DialogLine, Disguise,
    Identity, Knowledge, Observer, Suspicion,
};
pub use door::{door_at, door_voxel_sync, Door, DoorState};
pub use fire::{tick_fire, Burning};
pub use furniture::{
    furniture_emit_system, tick_carrying, Carried, Climbable, Container, Furniture,
    FurnitureKind, Haulable, LightSource, Painting, PowerSource, Powered, Rug, Stair, Window,
    WindowState,
};
pub use inject::{apply_action, apply_json, Action};
pub use library::{
    ensure_material, spawn_furniture_template, spawn_item_template, spawn_role_template,
    ContainerSpec, FurnitureSpawnOpts, FurnitureTemplate, ItemSpawnOpts, ItemTemplate, Library,
    LibraryHit, PaintingSpec, PoweredSpec, RoleSpawnOpts, RoleTemplate, RugSpec, WindowSpec,
};
pub use physics::{decay_coatings, footing_check, Coating, Locomotion};
pub use components::{ExtraData, Faction, Health, Kind, Position};
pub use dice::{
    check, manipulation_check, roll_d20, roll_dice, strength_check, CheckOutcome, RollResult,
};
pub use items::{
    drop_item, equip_item, give_item, held_weapons_summary, item_world_position, unequip_item,
    Ammo, AmmoKind, ArmorBonus, BodySlot, DamageDice, ElectricalConductivity, Inventory, Item,
    ItemMaterial, ItemName, Mass, RangedWeapon, Temperature, Texture, ThermalConductivity,
    TwoHanded, Wearable, Wearing,
};
pub use stats::Stats;
pub use status::{apply_status, tick_status_effects, StatusEffect, StatusEffects, StatusKind};
pub use log::{narrate, Event, EventLog};
pub use needs::{
    derive_mood, eat_on_use, fear_from_combat, tick_needs, Edible, Energy, Fear, Hunger, Mood,
};
pub use npc_schedule::{tick_schedules, Activity, Schedule, ScheduleEntry};
pub use pathfind::find_path;
pub use quality::{narrate_label, Paint, Quality, Style, Value};
pub use render::{
    AsciiRenderer, CompositeRenderer, LogRenderer, NullRenderer, Renderer,
};
pub use rng::Rng;
pub use scenario::Scenario;
pub use simulation::{RunOptions, Simulation};
pub use sound::{
    emit_combat_sounds, emit_movement_sounds, emit_scream, line_of_sight_blocked, update_hearing,
    update_sight, update_smell, HeardSound, Hearing, Perceived, SeenEntity, Sight, Smell,
    SmelledOdor, SoundKind,
};
pub use tasks::{
    execute_tasks, retaliation_system, Goal, RetaliateOnAttack, Task, TaskQueue,
};
pub use time::{weather_system, Clock, TimeOfDay, Tick, Weather, WeatherKind};
pub use world::{
    Chunk, Material, MaterialId, Pos, TileKind, Voxel, VoxelWorld, AIR, CHUNK_SIZE,
};

/// Re-export bevy ECS prelude items most commonly used by scenarios.
pub mod prelude {
    pub use bevy_ecs::prelude::{
        Component, Entity, IntoScheduleConfigs, Query, Res, ResMut, Resource, Schedule, World,
    };
    pub use bevy_ecs::query::{With, Without};
}
