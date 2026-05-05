//! A small farming scenario, written entirely on top of the task
//! system. Two farmers tend a 4x4 field of crops. Each crop ticks
//! through five growth stages (seed → sprout → growing → ripe →
//! harvested) — but only when a farmer interacts with it via
//! `Task::UseEntity`. The scenario completes when every crop has
//! been harvested.

use fortress_engine::actions::{fill_region_logged, note, spawn_creature};
use fortress_engine::prelude::*;
use fortress_engine::{
    execute_tasks, spawn_humanoid_body, Clock, Event, EventLog, Goal, Kind, Material, Position,
    Pos, Scenario, Task, TaskQueue, Voxel, VoxelWorld,
};

const FARMER: &str = "farm";

const FIELD_X: std::ops::Range<i32> = 1..5;
const FIELD_Y: std::ops::Range<i32> = 1..5;
const GROUND_MIN: Pos = Pos::new(-2, -2, 0);
const GROUND_MAX: Pos = Pos::new(8, 8, 0);

const STAGE_SEED: u8 = 0;
const STAGE_SPROUT: u8 = 1;
const STAGE_GROWING: u8 = 2;
const STAGE_RIPE: u8 = 3;
const STAGE_HARVESTED: u8 = 4;

#[derive(Component)]
pub struct Farmer;

#[derive(Component)]
pub struct Crop;

#[derive(Component, Copy, Clone, Debug)]
pub struct GrowthStage(pub u8);

#[derive(Default)]
pub struct Farming;

impl Scenario for Farming {
    fn name(&self) -> &str {
        "farming"
    }

    fn setup(&mut self, world: &mut World) {
        let soil = {
            let mut vw = world.resource_mut::<VoxelWorld>();
            let soil = vw.register_material(Material {
                name: "soil".into(),
                solid: false,
                density: 1.5,
                flammable: false,
            });
            vw.register_material(Material {
                name: "wheat".into(),
                solid: false,
                density: 0.4,
                flammable: true,
            });
            soil
        };
        note(world, "A small farm at dawn. Two farmers begin their morning rounds.");
        fill_region_logged(world, GROUND_MIN, GROUND_MAX, Voxel::floor(soil));
        note(world, "A 4x4 field of newly-planted seeds covers the south side of the plot.");

        // Spawn one crop entity per field tile, all starting as seeds.
        for y in FIELD_Y {
            for x in FIELD_X {
                let pos = Pos::new(x, y, 0);
                world.spawn((
                    Crop,
                    Kind(format!("crop_{x}_{y}")),
                    Position(pos),
                    GrowthStage(STAGE_SEED),
                ));
            }
        }

        // Two farmers near the edge of the field.
        for (i, (x, y)) in [(0, 0), (6, 6)].into_iter().enumerate() {
            let entity = spawn_creature(
                world,
                format!("farmer_{i}"),
                Pos::new(x, y, 0),
                100,
                Some(FARMER),
            );
            world
                .entity_mut(entity)
                .insert(Farmer)
                .insert(TaskQueue::default())
                .insert(Goal::default());
            spawn_humanoid_body(world, entity);
        }
        note(world, "The farmers stretch and look out across the field.");
    }

    fn build_schedule(&mut self) -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(
            (farmer_planner, execute_tasks, advance_used_crops).chain(),
        );
        schedule
    }

    fn is_complete(&self, world: &mut World) -> bool {
        let mut q = world.query::<&GrowthStage>();
        q.iter(world).all(|s| s.0 == STAGE_HARVESTED)
    }
}

/// Each tick, give every idle farmer a goal of tending the closest
/// crop that still needs work (anything that isn't harvested). Skips
/// crops another farmer is already heading toward.
fn farmer_planner(world: &mut World) {
    // Snapshot all unfinished crops.
    let crops: Vec<(Entity, Pos, u8)> = {
        let mut q = world.query::<(Entity, &Position, &GrowthStage)>();
        q.iter(world)
            .filter(|(_, _, s)| s.0 != STAGE_HARVESTED)
            .map(|(e, p, s)| (e, p.0, s.0))
            .collect()
    };

    // Snapshot every farmer that needs a new plan.
    let farmers: Vec<(Entity, Pos, Goal, bool)> = {
        let mut q = world
            .query_filtered::<(Entity, &Position, &Goal, &TaskQueue), With<Farmer>>();
        q.iter(world)
            .map(|(e, p, g, q)| (e, p.0, g.clone(), q.is_empty()))
            .collect()
    };

    // Track which crops are already targeted by a farmer this tick so
    // two farmers don't head for the same plant.
    let mut claimed: std::collections::HashSet<Entity> = farmers
        .iter()
        .filter_map(|(_, _, g, _)| match g {
            Goal::Tend(e) => Some(*e),
            _ => None,
        })
        .collect();

    for (farmer, pos, goal, queue_empty) in farmers {
        // Keep the existing plan if the goal's target still isn't
        // harvested.
        if let Goal::Tend(target) = goal {
            let still_unfinished = crops.iter().any(|(e, _, _)| *e == target);
            if still_unfinished && !queue_empty {
                continue;
            }
        }

        let pick = crops
            .iter()
            .filter(|(e, _, _)| !claimed.contains(e))
            .min_by_key(|(_, p, _)| pos.manhattan(*p))
            .copied();

        if let Some((target, target_pos, _stage)) = pick {
            claimed.insert(target);
            if let Some(mut q) = world.get_mut::<TaskQueue>(farmer) {
                q.clear();
                q.push(Task::MoveTo(target_pos));
                q.push(Task::UseEntity(target));
            }
            if let Some(mut g) = world.get_mut::<Goal>(farmer) {
                *g = Goal::Tend(target);
            }
        } else if !matches!(goal, Goal::Idle) {
            if let Some(mut q) = world.get_mut::<TaskQueue>(farmer) {
                q.clear();
            }
            if let Some(mut g) = world.get_mut::<Goal>(farmer) {
                *g = Goal::Idle;
            }
            push_note(
                world,
                format!(
                    "{} surveys the rows; nothing more to tend.",
                    label_kind(world, farmer)
                ),
            );
        }
    }
}

/// React to `EntityUsed` events on crops by advancing their stage.
/// Emitted by `Task::UseEntity` in the engine executor.
fn advance_used_crops(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let used: Vec<(Entity, Entity)> = world
        .resource::<EventLog>()
        .events_at(tick)
        .filter_map(|e| match e {
            Event::EntityUsed { user, target } => Some((*user, *target)),
            _ => None,
        })
        .collect();

    for (user, target) in used {
        let Some(stage) = world.get::<GrowthStage>(target).copied() else {
            continue;
        };
        if stage.0 >= STAGE_HARVESTED {
            continue;
        }
        let new_stage = GrowthStage(stage.0 + 1);
        if let Some(mut s) = world.get_mut::<GrowthStage>(target) {
            *s = new_stage;
        }
        let verb = match new_stage.0 {
            STAGE_SPROUT => "waters the seed; a green shoot pokes through the soil",
            STAGE_GROWING => "tends the sprout; it stretches taller",
            STAGE_RIPE => "watches the plant ripen, golden and full",
            STAGE_HARVESTED => "harvests the crop, sheaves under one arm",
            _ => "tends the plant",
        };
        push_note(
            world,
            format!(
                "{} {} ({}).",
                label_kind(world, user),
                verb,
                label_kind(world, target),
            ),
        );
    }
}

fn label_kind(world: &World, entity: Entity) -> String {
    match world.get::<Kind>(entity) {
        Some(k) => format!("{}#{}", k.0, entity.index()),
        None => format!("entity#{}", entity.index()),
    }
}

fn push_note(world: &mut World, msg: impl Into<String>) {
    let tick = world.resource::<Clock>().tick;
    world
        .resource_mut::<EventLog>()
        .push(tick, Event::Note(msg.into()));
}
