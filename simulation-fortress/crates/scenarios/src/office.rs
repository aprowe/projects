//! Office drama: a small open-plan office with a handful of workers
//! shuttling between desks and the coffee station. Without dialogue
//! or proper schedules the "drama" is light — periodic gossip notes
//! when two workers cross paths — but the scenario does what it's
//! here to do: exercise multi-actor task planning, the Hunger / Eat
//! goal swap, and the new Library spawn surface (workers come from
//! the `civilian` role template).

use std::collections::HashSet;

use fortress_engine::actions::{fill_region_logged, note};
use fortress_engine::library::RoleSpawnOpts;
use fortress_engine::prelude::*;
use fortress_engine::{
    derive_mood, eat_on_use, emit_combat_sounds, emit_movement_sounds, emit_scream,
    execute_tasks, spawn_role_template, tick_needs, update_hearing, Clock, Edible, Energy, Event,
    EventLog, Fear, Goal, Hearing, Hunger, Kind, Locomotion, Material, Mood, Perceived, Position,
    Pos, Rng, Scenario, Task, TaskQueue, Voxel, VoxelWorld,
};

const OFFICE: &str = "office";

const ROOM_MIN: Pos = Pos::new(0, 0, 0);
const ROOM_MAX: Pos = Pos::new(11, 9, 0);
const COFFEE_POS: Pos = Pos::new(10, 1, 0);
const DESK_POSITIONS: [Pos; 4] = [
    Pos::new(2, 3, 0),
    Pos::new(5, 3, 0),
    Pos::new(2, 6, 0),
    Pos::new(5, 6, 0),
];
const WORKER_NAMES: [&str; 5] = ["alice", "bob", "carol", "dave", "eve"];

#[derive(Component)]
pub struct Worker;

/// A workspace tile. The `assigned` field tracks which worker has
/// claimed it for this work cycle (so two workers don't fight over
/// the same desk).
#[derive(Component, Default)]
pub struct Desk {
    pub assigned: Option<Entity>,
}


#[derive(Default)]
pub struct OfficeDrama;

impl Scenario for OfficeDrama {
    fn name(&self) -> &str {
        "office drama"
    }

    fn setup(&mut self, world: &mut World) {
        let carpet = {
            let mut vw = world.resource_mut::<VoxelWorld>();
            vw.register_material(Material {
                name: "carpet".into(),
                solid: false,
                density: 0.3,
                flammable: true,
                friction: 0.85,
            })
        };
        note(world, "Monday morning at the office.");
        fill_region_logged(world, ROOM_MIN, ROOM_MAX, Voxel::floor(carpet));
        note(world, "Five workers file in, mugs and laptops in tow.");

        // Coffee station along one wall — Edible::coffee bumps both
        // hunger and energy when used.
        world.spawn((
            Kind("coffee_station".into()),
            Position(COFFEE_POS),
            Edible::coffee(),
        ));

        // Four desks scattered through the room.
        for (i, pos) in DESK_POSITIONS.iter().enumerate() {
            world.spawn((
                Desk::default(),
                Kind(format!("desk_{i}")),
                Position(*pos),
            ));
        }

        // Five workers spawned via the library's `civilian` role template,
        // each labeled with their own kind so narration uses names.
        for (i, &name) in WORKER_NAMES.iter().enumerate() {
            let entity = spawn_role_template(
                world,
                "civilian",
                RoleSpawnOpts {
                    at: Pos::new(1 + i as i32, 1, 0),
                    kind_label: Some(name.to_string()),
                    faction_override: Some(OFFICE.to_string()),
                    health_override: Some(100),
                },
            )
            .expect("civilian template should be in the library");
            world
                .entity_mut(entity)
                .insert(Worker)
                .insert(Hunger {
                    current: 0.1 + (i as f32) * 0.08,
                    rate: 0.012,
                })
                .insert(Energy::new(0.004))
                .insert(Mood::default())
                .insert(Hearing::normal())
                .insert(Fear::calm())
                .insert(Perceived::default());
        }
    }

    fn build_schedule(&mut self) -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                tick_needs,
                derive_mood,
                react_to_panic,
                worker_planner,
                execute_tasks,
                emit_combat_sounds,
                emit_movement_sounds,
                update_hearing,
                eat_on_use,
                narrate_coffee,
                handle_desk_use,
                gossip_event,
            )
                .chain(),
        );
        schedule
    }

    fn is_complete(&self, _world: &mut World) -> bool {
        false
    }
}

/// React to violent sounds heard this tick. Any worker whose
/// `Perceived` log contains a Scream/Blow/Bang gets a fear spike,
/// switches goal to `Goal::Flee` toward the corner farthest from
/// the noise, runs there, and screams themselves — propagating
/// panic to anyone within hearing range.
fn react_to_panic(world: &mut World) {
    let alerts: Vec<(Entity, Pos, f32)> = {
        let mut q = world.query_filtered::<(Entity, &Position, &Perceived), With<Worker>>();
        q.iter(world)
            .filter_map(|(e, _p, perceived)| {
                perceived
                    .loudest_violent()
                    .map(|hs| (e, hs.origin, hs.apparent_intensity))
            })
            .collect()
    };

    for (worker, noise_origin, intensity) in alerts {
        let already_fleeing = matches!(world.get::<Goal>(worker), Some(Goal::Flee(_)));
        let flee_target = farthest_corner_from(noise_origin);
        if !already_fleeing {
            push_note(
                world,
                format!(
                    "{} hears violence and bolts for cover.",
                    label_kind(world, worker)
                ),
            );
        }
        world
            .entity_mut(worker)
            .insert(Goal::Flee(flee_target))
            .insert(Locomotion::Running);
        if let Some(mut q) = world.get_mut::<TaskQueue>(worker) {
            q.clear();
            q.push(Task::MoveTo(flee_target));
        }
        if let Some(mut f) = world.get_mut::<Fear>(worker) {
            let was_terrified = f.is_terrified();
            f.frighten(0.4 + intensity * 0.4);
            // Newly terrified workers shriek, propagating panic.
            if !was_terrified && f.is_terrified() {
                emit_scream(world, worker, 0.9);
            }
        }
    }
}

fn farthest_corner_from(origin: Pos) -> Pos {
    let corners = [
        Pos::new(0, 0, 0),
        Pos::new(11, 0, 0),
        Pos::new(0, 9, 0),
        Pos::new(11, 9, 0),
    ];
    *corners
        .iter()
        .max_by_key(|c| c.manhattan(origin))
        .unwrap()
}

fn worker_planner(world: &mut World) {
    // Coffee station: there's always one. We find it by Edible — in
    // a more complex office we'd filter by Kind too.
    let coffee: Option<(Entity, Pos)> = {
        let mut q = world.query_filtered::<(Entity, &Position), With<Edible>>();
        q.iter(world).map(|(e, p)| (e, p.0)).next()
    };

    // Snapshot desks and current claims.
    let desks: Vec<(Entity, Pos, Option<Entity>)> = {
        let mut q = world.query::<(Entity, &Position, &Desk)>();
        q.iter(world)
            .map(|(e, p, d)| (e, p.0, d.assigned))
            .collect()
    };

    // Snapshot every worker that needs a new plan, including their
    // current fear level so we can decide whether to keep them in
    // panic mode or let them resume working.
    type WorkerSnap = (Entity, Pos, Goal, bool, f32, f32);
    let workers: Vec<WorkerSnap> = {
        let mut q = world.query_filtered::<
            (Entity, &Position, &Goal, &TaskQueue, &Hunger, &Fear),
            With<Worker>,
        >();
        q.iter(world)
            .map(|(e, p, g, q, h, f)| (e, p.0, g.clone(), q.is_empty(), h.current, f.current))
            .collect()
    };

    let mut already_claimed: HashSet<Entity> = desks
        .iter()
        .filter_map(|(e, _, claim)| claim.map(|_| *e))
        .collect();

    for (worker, pos, goal, queue_empty, hunger, fear) in workers {
        // Still fleeing — leave the planner out of it until either
        // the worker reaches the corner *and* fear has decayed below
        // the "shaken" threshold.
        if matches!(goal, Goal::Flee(_)) {
            if fear >= 0.3 {
                continue;
            }
            // Recovered: reset locomotion + fall through to normal.
            world
                .entity_mut(worker)
                .insert(Goal::Idle)
                .insert(Locomotion::Walking);
            push_note(
                world,
                format!(
                    "{} catches their breath and warily returns to work.",
                    label_kind(world, worker)
                ),
            );
        }

        let needs_coffee = hunger >= 0.55;

        // If already heading to coffee, leave them be.
        if matches!(goal, Goal::Eat(_)) && !queue_empty {
            continue;
        }

        if needs_coffee {
            if let Some((coffee_entity, coffee_pos)) = coffee {
                if !matches!(goal, Goal::Eat(e) if e == coffee_entity) {
                    push_note(
                        world,
                        format!(
                            "{} sighs and shuffles toward the coffee station.",
                            label_kind(world, worker)
                        ),
                    );
                }
                if let Some(mut q) = world.get_mut::<TaskQueue>(worker) {
                    q.clear();
                    q.push(Task::MoveTo(coffee_pos));
                    q.push(Task::UseEntity(coffee_entity));
                }
                if let Some(mut g) = world.get_mut::<Goal>(worker) {
                    *g = Goal::Eat(coffee_entity);
                }
                continue;
            }
        }

        // Keep working at a held desk if we already have one.
        if let Goal::Tend(target) = goal {
            let still_a_desk = desks.iter().any(|(e, _, _)| *e == target);
            if still_a_desk && !queue_empty {
                continue;
            }
        }

        // Pick an unclaimed desk.
        let pick = desks
            .iter()
            .filter(|(e, _, _)| !already_claimed.contains(e))
            .min_by_key(|(_, p, _)| pos.manhattan(*p))
            .copied();

        if let Some((desk_entity, desk_pos, _)) = pick {
            already_claimed.insert(desk_entity);
            if let Some(mut q) = world.get_mut::<TaskQueue>(worker) {
                q.clear();
                q.push(Task::MoveTo(desk_pos));
                q.push(Task::UseEntity(desk_entity));
                // A few ticks of "work" before they free the desk.
                q.push(Task::Wait(4));
            }
            if let Some(mut g) = world.get_mut::<Goal>(worker) {
                *g = Goal::Tend(desk_entity);
            }
            if let Some(mut d) = world.get_mut::<Desk>(desk_entity) {
                d.assigned = Some(worker);
            }
        } else if !matches!(goal, Goal::Idle) {
            if let Some(mut q) = world.get_mut::<TaskQueue>(worker) {
                q.clear();
            }
            if let Some(mut g) = world.get_mut::<Goal>(worker) {
                *g = Goal::Idle;
            }
        }
    }
}

/// Narration only — the engine's `eat_on_use` already updated
/// hunger/energy from the `Edible::coffee()` template values.
fn narrate_coffee(world: &mut World) {
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
        if world.get::<Edible>(target).is_none() {
            continue;
        }
        push_note(
            world,
            format!(
                "{} fills their mug and takes a slow sip.",
                label_kind(world, user)
            ),
        );
    }
}

fn handle_desk_use(world: &mut World) {
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
        if world.get::<Desk>(target).is_none() {
            continue;
        }
        push_note(
            world,
            format!(
                "{} sits down and starts typing.",
                label_kind(world, user)
            ),
        );
        // Free the desk so the planner can re-claim it after the wait.
        if let Some(mut d) = world.get_mut::<Desk>(target) {
            d.assigned = None;
        }
    }
}

/// Roughly every 7th tick, pick two workers in different cells and
/// emit a gossip note. Adds texture without needing a real dialogue
/// system.
fn gossip_event(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    if tick == 0 || !tick.is_multiple_of(7) {
        return;
    }
    // Only living, non-fleeing workers gossip — terrified or dead
    // workers don't make small talk by the printer.
    let workers: Vec<(Entity, String, Pos)> = {
        let mut q = world.query_filtered::<
            (Entity, &Kind, &Position, &fortress_engine::Health, &Goal, &Fear),
            With<Worker>,
        >();
        q.iter(world)
            .filter(|(_, _, _, h, g, f)| {
                h.is_alive() && !matches!(g, Goal::Flee(_)) && !f.is_terrified()
            })
            .map(|(e, k, p, _, _, _)| (e, k.0.clone(), p.0))
            .collect()
    };
    if workers.len() < 2 {
        return;
    }
    let (i, j, pick) = {
        let n = workers.len() as u32;
        let mut rng = world.resource_mut::<Rng>();
        let i = rng.range(n) as usize;
        let mut j = rng.range(n) as usize;
        if j == i {
            j = (j + 1) % workers.len();
        }
        let pick = rng.range(5) as usize;
        (i, j, pick)
    };
    let (_, name_a, _) = &workers[i];
    let (_, name_b, _) = &workers[j];
    let lines = [
        format!("{name_a} catches {name_b}'s eye over the cubicle wall."),
        format!("{name_a} whispers something to {name_b} about the new project."),
        format!("{name_a} mutters about deadlines; {name_b} nods sympathetically."),
        format!("{name_a} and {name_b} share a quiet laugh near the printer."),
        format!("{name_a} rolls their eyes; {name_b} stifles a grin."),
    ];
    push_note(world, lines[pick].clone());
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
