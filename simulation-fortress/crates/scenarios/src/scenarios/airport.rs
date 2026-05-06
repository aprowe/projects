//! A secret-agent infiltration scenario in the Hitman vein.
//!
//! Layout (24x12, single floor, public to airside reading top-to-bottom):
//!
//!   y= 0..= 1  outside / drop-off curb
//!   y= 2..= 4  public concourse (anyone allowed)
//!   y= 5       TSA checkpoint wall, with one open lane (door #1)
//!   y= 6..= 7  airside corridor (Crew clearance required)
//!   y= 8       crew lounge wall, with a locked staff door (door #2)
//!   y= 9..=10  crew lounge (Pilot clearance required) — the real pilot
//!              is here drinking coffee
//!   y=11       gate B7 doorway leading to the jet bridge
//!
//! Six NPCs:
//!   - tsa_agent (Crew clearance gate) at the checkpoint
//!   - boarding_agent (Pilot clearance gate) at the crew door
//!   - civilian_a, civilian_b — concourse bystanders (no Observer)
//!   - real_pilot (Captain Hayes) — sits in the lounge
//!   - cleaner — wanders the concourse with a low-clearance disguise
//!
//! The agent has scripted moves in three "acts":
//!
//!   Act 1: enter as a civilian, chat with the cleaner, swap into a
//!          stolen pilot's uniform from the cleaner's cart (high-quality
//!          disguise, presented_name = "Captain Hayes")
//!   Act 2: walk up to TSA, pass small talk (disguise + good lie),
//!          then slip past the boarding agent
//!   Act 3: cross the lounge — but the *real* Captain Hayes is sitting
//!          there, and the boarding agent personally knows his face,
//!          so suspicion accumulates fast. If the agent gets to the
//!          gate first, infiltration succeeds; otherwise the alarm
//!          fires and security swarms.
//!
//! Why this scenario: it stresses information asymmetry. The TSA
//! agent doesn't know what's going on in the lounge; the boarding
//! agent personally knows pilots' faces; the real pilot doesn't even
//! see the impostor at first. Each NPC's `Knowledge` is local. The
//! scripted dialog uses `Conversation` + `DialogLine::Lie` so an AI
//! co-author can later replace the script with an LLM-driven dialog
//! while reusing the same `Suspicion`/`Observer` machinery.

use fortress_engine::actions::{fill_region_logged, note, spawn_creature};
use fortress_engine::library::ItemSpawnOpts;
use fortress_engine::prelude::*;
use fortress_engine::{
    decay_coatings, derive_mood, dialog_system, door_voxel_sync, emit_combat_sounds,
    emit_movement_sounds, ensure_material, execute_tasks, fear_from_combat, footing_check,
    observation_system, retaliation_system, spawn_humanoid_body, spawn_item_template,
    tick_needs, update_hearing, update_sight, update_smell, Alarmed, Clearance, Clock,
    Conversation, DialogLine, Disguise, Door, Event, EventLog, Fear, Goal, Health, Hearing,
    Identity, Kind, Knowledge, Locomotion, Mood, Observer, Perceived, Position, Pos, Scenario,
    Sight, Stats, Suspicion, Task, TaskQueue, Voxel, VoxelWorld,
};

const TERMINAL_X_MIN: i32 = 0;
const TERMINAL_X_MAX: i32 = 23;
const TERMINAL_Y_MIN: i32 = 0;
const TERMINAL_Y_MAX: i32 = 11;

const TSA_WALL_Y: i32 = 5;
const TSA_LANE: Pos = Pos::new(11, TSA_WALL_Y, 0);

const CREW_WALL_Y: i32 = 8;
const CREW_DOOR: Pos = Pos::new(11, CREW_WALL_Y, 0);

const GATE_B7: Pos = Pos::new(20, 11, 0);

// NPC home positions. Observers stand AT their lane so their
// short-range scan catches only people actually presenting at the
// checkpoint, not random concourse traffic.
const TSA_POS: Pos = Pos::new(11, 5, 0);
const BOARDING_AGENT_POS: Pos = Pos::new(11, 7, 0);
const REAL_PILOT_POS: Pos = Pos::new(5, 10, 0);
const CIVILIAN_A_POS: Pos = Pos::new(3, 3, 0);
const CIVILIAN_B_POS: Pos = Pos::new(8, 2, 0);
const CLEANER_POS: Pos = Pos::new(17, 3, 0);

// Spy starting position — outside the terminal at the curb.
const SPY_START: Pos = Pos::new(11, 1, 0);

const SECURITY: &str = "security";
const PUBLIC: &str = "public";
const STAFF: &str = "staff";
const SPY_FACTION: &str = "spy";

#[derive(Component)]
pub struct Spy;
#[derive(Component)]
pub struct ScriptStep(pub u32);
#[derive(Component)]
pub struct Civilian;
#[derive(Component)]
pub struct GateBeacon;
#[derive(Component)]
pub struct ReachedGate;

#[derive(Default)]
pub struct AirportInfiltration;

impl Scenario for AirportInfiltration {
    fn name(&self) -> &str {
        "airport infiltration"
    }

    fn setup(&mut self, world: &mut World) {
        let tile = ensure_material(world, "stone").expect("stone");
        let wall_mat = ensure_material(world, "stone").expect("stone");
        let _ = wall_mat;

        // Outside curb (a strip of grass at y=0)
        let grass = ensure_material(world, "grass").expect("grass");
        fill_region_logged(
            world,
            Pos::new(TERMINAL_X_MIN - 1, TERMINAL_Y_MIN - 1, 0),
            Pos::new(TERMINAL_X_MAX + 1, TERMINAL_Y_MIN, 0),
            Voxel::floor(grass),
        );
        // Tiled terminal floor
        fill_region_logged(
            world,
            Pos::new(TERMINAL_X_MIN, TERMINAL_Y_MIN + 1, 0),
            Pos::new(TERMINAL_X_MAX, TERMINAL_Y_MAX, 0),
            Voxel::floor(tile),
        );
        // Outer terminal walls + interior partitions
        let wall = Voxel::wall(tile);
        {
            let mut vw = world.resource_mut::<VoxelWorld>();
            for x in (TERMINAL_X_MIN - 1)..=(TERMINAL_X_MAX + 1) {
                vw.set_voxel(Pos::new(x, TERMINAL_Y_MAX + 1, 0), wall);
            }
            for y in (TERMINAL_Y_MIN + 1)..=TERMINAL_Y_MAX {
                vw.set_voxel(Pos::new(TERMINAL_X_MIN - 1, y, 0), wall);
                vw.set_voxel(Pos::new(TERMINAL_X_MAX + 1, y, 0), wall);
            }
            // Front facade with one open door at x=11, y=1
            for x in TERMINAL_X_MIN..=TERMINAL_X_MAX {
                if x != 11 {
                    vw.set_voxel(Pos::new(x, 1, 0), wall);
                }
            }
            // TSA checkpoint wall (y=5) with one open lane at x=11
            for x in TERMINAL_X_MIN..=TERMINAL_X_MAX {
                if x != TSA_LANE.x {
                    vw.set_voxel(Pos::new(x, TSA_WALL_Y, 0), wall);
                }
            }
            // Crew lounge wall (y=8) — staff door at x=11
            for x in TERMINAL_X_MIN..=TERMINAL_X_MAX {
                if x != CREW_DOOR.x {
                    vw.set_voxel(Pos::new(x, CREW_WALL_Y, 0), wall);
                }
            }
        }

        note(
            world,
            "International airport, 06:41. The morning rush hasn't started; the terminal is half-empty.",
        );

        // Spawn the (locked) staff door entity at the crew gate
        let _crew_door = world
            .spawn((
                Position(CREW_DOOR),
                Kind("staff door".into()),
                Door::closed(tile, "staff door"),
            ))
            .id();

        // The TSA lane is open by default (no door entity — just a gap).
        // A bag X-ray machine sits next to it as flavor.
        world.spawn((
            Position(Pos::new(10, 4, 0)),
            Kind("X-ray belt".into()),
        ));
        world.spawn((
            Position(Pos::new(12, 4, 0)),
            Kind("X-ray belt".into()),
        ));

        // Gate B7 marker (a kind tag at the destination)
        world.spawn((
            Position(GATE_B7),
            Kind("gate B7".into()),
            GateBeacon,
        ));

        // ─── NPCs ──────────────────────────────────────────────────────
        // TSA agent — knows nothing about flight crews, just guards
        // the checkpoint. Anyone with a public clearance below Crew
        // gets challenged; nobody fools them about *faces* because
        // they don't know any.
        let tsa = spawn_humanoid_role(
            world,
            "tsa_agent",
            TSA_POS,
            SECURITY,
            70,
            Stats { str_: 12, dex: 11, con: 12, int: 10, wis: 13, cha: 10 },
        );
        world
            .entity_mut(tsa)
            .insert(Identity::new("Officer Ramirez", "tsa_agent", Clearance::Crew))
            .insert(
                Observer::new(2, Clearance::Crew, "TSA checkpoint")
                    .with_guarded_y(TSA_WALL_Y),
            )
            .insert(Knowledge::new()
                .with("badge_color:pilot", "navy_blue")
                .with("badge_color:crew", "light_blue")
                .with("protocol:greet", "yes"));
        for item in ["uniform shirt", "leather boots"] {
            let _ = spawn_item_template(world, item, ItemSpawnOpts {
                equip_on: Some(tsa), ..Default::default()
            });
        }

        // Boarding agent — guards the staff door / lounge. KNOWS the
        // real Captain Hayes' face. This is the dangerous observer.
        let boarding = spawn_humanoid_role(
            world,
            "boarding_agent",
            BOARDING_AGENT_POS,
            SECURITY,
            70,
            Stats { str_: 11, dex: 12, con: 12, int: 12, wis: 14, cha: 13 },
        );
        world
            .entity_mut(boarding)
            .insert(Identity::new("Mr. Doyle", "boarding_agent", Clearance::Pilot))
            .insert(
                Observer::new(3, Clearance::Pilot, "boarding gate")
                    .with_guarded_y(CREW_WALL_Y),
            )
            .insert(Knowledge::new()
                .with("face:Captain Hayes", "balding, mid-50s, gray goatee")
                .with("face:First Officer Park", "30s, dark hair, glasses")
                .with("badge_color:pilot", "navy_blue")
                .with("manifest:gate_b7", "Captain Hayes, First Officer Park"));
        for item in ["uniform shirt", "leather boots"] {
            let _ = spawn_item_template(world, item, ItemSpawnOpts {
                equip_on: Some(boarding), ..Default::default()
            });
        }

        // Real pilot — Captain Hayes himself, drinking coffee. He's
        // not an observer (heads down in the news). But if someone
        // *speaks* to him claiming to be him, he'll react.
        let real_pilot = spawn_humanoid_role(
            world,
            "real_pilot",
            REAL_PILOT_POS,
            STAFF,
            65,
            Stats { str_: 10, dex: 10, con: 11, int: 13, wis: 12, cha: 14 },
        );
        world
            .entity_mut(real_pilot)
            .insert(Identity::new("Captain Hayes", "pilot", Clearance::Pilot))
            .insert(Disguise::new("Captain Hayes", "pilot", Clearance::Pilot, 1.0))
            .insert(Knowledge::new()
                .with("face:First Officer Park", "30s, dark hair, glasses"));
        for item in ["uniform shirt", "leather boots"] {
            let _ = spawn_item_template(world, item, ItemSpawnOpts {
                equip_on: Some(real_pilot), ..Default::default()
            });
        }

        // Civilians — bystanders. They have a tourist disguise
        // (boarding pass + ID) which is real and matches their public
        // clearance. Out of TSA range anyway.
        for (name, pos) in [("civilian_a", CIVILIAN_A_POS), ("civilian_b", CIVILIAN_B_POS)] {
            let civ = spawn_humanoid_role(
                world,
                name,
                pos,
                PUBLIC,
                40,
                Stats::child(),
            );
            world
                .entity_mut(civ)
                .insert(Civilian)
                .insert(Identity::new(
                    format!("Passenger {name}"),
                    "civilian",
                    Clearance::Public,
                ))
                .insert(Disguise::new(
                    format!("Passenger {name}"),
                    "civilian",
                    Clearance::Public,
                    1.0,
                ));
        }

        // Cleaner — concourse worker. Wears low-quality disguise
        // (real, but only Crew clearance and a janitor uniform). The
        // spy will swipe a pilot's uniform from his cart.
        let cleaner = spawn_humanoid_role(
            world,
            "cleaner",
            CLEANER_POS,
            STAFF,
            45,
            Stats { str_: 10, dex: 10, con: 11, int: 9, wis: 9, cha: 9 },
        );
        world
            .entity_mut(cleaner)
            .insert(Identity::new("Hank", "cleaner", Clearance::Crew))
            .insert(Disguise::new(
                "Hank",
                "cleaner",
                Clearance::Crew,
                1.0,
            ));
        let _ = spawn_item_template(
            world,
            "uniform shirt",
            ItemSpawnOpts {
                at: Some(Pos::new(17, 4, 0)),
                override_label: Some("pilot's uniform jacket".into()),
                ..Default::default()
            },
        );

        // ─── The spy ────────────────────────────────────────────────────
        let spy = spawn_humanoid_role(
            world,
            "spy",
            SPY_START,
            SPY_FACTION,
            80,
            Stats { str_: 13, dex: 16, con: 13, int: 14, wis: 14, cha: 16 },
        );
        world
            .entity_mut(spy)
            .insert(Spy)
            .insert(ScriptStep(0))
            .insert(Identity::new("Agent 47", "operative", Clearance::Public))
            // Begin with a paper-thin tourist disguise.
            .insert(Disguise::new("Bill Stevens", "passenger", Clearance::Public, 0.9));
        for item in ["wool shirt", "leather boots"] {
            let _ = spawn_item_template(world, item, ItemSpawnOpts {
                equip_on: Some(spy), ..Default::default()
            });
        }

        note(
            world,
            "A man in a tan jacket steps out of a taxi. He has a small leather case and a confident stride.",
        );
    }

    fn build_schedule(&mut self) -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                tick_needs,
                derive_mood,
                door_voxel_sync,
                spy_planner,
                guard_planner,
                dialog_system,
                execute_tasks,
                emit_combat_sounds,
                emit_movement_sounds,
                update_sight,
                update_hearing,
                update_smell,
                observation_system,
                handle_door_use,
                footing_check,
                retaliation_system,
                fear_from_combat,
                decay_coatings,
                check_completion,
            )
                .chain(),
        );
        schedule
    }

    fn is_complete(&self, world: &mut World) -> bool {
        // Done if the spy reached the gate, or has been killed, or
        // the alarm has been raised AND the spy is dead.
        let mut q = world.query_filtered::<(&Health, Option<&ReachedGate>), With<Spy>>();
        for (h, reached) in q.iter(world) {
            if reached.is_some() {
                return true;
            }
            if !h.is_alive() {
                return true;
            }
        }
        false
    }
}

fn spawn_humanoid_role(
    world: &mut World,
    name: &str,
    pos: Pos,
    faction: &str,
    health: i32,
    stats: Stats,
) -> Entity {
    let entity = spawn_creature(world, name, pos, health, Some(faction));
    spawn_humanoid_body(world, entity);
    world
        .entity_mut(entity)
        .insert(TaskQueue::default())
        .insert(Goal::default())
        .insert(Mood::default())
        .insert(Fear::calm())
        .insert(Sight::normal())
        .insert(Hearing::normal())
        .insert(Perceived::default())
        .insert(stats);
    entity
}

// ─── per-tick systems ──────────────────────────────────────────────

/// Drives the spy through the three-act script. Each step advances
/// when the previous task queue is empty (so the spy isn't
/// interrupted mid-stride). Steps include disguise swaps, scripted
/// conversations (lies + small talk) and movement to checkpoint
/// landmarks.
fn spy_planner(world: &mut World) {
    let spies: Vec<(Entity, Pos, u32, bool)> = {
        let mut q = world.query_filtered::<(Entity, &Position, &ScriptStep, &TaskQueue), With<Spy>>();
        q.iter(world)
            .map(|(e, p, s, q)| (e, p.0, s.0, q.is_empty()))
            .collect()
    };

    for (spy, pos, step, queue_empty) in spies {
        if !queue_empty {
            continue;
        }
        // Look up the cleaner / TSA / boarding / pilot once each step.
        let cleaner = lookup_kind(world, "cleaner");
        let tsa = lookup_kind(world, "tsa_agent");
        let boarding = lookup_kind(world, "boarding_agent");
        let real_pilot = lookup_kind(world, "real_pilot");

        match step {
            0 => {
                // Walk into the terminal.
                push_task(world, spy, Task::MoveTo(Pos::new(11, 3, 0)));
                advance_step(world, spy);
            }
            1 => {
                // Approach the cleaner and chat.
                let approach = Pos::new(CLEANER_POS.x - 1, CLEANER_POS.y, 0);
                push_task(world, spy, Task::MoveTo(approach));
                if let Some(cleaner) = cleaner {
                    let mut conv = Conversation::default();
                    conv.push(DialogLine::Speak {
                        listener: Some(cleaner),
                        text: "Morning. Long shift?".into(),
                    });
                    conv.push(DialogLine::Pause(2));
                    conv.push(DialogLine::Ask {
                        listener: cleaner,
                        question: "Mind if I borrow that jacket? Spilled coffee on mine.".into(),
                    });
                    world.entity_mut(spy).insert(conv);
                }
                advance_step(world, spy);
            }
            2 => {
                // Pick up the pilot's uniform jacket from the cart and
                // change behind a column.
                let jacket_pos = Pos::new(17, 4, 0);
                push_task(world, spy, Task::MoveTo(jacket_pos));
                if let Some(jacket) = lookup_item(world, "pilot's uniform jacket") {
                    push_task(world, spy, Task::PickUp(jacket));
                    push_task(world, spy, Task::Equip(jacket));
                }
                // Swap into the new disguise *now* (before the next step
                // brings them in front of the TSA agent).
                world.entity_mut(spy).insert(Disguise::new(
                    "Captain Hayes",
                    "pilot",
                    Clearance::Pilot,
                    0.85,
                ));
                advance_step(world, spy);
            }
            3 => {
                // Walk to TSA lane and small-talk.
                push_task(world, spy, Task::MoveTo(Pos::new(11, 4, 0)));
                if let Some(tsa) = tsa {
                    let mut conv = Conversation::default();
                    conv.push(DialogLine::Lie {
                        listener: tsa,
                        text: "Captain Hayes, gate B7. Running a little late.".into(),
                        believability: 0.85,
                    });
                    conv.push(DialogLine::Speak {
                        listener: Some(tsa),
                        text: "Coffee's worse upstairs, isn't it?".into(),
                    });
                    world.entity_mut(spy).insert(conv);
                }
                advance_step(world, spy);
            }
            4 => {
                // Through the lane, approach the staff door.
                push_task(world, spy, Task::MoveTo(Pos::new(11, 7, 0)));
                if let Some(boarding) = boarding {
                    let mut conv = Conversation::default();
                    conv.push(DialogLine::Lie {
                        listener: boarding,
                        text: "Doyle, right? Hayes — flying B7 to Geneva.".into(),
                        // Lower believability — boarding agent KNOWS Hayes'
                        // face, so the lie won't sit well.
                        believability: 0.55,
                    });
                    world.entity_mut(spy).insert(conv);
                }
                advance_step(world, spy);
            }
            5 => {
                // Push through the staff door (UseEntity).
                if let Some(door) = lookup_kind(world, "staff door") {
                    let approach = Pos::new(CREW_DOOR.x, CREW_DOOR.y - 1, 0);
                    if pos != approach {
                        push_task(world, spy, Task::MoveTo(approach));
                    }
                    push_task(world, spy, Task::UseEntity(door));
                }
                advance_step(world, spy);
            }
            6 => {
                // Cross the lounge — past the real pilot. If the real
                // pilot is still alive and the boarding agent has line
                // of sight, alarm has likely already fired by now.
                push_task(world, spy, Task::MoveTo(Pos::new(15, 10, 0)));
                if let Some(real) = real_pilot {
                    // Try to bluff Captain Hayes himself with a story
                    // about a roster swap. Very low believability.
                    let mut conv = Conversation::default();
                    conv.push(DialogLine::Lie {
                        listener: real,
                        text: "Last-minute crew swap, captain. Dispatch will call you.".into(),
                        believability: 0.30,
                    });
                    world.entity_mut(spy).insert(conv);
                }
                advance_step(world, spy);
            }
            7 => {
                // Final dash to gate B7.
                push_task(world, spy, Task::MoveTo(GATE_B7));
                advance_step(world, spy);
            }
            _ => {
                // Done — wait at the gate.
                push_task(world, spy, Task::Wait(99));
            }
        }
    }
}

/// If an observer is `Alarmed`, set them to `Goal::Kill(spy)` and
/// queue an attack so they pursue. NPCs with no Alarmed marker stay
/// in place.
fn guard_planner(world: &mut World) {
    let spy = match lookup_kind(world, "spy") {
        Some(s) => s,
        None => return,
    };
    let alarmed: Vec<Entity> = {
        let mut q = world.query_filtered::<Entity, With<Alarmed>>();
        q.iter(world).collect()
    };
    for guard in alarmed {
        let already = world
            .get::<Goal>(guard)
            .map(|g| matches!(g, Goal::Kill(t) if *t == spy))
            .unwrap_or(false);
        if already {
            continue;
        }
        if let Some(mut q) = world.get_mut::<TaskQueue>(guard) {
            q.clear();
            q.push(Task::Attack(spy));
        }
        if let Some(mut g) = world.get_mut::<Goal>(guard) {
            *g = Goal::Kill(spy);
        }
        world.entity_mut(guard).insert(Locomotion::Running);

        // Once a guard goes loud, every other Observer hears about it
        // — bump their suspicion of the spy to a high level.
        let observers: Vec<Entity> = {
            let mut q = world.query_filtered::<Entity, With<Observer>>();
            q.iter(world).collect()
        };
        for obs in observers {
            if obs == guard {
                continue;
            }
            if world.get::<Suspicion>(obs).is_none() {
                world.entity_mut(obs).insert(Suspicion::default());
            }
            if let Some(mut sus) = world.get_mut::<Suspicion>(obs) {
                sus.raise(spy, 0.9);
            }
        }
    }
}

fn handle_door_use(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let attempts: Vec<(Entity, Entity)> = world
        .resource::<EventLog>()
        .events_at(tick)
        .filter_map(|e| match e {
            Event::EntityUsed { user, target } => Some((*user, *target)),
            _ => None,
        })
        .collect();
    for (user, target) in attempts {
        if world.get::<Door>(target).is_none() {
            continue;
        }
        // Spy can open any door without a check — they have a stolen
        // badge/keycard. Real-world version would key off Disguise
        // clearance.
        if let Some(mut d) = world.get_mut::<Door>(target) {
            d.state = fortress_engine::DoorState::Open;
        }
        let actor_label = label_kind(world, user);
        world.resource_mut::<EventLog>().push(
            tick,
            Event::DoorStateChanged {
                door: target,
                new_state: "open".into(),
                cause: format!("{actor_label} swiped a stolen keycard"),
            },
        );
    }
}

fn check_completion(world: &mut World) {
    // Mark the spy as having reached the gate.
    let spy = match lookup_kind(world, "spy") {
        Some(s) => s,
        None => return,
    };
    let pos = match world.get::<Position>(spy) {
        Some(p) => p.0,
        None => return,
    };
    if pos == GATE_B7 && world.get::<ReachedGate>(spy).is_none() {
        world.entity_mut(spy).insert(ReachedGate);
        let tick = world.resource::<Clock>().tick;
        world.resource_mut::<EventLog>().push(
            tick,
            Event::Note(
                "The spy slips onto the jet bridge. Boarding closes behind him. Mission successful."
                    .into(),
            ),
        );
    }
}

// ─── helpers ────────────────────────────────────────────────────────

fn push_task(world: &mut World, actor: Entity, task: Task) {
    if let Some(mut q) = world.get_mut::<TaskQueue>(actor) {
        q.push(task);
    }
}

fn advance_step(world: &mut World, actor: Entity) {
    if let Some(mut s) = world.get_mut::<ScriptStep>(actor) {
        s.0 += 1;
    }
}

fn lookup_kind(world: &mut World, name: &str) -> Option<Entity> {
    let mut q = world.query::<(Entity, &Kind)>();
    q.iter(world)
        .find(|(_, k)| k.0 == name)
        .map(|(e, _)| e)
}

fn lookup_item(world: &mut World, label: &str) -> Option<Entity> {
    use fortress_engine::ItemName;
    let mut q = world.query::<(Entity, &ItemName)>();
    q.iter(world)
        .find(|(_, n)| n.0 == label)
        .map(|(e, _)| e)
}

fn label_kind(world: &World, entity: Entity) -> String {
    match world.get::<Kind>(entity) {
        Some(k) => format!("{}#{}", k.0, entity.index()),
        None => format!("entity#{}", entity.index()),
    }
}
