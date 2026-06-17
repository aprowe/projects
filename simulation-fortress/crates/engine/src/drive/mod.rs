//! Utility-AI drives.
//!
//! A `Drive` is a small piece of behavior: given the current world
//! and an actor, it scores how relevant it is right now (0..=1) and,
//! if chosen, queues a single `Task` (or short task chain) on the
//! actor's `TaskQueue`. The set of drives an entity carries is
//! attached as a `Drives` component.
//!
//! The `drive_planner` system runs every tick: for every actor with
//! `Drives`, it scores each drive against the current world and
//! enacts the highest-scoring one — *if* the actor's `TaskQueue` is
//! empty (no current plan) **or** the best drive's score exceeds an
//! "interrupt threshold" (urgent drives like fleeing override
//! whatever you were doing).
//!
//! Drives are deliberately portable across scenarios:
//!
//!  - The drive *type* lives in the engine (this module + child
//!    modules).
//!  - The drive *instance* on a specific actor comes from the role
//!    template (most common) or from scenario setup (custom plot
//!    drives).
//!  - A drive's *relevance* is decided inside `score()` by reading
//!    the world. So a `WinFoodFight` drive on a kid scores zero in
//!    a quiet living room but 0.7 in a chaotic cafeteria — same
//!    kid, same drive, different context.

use std::fmt;

use bevy_ecs::prelude::{Component, Entity, World};

use crate::tasks::TaskQueue;

pub mod builtin;

pub use builtin::*;

/// Anything that can rate itself against the world and act when chosen.
pub trait Drive: Send + Sync {
    fn name(&self) -> &str;

    /// 0.0 = irrelevant, 1.0 = absolutely must do now. Anything
    /// returning < `THRESHOLD_ACT` is ignored. Drives returning
    /// >= `THRESHOLD_INTERRUPT` clear the existing task queue.
    fn score(&self, world: &mut World, actor: Entity) -> f32;

    /// Push tasks onto `actor`'s `TaskQueue`. May insert components
    /// (Locomotion, Goal, etc.). Should not assume the queue is
    /// already empty — `drive_planner` clears it for interrupts.
    fn enact(&self, world: &mut World, actor: Entity);
}

impl fmt::Debug for dyn Drive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Drive::{}", self.name())
    }
}

/// Below this score, drives are completely ignored.
pub const THRESHOLD_ACT: f32 = 0.10;
/// At or above this, the drive interrupts the actor's current plan.
pub const THRESHOLD_INTERRUPT: f32 = 0.70;

/// The set of drives an entity considers each tick.
#[derive(Component, Default)]
pub struct Drives(pub Vec<Box<dyn Drive>>);

impl Drives {
    pub fn new() -> Self { Self::default() }
    pub fn with(mut self, drive: Box<dyn Drive>) -> Self {
        self.0.push(drive);
        self
    }
    pub fn push(&mut self, drive: Box<dyn Drive>) {
        self.0.push(drive);
    }
    pub fn names(&self) -> Vec<&str> {
        self.0.iter().map(|d| d.name()).collect()
    }
}

/// Engine system. Runs every tick (typically before `execute_tasks`).
/// Picks the top-scoring drive per actor and enacts it.
pub fn drive_planner(world: &mut World) {
    // Collect actor entities first; we'll detach + reattach Drives
    // around the score/enact calls so the drives can mut-borrow
    // the world.
    let actors: Vec<Entity> = {
        let mut q = world.query_filtered::<Entity, bevy_ecs::query::With<Drives>>();
        q.iter(world).collect()
    };

    for actor in actors {
        let queue_empty = world
            .get::<TaskQueue>(actor)
            .map(|q| q.is_empty())
            .unwrap_or(true);

        // Detach Drives so we can mut-borrow world during scoring.
        let drives = match world.entity_mut(actor).take::<Drives>() {
            Some(d) => d,
            None => continue,
        };

        // Score every drive.
        let mut best_idx: Option<usize> = None;
        let mut best_score = THRESHOLD_ACT;
        for (i, d) in drives.0.iter().enumerate() {
            let s = d.score(world, actor);
            if s > best_score {
                best_score = s;
                best_idx = Some(i);
            }
        }

        if let Some(i) = best_idx {
            let interrupt = best_score >= THRESHOLD_INTERRUPT;
            if interrupt || queue_empty {
                if interrupt && !queue_empty {
                    if let Some(mut q) = world.get_mut::<TaskQueue>(actor) {
                        q.clear();
                    }
                }
                drives.0[i].enact(world, actor);
            }
        }

        // Reattach Drives.
        world.entity_mut(actor).insert(drives);
    }
}
