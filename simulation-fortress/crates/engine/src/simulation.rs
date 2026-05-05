use std::time::Duration;

use bevy_ecs::prelude::World;

use crate::log::EventLog;
use crate::render::{NullRenderer, Renderer};
use crate::rng::Rng;
use crate::scenario::Scenario;
use crate::time::{Clock, Tick};
use crate::world::VoxelWorld;

pub struct Simulation {
    pub world: World,
}

#[derive(Clone, Copy, Debug)]
pub struct RunOptions {
    pub max_ticks: u64,
    /// If `Some`, sleep this long after each rendered frame so the
    /// simulation plays back at human-readable speed.
    pub pacing: Option<Duration>,
    /// Seed for the engine RNG resource. Same seed -> same run.
    pub rng_seed: u64,
}

impl RunOptions {
    pub fn fast(max_ticks: u64) -> Self {
        Self {
            max_ticks,
            pacing: None,
            ..Self::default()
        }
    }

    pub fn paced(max_ticks: u64, pacing: Duration) -> Self {
        Self {
            max_ticks,
            pacing: Some(pacing),
            ..Self::default()
        }
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.rng_seed = seed;
        self
    }
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            max_ticks: 1_000,
            pacing: Some(Duration::from_millis(500)),
            rng_seed: 0xCAFE_BABE_DEAD_BEEF,
        }
    }
}

impl Simulation {
    pub fn new() -> Self {
        let mut world = World::new();
        world.insert_resource(VoxelWorld::new());
        world.insert_resource(Clock::default());
        world.insert_resource(EventLog::default());
        Self { world }
    }

    pub fn current_tick(&self) -> Tick {
        self.world.resource::<Clock>().tick
    }

    /// Run the scenario headlessly with the given options.
    pub fn run<S: Scenario>(&mut self, scenario: &mut S, options: RunOptions) -> Tick {
        self.run_with(scenario, options, &mut NullRenderer)
    }

    /// Run the scenario with a renderer attached. The renderer is invoked
    /// after `setup` (tick 0) and after every subsequent tick. The loop
    /// stops early when `Scenario::is_complete` returns true.
    pub fn run_with<S, R>(
        &mut self,
        scenario: &mut S,
        options: RunOptions,
        renderer: &mut R,
    ) -> Tick
    where
        S: Scenario,
        R: Renderer,
    {
        self.world.insert_resource(Rng::from_seed(options.rng_seed));
        scenario.setup(&mut self.world);
        let mut schedule = scenario.build_schedule();
        renderer.frame(&mut self.world, 0);
        if let Some(pace) = options.pacing {
            std::thread::sleep(pace);
        }

        for _ in 0..options.max_ticks {
            self.world.resource_mut::<Clock>().advance();
            schedule.run(&mut self.world);
            let tick = self.current_tick();
            renderer.frame(&mut self.world, tick);
            if scenario.is_complete(&mut self.world) {
                break;
            }
            if let Some(pace) = options.pacing {
                std::thread::sleep(pace);
            }
        }
        self.current_tick()
    }
}

impl Default for Simulation {
    fn default() -> Self {
        Self::new()
    }
}
