//! Fortress engine: a voxel-based simulation core.
//!
//! The engine provides primitives for building Dwarf-Fortress-style
//! simulations: a chunked voxel world, materials, entities with arbitrary
//! per-scenario data, a tick-based clock, and a `Scenario` trait that
//! drives setup and per-tick behavior.

pub mod entity;
pub mod log;
pub mod render;
pub mod scenario;
pub mod simulation;
pub mod time;
pub mod world;

pub use entity::{Entity, EntityId, EntityStore};
pub use log::{narrate, Event, EventLog};
pub use render::{AsciiRenderer, CompositeRenderer, LogRenderer, NullRenderer, Renderer, SimulationView};
pub use scenario::{Scenario, SetupContext, TickContext};
pub use simulation::{RunOptions, Simulation};
pub use time::{Clock, Tick};
pub use world::{Chunk, Material, MaterialId, Pos, Voxel, World, AIR, CHUNK_SIZE};
