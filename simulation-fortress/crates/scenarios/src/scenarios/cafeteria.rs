//! High-school cafeteria during lunch period — a schedule-driven
//! scenario that exercises `Schedule`/`Activity`, throwing
//! (`Task::Throw`), and panic-spread via sound.
//!
//! Layout (28×16):
//!
//!   ┌────────────── kitchen ──────────────┐──── dining ────┐
//!   │ stove fridge prep counter sink     ‖     four long  │
//!   │  chef + lunch ladies in here       ‖     tables     │
//!   │                                    ‖                │
//!   └─────── serving line at y=7 ────────┘────────────────┘
//!
//! Schedules:
//!   chef         06:00-13:30 work in kitchen, leaves 13:30
//!   lunch ladies 11:00-13:30 serve at the line
//!   freshmen     11:30-12:30 eat at tables
//!   seniors      11:50-12:50 eat at tables (later wave)
//!   custodian    11:00-12:00 mop, 13:00-14:00 mop again
//!
//! At 12:15, one freshman with the brittle nerves throws a plate of
//! mashed potato. The screams trigger panic via `fear_from_combat`'s
//! sound pipeline, and the planner cascades — anyone holding food
//! near the impact returns fire. Custodian flees.

use fortress_engine::actions::{fill_region_logged, note, spawn_creature};
use fortress_engine::library::{FurnitureSpawnOpts, ItemSpawnOpts};
use fortress_engine::prelude::*;
use fortress_engine::{
    decay_coatings, derive_mood, door_voxel_sync, drive_planner, emit_combat_sounds,
    emit_movement_sounds, ensure_material, execute_tasks, fear_from_combat, footing_check,
    furniture_emit_system, retaliation_system, spawn_furniture_template, spawn_humanoid_body,
    spawn_item_template, tick_carrying, tick_needs, tick_schedules, tick_status_effects,
    update_hearing, update_sight, update_smell, Activity as Act, BreakUpFight, CleanSpills,
    Clock, Drives, Event, EventLog, ExitUnfamiliar, Fear, FleeFromViolence, Goal,
    Hearing, Idle, Inventory, Kind, Messiness, Mood, Perceived,
    Position, Pos, RetaliateOnAttack, Schedule, ScheduleEntry, Scenario, Sight,
    Stats, Task, TaskQueue, Voxel, VoxelWorld, WinFoodFight,
};

const X_MIN: i32 = 1;
const X_MAX: i32 = 28;
const Y_MIN: i32 = 1;
const Y_MAX: i32 = 15;
const SERVING_Y: i32 = 7; // serving counter wall
const KITCHEN_X_MAX: i32 = 13; // kitchen is x=1..=13, dining is x=14..=28

const ENTRANCE: Pos = Pos::new(20, Y_MAX, 0);

#[derive(Component)] pub struct Student;
#[derive(Component)] pub struct Worker;
#[derive(Component)] pub struct InstigatorTag; // The freshman who throws

#[derive(Default)]
pub struct Cafeteria;

impl Scenario for Cafeteria {
    fn name(&self) -> &str { "cafeteria food fight" }

    fn setup(&mut self, world: &mut World) {
        // 11:25 AM — staff already on duty, students about to flood in.
        {
            let mut c = world.resource_mut::<Clock>();
            c.start_minute = 11 * 60 + 25;
        }

        // Floors: kitchen tile, dining linoleum.
        let tile = ensure_material(world, "tile").expect("tile");
        let lino = ensure_material(world, "linoleum").expect("linoleum");
        let drywall = ensure_material(world, "drywall").expect("drywall");
        fill_region_logged(world,
            Pos::new(X_MIN, Y_MIN, 0),
            Pos::new(KITCHEN_X_MAX, Y_MAX, 0),
            Voxel::floor(tile));
        fill_region_logged(world,
            Pos::new(KITCHEN_X_MAX + 1, Y_MIN, 0),
            Pos::new(X_MAX, Y_MAX, 0),
            Voxel::floor(lino));
        // Outer walls + serving counter (a wall along y=7 with a gap
        // for the line at x=11..13).
        let outer = Voxel::wall(drywall);
        {
            let mut vw = world.resource_mut::<VoxelWorld>();
            for x in (X_MIN - 1)..=(X_MAX + 1) {
                vw.set_voxel(Pos::new(x, Y_MIN - 1, 0), outer);
                if x != ENTRANCE.x {
                    vw.set_voxel(Pos::new(x, Y_MAX + 1, 0), outer);
                }
            }
            for y in Y_MIN..=Y_MAX {
                vw.set_voxel(Pos::new(X_MIN - 1, y, 0), outer);
                vw.set_voxel(Pos::new(X_MAX + 1, y, 0), outer);
            }
            // Serving counter: wall at y=SERVING_Y from kitchen edge
            // to the dining wall. The serving line is the gap at
            // x=11..=13 (where students queue and lunch ladies plate).
            for x in X_MIN..=KITCHEN_X_MAX {
                if !(11..=13).contains(&x) {
                    vw.set_voxel(Pos::new(x, SERVING_Y, 0), outer);
                }
            }
        }

        // Kitchen equipment.
        place(world, "refrigerator", Pos::new(2, 2, 0));
        place_powered(world, "stove on", Pos::new(2, 5, 0), "industrial stove");
        place(world, "kitchen sink", Pos::new(11, 2, 0));
        place(world, "dishwasher", Pos::new(13, 5, 0));
        place(world, "kitchen island", Pos::new(5, 4, 0));
        place(world, "ceiling fan", Pos::new(7, 3, 0));

        // Dining: four long tables with chairs.
        for (tx, ty) in [(16, 9), (16, 12), (22, 9), (22, 12)] {
            place(world, "dining table", Pos::new(tx, ty, 0));
            for (cx, cy) in [(tx - 1, ty), (tx + 1, ty), (tx + 4, ty)] {
                place(world, "dining chair", Pos::new(cx, cy, 0));
            }
        }
        place(world, "trash can", Pos::new(28, Y_MAX - 1, 0));
        place(world, "wall clock", Pos::new(20, Y_MIN, 0));
        place(world, "vending machine", Pos::new(28, 11, 0));

        note(world, "11:25 AM. The cafeteria smells of fryer oil and floor wax. The lunch ladies are setting trays on the line; the bell rings in five minutes.");

        // ─── Workers ───────────────────────────────────────────────
        // Chef in the kitchen, busy at stove until 13:30.
        let chef = humanoid(world, "chef", Pos::new(3, 5, 0), "staff", 80,
            Stats { str_: 12, dex: 13, con: 13, int: 12, wis: 13, cha: 12 });
        equip(world, chef, &["uniform shirt", "rubber boots", "kitchen knife"]);
        world.entity_mut(chef).insert(Worker).insert(Messiness::default())
            .insert(Schedule::new(vec![
                ScheduleEntry::new(6 * 60, 13 * 60 + 30, Act::WorkAt(Pos::new(3, 5, 0))),
                ScheduleEntry::new(13 * 60 + 30, 14 * 60, Act::Travel(ENTRANCE)),
            ]))
            // Chef wants to keep the kitchen clean & break up trouble
            // that strays into it; flees if it gets too crazy.
            .insert(Drives::new()
                .with(Box::new(BreakUpFight {
                    range: 6,
                    jurisdiction: Some((Pos::new(X_MIN, Y_MIN, 0),
                                        Pos::new(KITCHEN_X_MAX, Y_MAX, 0))),
                }))
                .with(Box::new(CleanSpills { range: 5 }))
                .with(Box::new(FleeFromViolence { flee_to: ENTRANCE }))
                .with(Box::new(Idle)));

        // Two lunch ladies on the serving line.
        for (i, x) in [11, 13].iter().enumerate() {
            let line_pos = Pos::new(*x, 6, 0);
            let lady = humanoid(world, &format!("lunch_lady_{i}"), line_pos, "staff", 65,
                Stats { str_: 11, dex: 11, con: 13, int: 11, wis: 12, cha: 11 });
            equip(world, lady, &["uniform shirt", "rubber boots"]);
            // Hand each one a serving ladle (kitchen knife mass; harmless).
            let _ = spawn_item_template(world, "frying pan",
                ItemSpawnOpts { equip_on: Some(lady), ..Default::default() });
            world.entity_mut(lady).insert(Worker).insert(Messiness::default())
                .insert(Schedule::new(vec![
                    ScheduleEntry::new(11 * 60, 13 * 60 + 30, Act::WorkAt(line_pos)),
                    ScheduleEntry::new(13 * 60 + 30, 14 * 60, Act::Travel(ENTRANCE)),
                ]))
                // Lunch ladies clean spills, break up fights at the
                // line, flee if it gets violent.
                .insert(Drives::new()
                    .with(Box::new(CleanSpills { range: 6 }))
                    .with(Box::new(BreakUpFight { range: 5, jurisdiction: None }))
                    .with(Box::new(FleeFromViolence { flee_to: ENTRANCE }))
                    .with(Box::new(Idle)));
        }

        // Custodian: mops dining 11:00-12:00, returns 13:00-14:00.
        let cust = humanoid(world, "custodian", Pos::new(20, 14, 0), "staff", 60,
            Stats::citizen());
        equip(world, cust, &["uniform shirt", "rubber boots"]);
        world.entity_mut(cust).insert(Worker).insert(Messiness::default())
            .insert(Schedule::new(vec![
                ScheduleEntry::new(11 * 60, 12 * 60, Act::WorkAt(Pos::new(25, 14, 0))),
                ScheduleEntry::new(12 * 60, 13 * 60, Act::Idle(ENTRANCE)),
                ScheduleEntry::new(13 * 60, 14 * 60, Act::WorkAt(Pos::new(18, 14, 0))),
            ]))
            // Custodian: cleans relentlessly, has no stomach for fighting.
            .insert(Drives::new()
                .with(Box::new(FleeFromViolence { flee_to: ENTRANCE }))
                .with(Box::new(CleanSpills { range: 8 }))
                .with(Box::new(Idle)));

        // ─── Students ──────────────────────────────────────────────
        // Freshmen come in at 11:30, eat at tables, leave 12:30.
        let freshman_seats = [
            (15, 9), (17, 9), (18, 9), // table 1
            (15, 12), (18, 12),        // table 2
        ];
        for (i, (sx, sy)) in freshman_seats.iter().enumerate() {
            let seat = Pos::new(*sx, *sy, 0);
            let f = humanoid(world, &format!("freshman_{i}"), ENTRANCE, "freshman", 55,
                Stats { str_: 9, dex: 13, con: 11, int: 11, wis: 9, cha: 11 });
            equip(world, f, &["hoodie", "rubber boots"]);
            // Each carries a tray of lunch — useful as a thrown item later.
            let _ = spawn_item_template(world, "plate of mashed potato",
                ItemSpawnOpts { give_to: Some(f), ..Default::default() });
            world.entity_mut(f).insert(Student).insert(RetaliateOnAttack)
                .insert(Messiness::default())
                .insert(Schedule::new(vec![
                    ScheduleEntry::new(11 * 60 + 30, 12 * 60 + 30, Act::Idle(seat)),
                    ScheduleEntry::new(12 * 60 + 30, 13 * 60, Act::Travel(ENTRANCE)),
                ]))
                // Freshman: throw food at seniors when chaos starts;
                // flee if it gets out of hand; head for the door
                // when bored.
                .insert(Drives::new()
                    .with(Box::new(FleeFromViolence { flee_to: ENTRANCE }))
                    .with(Box::new(WinFoodFight {
                        rival_factions: vec!["senior".into(), "freshman".into()],
                    }))
                    .with(Box::new(ExitUnfamiliar { exit_hint: ENTRANCE }))
                    .with(Box::new(Idle)));
            // Mark the first as the instigator.
            if i == 0 {
                world.entity_mut(f).insert(InstigatorTag);
            }
        }
        // Seniors arrive a little later, sit at the second pair.
        let senior_seats = [(21, 9), (24, 9), (21, 12), (25, 12)];
        for (i, (sx, sy)) in senior_seats.iter().enumerate() {
            let seat = Pos::new(*sx, *sy, 0);
            let s = humanoid(world, &format!("senior_{i}"), ENTRANCE, "senior", 65,
                Stats { str_: 12, dex: 13, con: 12, int: 11, wis: 11, cha: 12 });
            equip(world, s, &["leather jacket", "leather boots"]);
            let _ = spawn_item_template(world, "bottle of ketchup",
                ItemSpawnOpts { give_to: Some(s), ..Default::default() });
            world.entity_mut(s).insert(Student).insert(RetaliateOnAttack)
                .insert(Messiness::default())
                .insert(Schedule::new(vec![
                    ScheduleEntry::new(11 * 60 + 50, 12 * 60 + 50, Act::Idle(seat)),
                    ScheduleEntry::new(12 * 60 + 50, 13 * 60 + 20, Act::Travel(ENTRANCE)),
                ]))
                // Senior: same drives as freshmen but treats both
                // factions as rivals (food fights are tribal).
                .insert(Drives::new()
                    .with(Box::new(FleeFromViolence { flee_to: ENTRANCE }))
                    .with(Box::new(WinFoodFight {
                        rival_factions: vec!["freshman".into(), "senior".into()],
                    }))
                    .with(Box::new(ExitUnfamiliar { exit_hint: ENTRANCE }))
                    .with(Box::new(Idle)));
        }

        note(world, "Bell rings. The herd thunders down the hall.");
    }

    fn build_schedule(&mut self) -> Schedule_ {
        let mut s = Schedule_::default();
        s.add_systems((
            tick_needs, derive_mood, door_voxel_sync,
            tick_schedules,
            instigator_kicks_off,    // single-shot: instigator throws at 12:15
            drive_planner,           // utility-AI runs every actor's drives
            execute_tasks,
            tick_carrying,
            apply_throw_messiness,   // post-impact: convert throws → Messiness
            handle_door_use,
        ).chain());
        s.add_systems((
            furniture_emit_system, emit_combat_sounds, emit_movement_sounds,
            update_sight, update_hearing, update_smell,
            tick_status_effects, footing_check, retaliation_system,
            fear_from_combat, decay_coatings,
            decay_messiness,
        ).chain().after(handle_door_use));
        s
    }

    fn is_complete(&self, world: &mut World) -> bool {
        let now = world.resource::<Clock>().minute_of_day();
        if now <= 13 * 60 { return false; }
        // Bell rings — score the food fight by Messiness. Lowest
        // total wins the period; highest is the public laughingstock.
        type Row = (String, f32);
        let mut scores: Vec<Row> = {
            let mut q = world.query_filtered::<(&Kind, &Messiness), With<Student>>();
            q.iter(world).map(|(k, m)| (k.0.clone(), m.0)).collect()
        };
        if !scores.is_empty() {
            scores.sort_by(|a, b| a.1.total_cmp(&b.1));
            let cleanest = &scores[0];
            let messiest = scores.last().unwrap();
            let tick = world.resource::<Clock>().tick;
            world.resource_mut::<EventLog>().push(tick, Event::Note(format!(
                "The bell rings. The cafeteria is wreckage. {} walks out clean (mess {:.2}). {} is unrecognizable under condiment ({:.2}).",
                cleanest.0, cleanest.1, messiest.0, messiest.1,
            )));
        }
        true
    }
}

// rename to avoid clashing with our local Schedule re-export (the
// engine's `Schedule` component is the daily-routine type; the
// bevy schedule type is what we need here for systems).
type Schedule_ = bevy_ecs::schedule::Schedule;

// ─── helpers ──────────────────────────────────────────────────────

fn place(world: &mut World, template: &str, pos: Pos) {
    let _ = spawn_furniture_template(world, template,
        FurnitureSpawnOpts { at: pos, kind_label: None });
}
fn place_powered(world: &mut World, template: &str, pos: Pos, label: &str) {
    let _ = spawn_furniture_template(world, template,
        FurnitureSpawnOpts { at: pos, kind_label: Some(label.into()) });
}

fn humanoid(world: &mut World, name: &str, pos: Pos, faction: &str, health: i32, stats: Stats) -> Entity {
    let entity = spawn_creature(world, name, pos, health, Some(faction));
    spawn_humanoid_body(world, entity);
    world.entity_mut(entity)
        .insert(TaskQueue::default()).insert(Goal::default()).insert(Mood::default())
        .insert(Fear::calm()).insert(Sight::normal()).insert(Hearing::normal())
        .insert(Perceived::default()).insert(stats);
    entity
}

fn equip(world: &mut World, who: Entity, items: &[&str]) {
    for item in items {
        let _ = spawn_item_template(world, item,
            ItemSpawnOpts { equip_on: Some(who), ..Default::default() });
    }
}

// ─── single-shot: the instigator launches the fight ───────────────
// Most cafeteria behavior emerges from the engine's `drive_planner`
// running each actor's `WinFoodFight` / `BreakUpFight` / etc. drives.
// But the very FIRST throw needs a kick — without it, no chaos has
// occurred yet, so the WinFoodFight drives all score zero (the
// "sees rivals throwing" + recent-violence inputs are flat).
//
// At 12:15 we manually push a Throw task on the InstigatorTag
// freshman; once that lands and Messiness starts spreading, the
// drives pick up and the simulation runs itself.

fn instigator_kicks_off(world: &mut World) {
    let now = world.resource::<Clock>().minute_of_day();
    if now != 12 * 60 + 15 { return; }
    let instigator: Option<(Entity, Pos)> = {
        let mut q = world.query_filtered::<(Entity, &Position), With<InstigatorTag>>();
        q.iter(world).next().map(|(e, p)| (e, p.0))
    };
    let Some((inst, ipos)) = instigator else { return };
    // Pick the closest senior across the room.
    let target_pos: Option<Pos> = {
        let mut q = world.query_filtered::<(&Kind, &Position), With<Student>>();
        q.iter(world)
            .filter(|(k, _)| k.0.starts_with("senior_"))
            .map(|(_, p)| p.0)
            .min_by_key(|p| p.manhattan(ipos))
    };
    let tray: Option<Entity> = world.get::<Inventory>(inst)
        .and_then(|inv| inv.0.iter().copied().next());
    if let (Some(tray), Some(tpos)) = (tray, target_pos) {
        if let Some(mut q) = world.get_mut::<TaskQueue>(inst) {
            q.clear();
            q.push(Task::Throw(tray, tpos));
        }
        let tick = world.resource::<Clock>().tick;
        world.resource_mut::<EventLog>().push(tick, Event::Note(
            "A freshman stands up on the table and hurls a plate of mashed potato across the cafeteria.".into()
        ));
    }
    // Mark the instigator so we don't re-fire next tick.
    world.entity_mut(inst).remove::<InstigatorTag>();
}

// ─── post-throw: score Messiness on whoever got hit ───────────────
//
// The engine's `Task::Throw` already drops the projectile at the
// landing tile and emits an `EntityAttacked` event with mass-based
// HP damage. We piggyback on that event here: any creature attacked
// by a thrown food item picks up Messiness equal to the food's
// vividness factor (smell_intensity + 0.5).

fn apply_throw_messiness(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    type Hit = (Entity, Entity);
    let hits: Vec<Hit> = world.resource::<EventLog>().events_at(tick)
        .filter_map(|e| match e {
            Event::EntityAttacked { attacker: Some(a), target, .. } => Some((*a, *target)),
            _ => None,
        })
        .collect();
    for (attacker, target) in hits {
        // Did the attacker just THROW a food item this tick? Look
        // for an ItemDropped event from the same actor.
        let thrown_food: Option<f32> = world
            .resource::<EventLog>()
            .events_at(tick)
            .find_map(|e| match e {
                Event::ItemDropped { dropper, item, .. } if *dropper == attacker => {
                    let mat = world.get::<fortress_engine::ItemMaterial>(*item).map(|m| m.0)?;
                    let m = world.resource::<VoxelWorld>().material(mat)?;
                    Some(m.smell_intensity + 0.5)
                }
                _ => None,
            });
        if let Some(vividness) = thrown_food {
            if world.get::<Messiness>(target).is_none() {
                world.entity_mut(target).insert(Messiness::default());
            }
            if let Some(mut m) = world.get_mut::<Messiness>(target) {
                m.add(vividness * 0.35);
            }
        }
    }
}

// Slow per-tick drying: ketchup drips off, potato gets brushed.
fn decay_messiness(world: &mut World) {
    let mut q = world.query::<&mut Messiness>();
    for mut m in q.iter_mut(world) {
        m.0 = (m.0 - 0.005).max(0.0);
    }
}

fn handle_door_use(_world: &mut World) {}
