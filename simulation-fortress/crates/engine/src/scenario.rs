//! Scenario lifecycle.
//!
//! A scenario describes one self-contained simulation. It has three
//! responsibilities:
//!
//! 1. `setup` populates the bevy ECS world with entities and configures
//!    the `VoxelWorld` resource.
//! 2. `build_schedule` returns a `Schedule` whose systems implement the
//!    per-tick logic. The simulation runs this schedule once per tick.
//! 3. `is_complete` reports whether the run should end early.
//!
//! Scenarios can register their own components, resources, and events
//! on the world during `setup`; the engine just calls the trait methods
//! at the right time and offers helpers (see `crate::actions`) for
//! mutations that should auto-emit log events.

use bevy_ecs::prelude::{Schedule, World};

pub trait Scenario {
    fn name(&self) -> &str;

    fn setup(&mut self, world: &mut World);

    fn build_schedule(&mut self) -> Schedule;

    fn is_complete(&self, _world: &mut World) -> bool {
        false
    }
}
