//! Daily routines for NPCs.
//!
//! `Schedule` is an opt-in component that lists time-keyed
//! activities ("at 06:00 go to the cafe and cook"). The
//! `tick_schedules` system runs each tick, looks up the active
//! entry by `Clock::minute_of_day`, and queues the corresponding
//! `Task` on the actor *if their queue is empty and they're not
//! already engaged in combat*. Combat / fear takes priority — a
//! teller seeing a robber drops her shift and cowers.
//!
//! The scenarios layer drives the data: bank tellers arrive at
//! 08:30, take a coffee break at 11:00, leave at 17:00. The engine
//! just walks through it.

use bevy_ecs::prelude::{Component, Entity, World};

use crate::components::Position;
use crate::tasks::{Goal, Task, TaskQueue};
use crate::time::Clock;
use crate::world::Pos;

#[derive(Component, Clone, Debug)]
pub struct Schedule {
    pub entries: Vec<ScheduleEntry>,
    /// Index of the entry that's currently being honored. When the
    /// active entry's window closes, this advances and the planner
    /// queues a new MoveTo for the next entry.
    pub active: Option<usize>,
}

impl Schedule {
    pub fn new(entries: Vec<ScheduleEntry>) -> Self {
        Self { entries, active: None }
    }
}

#[derive(Clone, Debug)]
pub struct ScheduleEntry {
    /// Inclusive start in minutes since midnight (0..1440).
    pub start_min: u32,
    /// Exclusive end in minutes since midnight.
    pub end_min: u32,
    pub activity: Activity,
}

impl ScheduleEntry {
    pub fn new(start_min: u32, end_min: u32, activity: Activity) -> Self {
        Self { start_min, end_min, activity }
    }

    pub fn covers(&self, minute: u32) -> bool {
        // Entries that wrap past midnight (e.g. 22:00 - 06:00).
        if self.start_min <= self.end_min {
            (self.start_min..self.end_min).contains(&minute)
        } else {
            minute >= self.start_min || minute < self.end_min
        }
    }
}

/// What the actor should be doing during this window.
#[derive(Clone, Debug)]
pub enum Activity {
    /// Move to a tile and stay there idling.
    Idle(Pos),
    /// Move to a tile and stay; same as Idle but flagged as "work".
    WorkAt(Pos),
    /// Move to a tile and sleep — applies StatusKind::Sleeping.
    SleepAt(Pos),
    /// Move to a tile (no holding pattern after).
    Travel(Pos),
}

impl Activity {
    pub fn destination(&self) -> Pos {
        match self {
            Activity::Idle(p) | Activity::WorkAt(p) | Activity::SleepAt(p) | Activity::Travel(p) => *p,
        }
    }
}

/// Per-tick: walk every entity with a Schedule, find the entry
/// covering the current minute_of_day, and queue a MoveTo if they
/// aren't already at the destination. Skip entities with a non-Idle
/// Goal (they're in combat / focused on a target).
pub fn tick_schedules(world: &mut World) {
    let now = world.resource::<Clock>().minute_of_day();
    type Row = (Entity, Pos, bool, Option<usize>, Vec<ScheduleEntry>);
    let actors: Vec<Row> = {
        let mut q = world
            .query::<(Entity, &Position, &TaskQueue, &Goal, &Schedule)>();
        q.iter(world)
            .map(|(e, p, q, g, s)| {
                let busy = q.0.front().is_some() || !matches!(g, Goal::Idle | Goal::GoTo(_));
                (e, p.0, busy, s.active, s.entries.clone())
            })
            .collect()
    };
    for (entity, pos, busy, active, entries) in actors {
        if entries.is_empty() {
            continue;
        }
        // Find the entry that covers `now`.
        let next_idx = entries.iter().position(|e| e.covers(now));
        // Update the recorded active entry on the schedule itself.
        if let Some(mut s) = world.get_mut::<Schedule>(entity) {
            s.active = next_idx;
        }
        let Some(idx) = next_idx else {
            continue;
        };
        // If we just changed entries OR the queue is empty, queue
        // the new MoveTo. We never override combat (busy = true).
        let entry = &entries[idx];
        let dest = entry.activity.destination();
        let entry_changed = active != Some(idx);
        if busy && !entry_changed {
            continue;
        }
        if pos == dest {
            // Already at the destination — for SleepAt apply the
            // status if not already there.
            if matches!(entry.activity, Activity::SleepAt(_)) {
                use crate::status::{apply_status, StatusKind, StatusEffects};
                let already = world
                    .get::<StatusEffects>(entity)
                    .map(|e| e.0.iter().any(|x| x.kind == StatusKind::Sleeping))
                    .unwrap_or(false);
                if !already {
                    apply_status(world, entity, StatusKind::Sleeping, 60, 1);
                }
            }
            continue;
        }
        if entry_changed {
            if let Some(mut q) = world.get_mut::<TaskQueue>(entity) {
                q.clear();
                q.push(Task::MoveTo(dest));
            }
            if let Some(mut g) = world.get_mut::<Goal>(entity) {
                *g = Goal::GoTo(dest);
            }
        } else if !busy {
            if let Some(mut q) = world.get_mut::<TaskQueue>(entity) {
                q.push(Task::MoveTo(dest));
            }
        }
    }
}
