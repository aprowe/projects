//! Task system: how actors decide what to do.
//!
//! Three layers:
//!
//! - `Goal` — high-level intent (Idle, Kill, Tend, GoTo). Set by a
//!   scenario's "behavior" or "planner" system based on world state.
//! - `TaskQueue` — ordered low-level actions to satisfy the goal,
//!   each one a `Task` variant. Filled by the planner; drained by the
//!   executor.
//! - `Task` — a single concrete action (move, attack, use, wait, …).
//!
//! The engine's `execute_tasks` system advances the head task each
//! tick. It handles built-in variants (movement, attack auto-pursue,
//! item pickup/equip, waits, generic "use entity") natively. Anything
//! else is the scenario's job to react to.
//!
//! Two opt-in helpers ride along here too: `RetaliateOnAttack` queues
//! a counter-attack when the wearer is hit, and `retaliation_system`
//! reads the event log to do the queueing.

use std::collections::VecDeque;

use bevy_ecs::prelude::{Component, Entity, World};

use crate::combat::resolve_attack;
use crate::components::{Health, Position};
use crate::items::{equip_item as engine_equip_item, give_item};
use crate::log::{Event, EventLog};
use crate::pathfind::find_path;
use crate::time::Clock;
use crate::world::{Pos, VoxelWorld};

/// What the actor wants to be doing right now. Replaceable by the
/// scenario's planner when world state changes; the engine doesn't
/// auto-pick goals — it just stores and reports them.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub enum Goal {
    #[default]
    Idle,
    GoTo(Pos),
    Kill(Entity),
    Tend(Entity),
    Eat(Entity),
    /// Flee toward a destination tile (typically away from a perceived
    /// threat). Scenarios should also set `Locomotion::Running` on the
    /// fleeing actor; the goal itself is just intent.
    Flee(Pos),
}

impl Goal {
    pub fn label(&self) -> String {
        match self {
            Goal::Idle => "idle".into(),
            Goal::GoTo(p) => format!("go to ({}, {}, {})", p.x, p.y, p.z),
            Goal::Kill(e) => format!("kill #{}", e.index()),
            Goal::Tend(e) => format!("tend #{}", e.index()),
            Goal::Eat(e) => format!("eat #{}", e.index()),
            Goal::Flee(p) => format!("flee to ({}, {}, {})", p.x, p.y, p.z),
        }
    }
}

/// One concrete action. The engine knows how to execute every variant.
#[derive(Clone, Debug)]
pub enum Task {
    /// Walk to `target` via the pathfinder. Completes when the actor's
    /// `Position` matches; fails if no path exists.
    MoveTo(Pos),
    /// Attack `target`. Auto-pursues by stepping toward the target
    /// when not adjacent. Resolves a single blow per tick when
    /// adjacent. Completes when `target` dies.
    Attack(Entity),
    /// Wait for the given number of ticks. Decrements each tick;
    /// completes when the counter reaches zero.
    Wait(u32),
    /// Generic interaction. Emits `EntityUsed { user, target }` and
    /// completes immediately. Scenarios add their own systems that
    /// observe `EntityUsed` events and apply effects (advance crop
    /// growth, trigger lever, etc).
    UseEntity(Entity),
    /// Add an item entity to the actor's `Inventory`. Completes the
    /// same tick.
    PickUp(Entity),
    /// Equip an item the actor possesses (must have `Wearable`).
    Equip(Entity),
}

impl Task {
    pub fn label(&self) -> String {
        match self {
            Task::MoveTo(p) => format!("MoveTo({}, {}, {})", p.x, p.y, p.z),
            Task::Attack(e) => format!("Attack(#{})", e.index()),
            Task::Wait(t) => format!("Wait({t})"),
            Task::UseEntity(e) => format!("UseEntity(#{})", e.index()),
            Task::PickUp(e) => format!("PickUp(#{})", e.index()),
            Task::Equip(e) => format!("Equip(#{})", e.index()),
        }
    }
}

#[derive(Component, Default, Debug)]
pub struct TaskQueue(pub VecDeque<Task>);

impl TaskQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, task: Task) {
        self.0.push_back(task);
    }

    pub fn push_front(&mut self, task: Task) {
        self.0.push_front(task);
    }

    pub fn front(&self) -> Option<&Task> {
        self.0.front()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Marker: when this entity is hit by `Event::EntityAttacked`, the
/// `retaliation_system` queues `Task::Attack(attacker)` at the front
/// of its queue (one-tick lag — retaliation lands the *next* tick).
#[derive(Component, Copy, Clone, Debug)]
pub struct RetaliateOnAttack;

/// Engine task executor. Run once per tick after scenario planners.
pub fn execute_tasks(world: &mut World) {
    let actors: Vec<Entity> = {
        let mut q = world.query_filtered::<Entity, bevy_ecs::query::With<TaskQueue>>();
        q.iter(world).collect()
    };

    for actor in actors {
        let alive = world
            .get::<Health>(actor)
            .map(|h| h.is_alive())
            .unwrap_or(true);
        if !alive {
            continue;
        }

        let task = world
            .get::<TaskQueue>(actor)
            .and_then(|q| q.0.front().cloned());
        let Some(task) = task else {
            continue;
        };

        let outcome = execute_one(world, actor, task);

        match outcome {
            TaskOutcome::Continue => {}
            TaskOutcome::Complete => {
                if let Some(mut q) = world.get_mut::<TaskQueue>(actor) {
                    q.0.pop_front();
                }
            }
            TaskOutcome::Failed(reason) => {
                if let Some(mut q) = world.get_mut::<TaskQueue>(actor) {
                    q.0.pop_front();
                }
                push_event(
                    world,
                    Event::TaskFailed {
                        entity: actor,
                        reason,
                    },
                );
            }
        }
    }
}

/// Watches the current tick's `EntityAttacked` events and queues a
/// counter-attack on any target that has the `RetaliateOnAttack`
/// marker. Adds a `TaskQueue` to the target if needed.
pub fn retaliation_system(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let pairs: Vec<(Entity, Entity)> = world
        .resource::<EventLog>()
        .events_at(tick)
        .filter_map(|e| match e {
            Event::EntityAttacked {
                attacker: Some(a),
                target,
                ..
            } => Some((*a, *target)),
            _ => None,
        })
        .collect();

    for (attacker, target) in pairs {
        if world.get::<RetaliateOnAttack>(target).is_none() {
            continue;
        }
        let target_alive = world
            .get::<Health>(target)
            .map(|h| h.is_alive())
            .unwrap_or(false);
        if !target_alive {
            continue;
        }
        let mut entity_mut = world.entity_mut(target);
        if !entity_mut.contains::<TaskQueue>() {
            entity_mut.insert(TaskQueue::default());
        }
        let mut q = entity_mut.get_mut::<TaskQueue>().expect("just inserted");
        let already_queued = matches!(q.0.front(), Some(Task::Attack(t)) if *t == attacker);
        if !already_queued {
            q.0.push_front(Task::Attack(attacker));
        }
    }
}

enum TaskOutcome {
    Continue,
    Complete,
    Failed(&'static str),
}

fn execute_one(world: &mut World, actor: Entity, task: Task) -> TaskOutcome {
    match task {
        Task::MoveTo(target) => execute_move(world, actor, target),
        Task::Attack(target) => execute_attack(world, actor, target),
        Task::Wait(ticks) => execute_wait(world, actor, ticks),
        Task::UseEntity(target) => execute_use(world, actor, target),
        Task::PickUp(item) => execute_pickup(world, actor, item),
        Task::Equip(item) => execute_equip(world, actor, item),
    }
}

fn execute_move(world: &mut World, actor: Entity, target: Pos) -> TaskOutcome {
    let pos = match world.get::<Position>(actor) {
        Some(p) => p.0,
        None => return TaskOutcome::Failed("no position"),
    };
    if pos == target {
        return TaskOutcome::Complete;
    }
    let path = {
        let vw = world.resource::<VoxelWorld>();
        find_path(vw, pos, target, 4096)
    };
    let Some(path) = path else {
        return TaskOutcome::Failed("no path");
    };
    if path.len() < 2 {
        return TaskOutcome::Complete;
    }
    let next = path[1];
    if let Some(mut p) = world.get_mut::<Position>(actor) {
        p.0 = next;
    }
    push_event(
        world,
        Event::EntityMoved {
            entity: actor,
            from: pos,
            to: next,
        },
    );
    TaskOutcome::Continue
}

fn execute_attack(world: &mut World, actor: Entity, target: Entity) -> TaskOutcome {
    let target_alive = world
        .get::<Health>(target)
        .map(|h| h.is_alive())
        .unwrap_or(false);
    if !target_alive {
        return TaskOutcome::Complete;
    }
    let actor_pos = match world.get::<Position>(actor) {
        Some(p) => p.0,
        None => return TaskOutcome::Failed("no position"),
    };
    let target_pos = match world.get::<Position>(target) {
        Some(p) => p.0,
        None => return TaskOutcome::Failed("target has no position"),
    };

    if chebyshev(actor_pos, target_pos) <= 1 {
        resolve_attack(world, actor, target);
        return TaskOutcome::Continue;
    }

    let path = {
        let vw = world.resource::<VoxelWorld>();
        find_path(vw, actor_pos, target_pos, 4096)
    };
    let Some(path) = path else {
        return TaskOutcome::Failed("can't reach target");
    };
    if path.len() < 2 {
        return TaskOutcome::Continue;
    }
    let next = path[1];
    if next == target_pos {
        resolve_attack(world, actor, target);
        return TaskOutcome::Continue;
    }
    if let Some(mut p) = world.get_mut::<Position>(actor) {
        p.0 = next;
    }
    push_event(
        world,
        Event::EntityMoved {
            entity: actor,
            from: actor_pos,
            to: next,
        },
    );
    TaskOutcome::Continue
}

fn execute_wait(world: &mut World, actor: Entity, ticks: u32) -> TaskOutcome {
    if ticks <= 1 {
        return TaskOutcome::Complete;
    }
    if let Some(mut q) = world.get_mut::<TaskQueue>(actor) {
        if let Some(front) = q.0.front_mut() {
            *front = Task::Wait(ticks - 1);
        }
    }
    TaskOutcome::Continue
}

fn execute_use(world: &mut World, actor: Entity, target: Entity) -> TaskOutcome {
    push_event(
        world,
        Event::EntityUsed {
            user: actor,
            target,
        },
    );
    TaskOutcome::Complete
}

fn execute_pickup(world: &mut World, actor: Entity, item: Entity) -> TaskOutcome {
    give_item(world, actor, item);
    TaskOutcome::Complete
}

fn execute_equip(world: &mut World, actor: Entity, item: Entity) -> TaskOutcome {
    if engine_equip_item(world, actor, item).is_some() {
        TaskOutcome::Complete
    } else {
        TaskOutcome::Failed("item not wearable")
    }
}

fn chebyshev(a: Pos, b: Pos) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs()).max((a.z - b.z).abs())
}

fn push_event(world: &mut World, event: Event) {
    let tick = world.resource::<Clock>().tick;
    world.resource_mut::<EventLog>().push(tick, event);
}
