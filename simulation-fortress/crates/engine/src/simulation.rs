use std::time::Duration;

use crate::entity::EntityStore;
use crate::log::EventLog;
use crate::render::{NullRenderer, Renderer, SimulationView};
use crate::scenario::{Scenario, SetupContext, TickContext};
use crate::time::{Clock, Tick};
use crate::world::World;

pub struct Simulation {
    pub world: World,
    pub entities: EntityStore,
    pub clock: Clock,
    pub log: EventLog,
}

#[derive(Clone, Copy, Debug)]
pub struct RunOptions {
    pub max_ticks: u64,
    /// If `Some`, sleep this long after each rendered frame so the
    /// simulation plays back at human-readable speed.
    pub pacing: Option<Duration>,
}

impl RunOptions {
    pub fn fast(max_ticks: u64) -> Self {
        Self {
            max_ticks,
            pacing: None,
        }
    }

    pub fn paced(max_ticks: u64, pacing: Duration) -> Self {
        Self {
            max_ticks,
            pacing: Some(pacing),
        }
    }
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            max_ticks: 1_000,
            pacing: Some(Duration::from_millis(500)),
        }
    }
}

impl Simulation {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            entities: EntityStore::default(),
            clock: Clock::new(),
            log: EventLog::new(),
        }
    }

    pub fn view(&self) -> SimulationView<'_> {
        SimulationView {
            world: &self.world,
            entities: &self.entities,
            log: &self.log,
        }
    }

    /// Advance the clock by one tick and call the scenario's `tick` hook.
    pub fn step<S: Scenario>(&mut self, scenario: &mut S) -> Tick {
        let tick = self.clock.advance();
        let mut ctx = TickContext {
            world: &mut self.world,
            entities: &mut self.entities,
            log: &mut self.log,
            tick,
        };
        scenario.tick(&mut ctx);
        tick
    }

    /// Run the scenario headlessly with default options.
    pub fn run<S: Scenario>(&mut self, scenario: &mut S, options: RunOptions) -> Tick {
        self.run_with(scenario, options, &mut NullRenderer)
    }

    /// Run the scenario, calling `renderer.frame` after setup and after
    /// every tick. Stops early if the scenario reports completion.
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
        {
            let mut ctx = SetupContext {
                world: &mut self.world,
                entities: &mut self.entities,
                log: &mut self.log,
            };
            scenario.setup(&mut ctx);
        }
        renderer.frame(&self.view(), 0);
        if let Some(pace) = options.pacing {
            std::thread::sleep(pace);
        }

        for _ in 0..options.max_ticks {
            let tick = self.step(scenario);
            renderer.frame(&self.view(), tick);
            if scenario.is_complete(&self.world, &self.entities, tick) {
                break;
            }
            if let Some(pace) = options.pacing {
                std::thread::sleep(pace);
            }
        }
        self.clock.tick
    }
}

impl Default for Simulation {
    fn default() -> Self {
        Self::new()
    }
}
