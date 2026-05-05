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
pub mod combat;
pub mod components;
pub mod inject;
pub mod items;
pub mod library;
pub mod log;
pub mod needs;
pub mod pathfind;
pub mod physics;
pub mod render;
pub mod rng;
pub mod scenario;
pub mod simulation;
pub mod sound;
pub mod tasks;
pub mod time;
pub mod world;

pub use actions::{fill_region_logged, note, set_voxel_logged, spawn_creature};
pub use anatomy::{
    anatomy_alive, apply_body_plan, default_functions, dragon_body_plan, function_capacity,
    humanoid_body_plan, quadruped_body_plan, spawn_humanoid_body, BodyPart, BodyPartKind,
    BodyPartSpec, BodyPlan, Function, HitWeight, PartHealth, PartLabel, PartOf, PartStatus,
    ProvidesFunctions,
};
pub use combat::{resolve_attack, AttackResult};
pub use inject::{apply_action, apply_json, Action};
pub use library::{
    ensure_material, spawn_item_template, spawn_role_template, ItemSpawnOpts, ItemTemplate,
    Library, LibraryHit, RoleSpawnOpts, RoleTemplate,
};
pub use physics::{footing_check, Coating, Locomotion};
pub use components::{ExtraData, Faction, Health, Kind, Position};
pub use items::{
    drop_item, equip_item, give_item, held_weapons_summary, item_world_position, unequip_item,
    BodySlot, ElectricalConductivity, Inventory, Item, ItemMaterial, ItemName, Mass, Temperature,
    Texture, ThermalConductivity, Wearable, Wearing,
};
pub use log::{narrate, Event, EventLog};
pub use needs::{
    derive_mood, eat_on_use, fear_from_combat, tick_needs, Edible, Energy, Fear, Hunger, Mood,
};
pub use pathfind::find_path;
pub use render::{
    AsciiRenderer, CompositeRenderer, LogRenderer, NullRenderer, Renderer,
};
pub use rng::Rng;
pub use scenario::Scenario;
pub use simulation::{RunOptions, Simulation};
pub use sound::{
    emit_combat_sounds, emit_movement_sounds, emit_scream, update_hearing, HeardSound, Hearing,
    Perceived, SoundKind,
};
pub use tasks::{
    execute_tasks, retaliation_system, Goal, RetaliateOnAttack, Task, TaskQueue,
};
pub use time::{Clock, Tick};
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
