//! Family-home invasion. Two armed strangers come in through the
//! south door of a small house. Mom and Dad are in the main room with
//! a few valuables on display. The Kid is hiding in a small closet
//! at the NE corner — separated from the main room by an inner wall
//! that blocks line of sight. The kid is silent, sneaking, and never
//! moves on its own.
//!
//! Without intervention the invaders see the parents, kill them,
//! collect the valuables, and walk back out. The kid is never seen
//! and the engine's perception systems don't reveal him: line of
//! sight is blocked, his Locomotion::Sneaking emits no footstep
//! sounds, and his fear stays hidden inside the closet.
//!
//! Two REPL injections change the outcome:
//!
//! - `{"action":"Scream","source":"kid"}` — kid yells. The closest
//!   invader's hearing picks it up, switches to "investigate noise",
//!   pathfinds to the closet, sees the kid through the doorway,
//!   kills him.
//! - `{"action":"Spawn","template":"sniffer_dog","at":{...}}` — drop
//!   a tracker dog onto the porch. The kid's panic-pee coating
//!   (auto-spawned when his fear crosses the terror threshold) is
//!   strongly scented urine; the dog has Smell::keen and follows
//!   the strongest odor; pathfinds to the closet; finds the kid;
//!   bites him.
//!
//! The whole thing is emergent: no scenario code says "if kid
//! screams then invader goes to closet." The scenario only sets up
//! perception components and goal-switching planners; the engine's
//! sound/sight/smell + planner machinery does the rest.

use std::collections::HashSet;

use fortress_engine::actions::{fill_region_logged, note, spawn_creature};
use fortress_engine::library::ItemSpawnOpts;
use fortress_engine::physics::Coating;
use fortress_engine::prelude::*;
use fortress_engine::{
    decay_coatings, derive_mood, emit_combat_sounds, emit_movement_sounds, ensure_material,
    execute_tasks, fear_from_combat, footing_check, retaliation_system, spawn_humanoid_body,
    spawn_item_template, tick_needs, update_hearing, update_sight, update_smell, Clock, Event,
    EventLog, Fear, Goal, Health, Hearing, Kind, Locomotion, Mood, Perceived, Position, Pos,
    RetaliateOnAttack, Scenario, Sight, Smell, Task, TaskQueue, Voxel, VoxelWorld,
};

const HOUSE_X_MIN: i32 = 1;
const HOUSE_X_MAX: i32 = 8;
const HOUSE_Y_MIN: i32 = 1;
const HOUSE_Y_MAX: i32 = 7;
const DOOR: Pos = Pos::new(4, 7, 0); // last interior tile by the south wall
const SOUTH_WALL_Y: i32 = HOUSE_Y_MAX + 1; // 8
const OUTSIDE_DOOR: Pos = Pos::new(4, 11, 0);
const INVADER_ENTRY_1: Pos = Pos::new(3, 10, 0);
const INVADER_ENTRY_2: Pos = Pos::new(5, 10, 0);

const MOM_POS: Pos = Pos::new(3, 3, 0);
const DAD_POS: Pos = Pos::new(4, 4, 0);
const KID_POS: Pos = Pos::new(7, 2, 0); // inside the NE closet

const NECKLACE_POS: Pos = Pos::new(2, 2, 0);
const WALLET_POS: Pos = Pos::new(5, 5, 0);

const MAIN_FACTION: &str = "family";
const KID_FACTION: &str = "kid";
const INVADER_FACTION: &str = "invader";

/// Marker components for behavior.
#[derive(Component)]
pub struct Invader;
#[derive(Component)]
pub struct Family;
#[derive(Component)]
pub struct Kid;
/// Marker on items the invaders are looking to steal.
#[derive(Component)]
pub struct Valuable;
/// Marker on invaders who've reached the exit and left.
#[derive(Component)]
pub struct Departed;
/// Marker once an invader has crossed into the house — prevents
/// `check_invader_departure` from firing while they're still on the
/// approach.
#[derive(Component)]
pub struct EnteredHouse;
/// Marker on the kid once they've wet themselves — prevents the
/// scenario from spawning a second urine coating.
#[derive(Component)]
pub struct PantsWet;
/// Smell-tracker AI: the dog. Its planner walks toward the
/// strongest perceived smell, switching to combat when it sees a
/// living target.
#[derive(Component)]
pub struct Tracker;

#[derive(Default)]
pub struct FamilyHome;

impl Scenario for FamilyHome {
    fn name(&self) -> &str {
        "family home invasion"
    }

    fn setup(&mut self, world: &mut World) {
        // Materials are pulled from the library so the scenario
        // doesn't have to register them itself.
        let wood = ensure_material(world, "wood").expect("wood in library");
        let grass = ensure_material(world, "grass").expect("grass in library");

        // Outdoor lawn surrounding the house.
        fill_region_logged(
            world,
            Pos::new(-2, -2, 0),
            Pos::new(11, 11, 0),
            Voxel::floor(grass),
        );
        // Wooden floor inside the house.
        fill_region_logged(
            world,
            Pos::new(HOUSE_X_MIN, HOUSE_Y_MIN, 0),
            Pos::new(HOUSE_X_MAX, HOUSE_Y_MAX, 0),
            Voxel::floor(wood),
        );

        let wall = Voxel::wall(wood);
        // Outer perimeter walls
        {
            let mut vw = world.resource_mut::<VoxelWorld>();
            // North wall
            for x in (HOUSE_X_MIN - 1)..=(HOUSE_X_MAX + 1) {
                vw.set_voxel(Pos::new(x, HOUSE_Y_MIN - 1, 0), wall);
            }
            // South wall (with door gap at col DOOR.x)
            for x in (HOUSE_X_MIN - 1)..=(HOUSE_X_MAX + 1) {
                if x != DOOR.x {
                    vw.set_voxel(Pos::new(x, SOUTH_WALL_Y, 0), wall);
                }
            }
            // East and West walls
            for y in HOUSE_Y_MIN..=HOUSE_Y_MAX {
                vw.set_voxel(Pos::new(HOUSE_X_MIN - 1, y, 0), wall);
                vw.set_voxel(Pos::new(HOUSE_X_MAX + 1, y, 0), wall);
            }
            // NE closet inner walls — 2x2 closet at (7..=8, 1..=2)
            // with a south-facing opening at (8, 3).
            //   col 6: (6,1), (6,2)  west wall of closet
            //   row 3: (6,3), (7,3)  south wall except (8,3) opening
            vw.set_voxel(Pos::new(6, 1, 0), wall);
            vw.set_voxel(Pos::new(6, 2, 0), wall);
            vw.set_voxel(Pos::new(6, 3, 0), wall);
            vw.set_voxel(Pos::new(7, 3, 0), wall);
        }

        note(
            world,
            "A small wood-frame house. Mom and Dad are home; their kid is in the closet.",
        );

        // ─── Family members ────────────────────────────────────────
        let mom = spawn_humanoid_civilian(world, "mom", MOM_POS, MAIN_FACTION, 70);
        world
            .entity_mut(mom)
            .insert(Family)
            .insert(RetaliateOnAttack);
        equip_clothing(world, mom);

        let dad = spawn_humanoid_civilian(world, "dad", DAD_POS, MAIN_FACTION, 80);
        world
            .entity_mut(dad)
            .insert(Family)
            .insert(RetaliateOnAttack);
        equip_clothing(world, dad);

        // The kid: lower HP, sneaking by default, no retaliation.
        // Has Hearing so he can hear the parents being attacked
        // (which spikes his fear via fear_from_combat — wait, that
        // only fires when YOU are wounded. We add a scenario-side
        // bystander_fear system to spike fear for anyone who hears
        // violence. Same emergent path as office.)
        let kid = spawn_humanoid_civilian(world, "kid", KID_POS, KID_FACTION, 25);
        world
            .entity_mut(kid)
            .insert(Kid)
            .insert(Locomotion::Sneaking);
        equip_clothing(world, kid);

        // ─── Valuables on the floor ────────────────────────────────
        let necklace = spawn_item_template(
            world,
            "kitchen knife", // re-using existing template; relabel
            ItemSpawnOpts {
                at: Some(NECKLACE_POS),
                override_label: Some("gold necklace".into()),
                ..Default::default()
            },
        )
        .expect("library has 'kitchen knife'");
        world.entity_mut(necklace).insert(Valuable);

        let wallet = spawn_item_template(
            world,
            "kitchen knife",
            ItemSpawnOpts {
                at: Some(WALLET_POS),
                override_label: Some("leather wallet".into()),
                ..Default::default()
            },
        )
        .expect("library has 'kitchen knife'");
        world.entity_mut(wallet).insert(Valuable);

        // ─── Invaders ──────────────────────────────────────────────
        let invader_a = spawn_humanoid_civilian(world, "invader_a", INVADER_ENTRY_1, INVADER_FACTION, 110);
        world.entity_mut(invader_a).insert(Invader);
        let crowbar = spawn_item_template(
            world,
            "steel crowbar",
            ItemSpawnOpts {
                equip_on: Some(invader_a),
                ..Default::default()
            },
        )
        .expect("library has 'steel crowbar'");
        let _ = crowbar;

        let invader_b = spawn_humanoid_civilian(world, "invader_b", INVADER_ENTRY_2, INVADER_FACTION, 110);
        world.entity_mut(invader_b).insert(Invader);
        let knife = spawn_item_template(
            world,
            "kitchen knife",
            ItemSpawnOpts {
                equip_on: Some(invader_b),
                ..Default::default()
            },
        )
        .expect("library has 'kitchen knife'");
        let _ = knife;

        note(
            world,
            "Two strangers in dark hoodies stop at the front step. Crowbar and knife in hand.",
        );
    }

    fn build_schedule(&mut self) -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                tick_needs,
                derive_mood,
                graft_tracker_components,
                bystander_fear,
                kid_panic_pees,
                invader_planner,
                tracker_planner,
                execute_tasks,
                emit_combat_sounds,
                emit_movement_sounds,
                update_sight,
                update_hearing,
                update_smell,
                decay_coatings,
                footing_check,
                retaliation_system,
                fear_from_combat,
                check_invader_departure,
            )
                .chain(),
        );
        schedule
    }

    fn is_complete(&self, world: &mut World) -> bool {
        // Done when every invader is either dead or departed.
        let mut q = world.query_filtered::<(&Health, Option<&Departed>), With<Invader>>();
        let any_active = q
            .iter(world)
            .any(|(h, dep)| h.is_alive() && dep.is_none());
        !any_active
    }
}

// ─── helpers ───────────────────────────────────────────────────────

fn spawn_humanoid_civilian(
    world: &mut World,
    name: &str,
    pos: Pos,
    faction: &str,
    health: i32,
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
        .insert(Perceived::default());
    entity
}

fn equip_clothing(world: &mut World, wearer: Entity) {
    let _ = spawn_item_template(
        world,
        "cotton t-shirt",
        ItemSpawnOpts {
            equip_on: Some(wearer),
            ..Default::default()
        },
    );
    let _ = spawn_item_template(
        world,
        "leather boots",
        ItemSpawnOpts {
            equip_on: Some(wearer),
            ..Default::default()
        },
    );
}

// ─── per-tick systems ──────────────────────────────────────────────

/// Spike Fear when the listener hears violent sounds — the engine's
/// `fear_from_combat` only fires on direct wounds, so this is the
/// "I heard mom scream" pathway.
fn bystander_fear(world: &mut World) {
    let alerts: Vec<(Entity, f32)> = {
        let mut q = world.query::<(Entity, &Perceived, &Fear)>();
        q.iter(world)
            .filter_map(|(e, perceived, _)| {
                perceived
                    .loudest_violent()
                    .map(|hs| (e, hs.apparent_intensity))
            })
            .collect()
    };
    for (entity, intensity) in alerts {
        if let Some(mut f) = world.get_mut::<Fear>(entity) {
            f.frighten(0.15 + intensity * 0.25);
        }
    }
}

/// When the kid's fear first crosses the terror threshold, drop a
/// urine coating at his current tile. Once. The smell lingers, and
/// any creature with `Smell` will perceive it.
fn kid_panic_pees(world: &mut World) {
    let kids: Vec<(Entity, Pos, f32)> = {
        let mut q = world
            .query_filtered::<(Entity, &Position, &Fear), (With<Kid>, Without<PantsWet>)>();
        q.iter(world)
            .map(|(e, p, f)| (e, p.0, f.current))
            .collect()
    };
    for (kid, pos, fear) in kids {
        if fear < 0.8 {
            continue;
        }
        let urine = match ensure_material(world, "urine") {
            Ok(id) => id,
            Err(_) => return,
        };
        world.spawn((
            Position(pos),
            Kind("puddle of urine".into()),
            Coating {
                material: urine,
                volume: 1.0,
            },
        ));
        world.entity_mut(kid).insert(PantsWet);
        let label = label_kind(world, kid);
        let tick = world.resource::<Clock>().tick;
        world.resource_mut::<EventLog>().push(
            tick,
            Event::Note(format!(
                "{label} wets himself in fear, leaving a fresh puddle on the closet floor."
            )),
        );
    }
}

/// Invader behavior, expressed as a state machine driven by
/// perception:
///
///   1. If we can SEE a living non-invader, kill them.
///   2. Else if we recently HEARD a violent sound, walk toward it
///      to investigate.
///   3. Else if there are valuables still on the floor in the house,
///      walk to the nearest and pick it up.
///   4. Else, we're done — head for the front door and leave.
fn invader_planner(world: &mut World) {
    let invaders: Vec<(Entity, Pos, Goal, bool)> = {
        let mut q = world.query_filtered::<(Entity, &Position, &Goal, &TaskQueue), (
            With<Invader>,
            Without<Departed>,
        )>();
        q.iter(world)
            .map(|(e, p, g, q)| (e, p.0, g.clone(), q.is_empty()))
            .collect()
    };

    // Snapshot which entities are themselves invaders so we don't
    // target each other.
    let invader_ids: HashSet<Entity> = invaders.iter().map(|(e, _, _, _)| *e).collect();

    // Snapshot all valuables still sitting on the floor (have a
    // Position component). Once an invader picks one up, the
    // engine's give_item drops Position so they fall out of this
    // list naturally.
    let valuables: Vec<(Entity, Pos)> = {
        let mut q = world.query_filtered::<(Entity, &Position), With<Valuable>>();
        q.iter(world).map(|(e, p)| (e, p.0)).collect()
    };

    for (invader, pos, goal, queue_empty) in invaders {
        let alive = world
            .get::<Health>(invader)
            .map(|h| h.is_alive())
            .unwrap_or(false);
        if !alive {
            continue;
        }

        // 1. See a target?
        let target = world
            .get::<Perceived>(invader)
            .and_then(|p| {
                p.seen
                    .iter()
                    .filter(|s| !invader_ids.contains(&s.entity))
                    .filter(|s| {
                        world
                            .get::<Health>(s.entity)
                            .map(|h| h.is_alive())
                            .unwrap_or(false)
                    })
                    .min_by_key(|s| s.distance)
                    .map(|s| s.entity)
            });
        if let Some(t) = target {
            if !matches!(goal, Goal::Kill(e) if e == t) {
                if let Some(mut q) = world.get_mut::<TaskQueue>(invader) {
                    q.clear();
                    q.push(Task::Attack(t));
                }
                if let Some(mut g) = world.get_mut::<Goal>(invader) {
                    *g = Goal::Kill(t);
                }
                world.entity_mut(invader).insert(Locomotion::Running);
            }
            continue;
        }

        // 2. Heard a violent noise recently?
        let noise_origin = world
            .get::<Perceived>(invader)
            .and_then(|p| p.loudest_violent().map(|hs| hs.origin));
        if let Some(origin) = noise_origin {
            let already_going = matches!(goal, Goal::GoTo(p) if p == origin);
            if !already_going || queue_empty {
                if let Some(mut q) = world.get_mut::<TaskQueue>(invader) {
                    q.clear();
                    q.push(Task::MoveTo(origin));
                }
                if let Some(mut g) = world.get_mut::<Goal>(invader) {
                    *g = Goal::GoTo(origin);
                }
                world.entity_mut(invader).insert(Locomotion::Running);
            }
            continue;
        }

        // 3. Valuables still around?
        let target_loot = valuables
            .iter()
            .min_by_key(|(_, vp)| pos.manhattan(*vp))
            .copied();
        if let Some((item, item_pos)) = target_loot {
            let already = matches!(goal, Goal::Tend(e) if e == item);
            if !already || queue_empty {
                if let Some(mut q) = world.get_mut::<TaskQueue>(invader) {
                    q.clear();
                    q.push(Task::MoveTo(item_pos));
                    q.push(Task::PickUp(item));
                }
                if let Some(mut g) = world.get_mut::<Goal>(invader) {
                    *g = Goal::Tend(item);
                }
                world.entity_mut(invader).insert(Locomotion::Walking);
            }
            continue;
        }

        // 4. Done — leave.
        let leaving = matches!(goal, Goal::GoTo(p) if p == OUTSIDE_DOOR);
        if !leaving || queue_empty {
            if let Some(mut q) = world.get_mut::<TaskQueue>(invader) {
                q.clear();
                q.push(Task::MoveTo(OUTSIDE_DOOR));
            }
            if let Some(mut g) = world.get_mut::<Goal>(invader) {
                *g = Goal::GoTo(OUTSIDE_DOOR);
            }
            world.entity_mut(invader).insert(Locomotion::Walking);
        }
    }
}

/// Tracker (dog) behavior: smell wins over sight until the dog is
/// right on top of the source. So the dog ignores incidental
/// passers-by — even armed invaders — and pursues whatever is
/// emitting the strongest odor. Once it arrives at the source tile
/// (or adjacent), it bites whatever it can see there.
fn tracker_planner(world: &mut World) {
    let trackers: Vec<(Entity, Pos)> = {
        let mut q = world.query_filtered::<(Entity, &Position), With<Tracker>>();
        q.iter(world).map(|(e, p)| (e, p.0)).collect()
    };

    for (dog, pos) in trackers {
        let alive = world
            .get::<Health>(dog)
            .map(|h| h.is_alive())
            .unwrap_or(false);
        if !alive {
            continue;
        }

        let strongest = world
            .get::<Perceived>(dog)
            .and_then(|p| p.strongest_smell())
            .map(|s| (s.origin, s.apparent_intensity));

        if let Some((origin, _)) = strongest {
            let dist = pos.chebyshev(origin);
            if dist > 1 {
                // Still tracking — go toward the smell.
                if let Some(mut q) = world.get_mut::<TaskQueue>(dog) {
                    if !matches!(q.front(), Some(Task::MoveTo(p)) if *p == origin) {
                        q.clear();
                        q.push(Task::MoveTo(origin));
                    }
                }
                if let Some(mut g) = world.get_mut::<Goal>(dog) {
                    *g = Goal::GoTo(origin);
                }
                world.entity_mut(dog).insert(Locomotion::Walking);
                continue;
            }
            // We're at the source. Bite whatever's visible nearby.
        }

        let visible_target = world.get::<Perceived>(dog).and_then(|p| {
            p.seen
                .iter()
                .filter(|s| world.get::<Tracker>(s.entity).is_none())
                .filter(|s| {
                    world
                        .get::<Health>(s.entity)
                        .map(|h| h.is_alive())
                        .unwrap_or(false)
                })
                .min_by_key(|s| s.distance)
                .map(|s| s.entity)
        });
        if let Some(t) = visible_target {
            if let Some(mut q) = world.get_mut::<TaskQueue>(dog) {
                q.clear();
                q.push(Task::Attack(t));
            }
            if let Some(mut g) = world.get_mut::<Goal>(dog) {
                *g = Goal::Kill(t);
            }
            world.entity_mut(dog).insert(Locomotion::Running);
        }
    }
}

/// Mark invaders who reach the exit tile as Departed — but only
/// once they've actually been inside. Prevents the spawn-just-outside
/// position from triggering a false departure.
fn check_invader_departure(world: &mut World) {
    // Pass 1: tag any invader who is currently inside the house with
    // `EnteredHouse`.
    let entered: Vec<Entity> = {
        let mut q = world
            .query_filtered::<(Entity, &Position), (With<Invader>, Without<EnteredHouse>)>();
        q.iter(world)
            .filter(|(_, p)| {
                let p = p.0;
                p.x >= HOUSE_X_MIN
                    && p.x <= HOUSE_X_MAX
                    && p.y >= HOUSE_Y_MIN
                    && p.y <= HOUSE_Y_MAX
            })
            .map(|(e, _)| e)
            .collect()
    };
    for e in entered {
        world.entity_mut(e).insert(EnteredHouse);
    }

    // Pass 2: any invader at OUTSIDE_DOOR who's been inside is gone.
    let arrivals: Vec<Entity> = {
        let mut q = world.query_filtered::<
            (Entity, &Position),
            (With<Invader>, With<EnteredHouse>, Without<Departed>),
        >();
        q.iter(world)
            .filter(|(_, p)| p.0 == OUTSIDE_DOOR)
            .map(|(e, _)| e)
            .collect()
    };
    for invader in arrivals {
        world.entity_mut(invader).insert(Departed);
        let label = label_kind(world, invader);
        push_note(
            world,
            format!("{label} steps out the front door and disappears into the night."),
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

/// Hook for the dog variant: spawn a tracker at runtime via REPL
/// `{"action":"Spawn","template":"sniffer_dog",...}`. We can't add a
/// custom role to the engine's library at runtime here, so the REPL
/// uses the existing "dog" role and the scenario adds the Tracker
/// marker + Smell/Hearing/Sight via a small post-spawn system.
pub fn graft_tracker_components(world: &mut World) {
    let untagged_dogs: Vec<Entity> = {
        let mut q = world.query_filtered::<(Entity, &Kind), Without<Tracker>>();
        q.iter(world)
            .filter(|(_, k)| k.0.contains("dog"))
            .map(|(e, _)| e)
            .collect()
    };
    for dog in untagged_dogs {
        world
            .entity_mut(dog)
            .insert(Tracker)
            .insert(Smell::keen())
            .insert(Hearing::keen())
            .insert(Sight::normal())
            .insert(Perceived::default())
            .insert(TaskQueue::default())
            .insert(Goal::default());
        push_note(
            world,
            format!(
                "{} bounds onto the porch, nose to the air.",
                label_kind(world, dog)
            ),
        );
    }
}
