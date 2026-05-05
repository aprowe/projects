//! A two-room-plus-foyer mansion home invasion. Bigger than the
//! cabin scenarios, with:
//!
//! - 16x12 voxel footprint, four rooms separated by inner walls,
//!   each with a door
//! - Doors as entities with state (Closed, Locked, Open, Broken).
//!   Opening a closed door is a manipulation_check (DEX + Grasp).
//!   Locked doors can be picked or forced (strength_check) — and a
//!   creature with crushed hands can do neither.
//! - Furniture as inert positioned entities for narration + map glyphs
//! - 4 family members (father, mother, teen, child) and 3 invaders
//!   (a brute and two burglars) with role-based stats
//! - D&D combat: attack roll vs AC, damage dice, crits, misses
//! - Valuables behind a locked dresser drawer
//!
//! All RNG flows through the seeded engine `Rng`.

use std::collections::HashSet;

use fortress_engine::actions::{fill_region_logged, note, spawn_creature};
use fortress_engine::library::ItemSpawnOpts;
use fortress_engine::prelude::*;
use fortress_engine::{
    decay_coatings, derive_mood, door_voxel_sync, emit_combat_sounds, emit_movement_sounds,
    ensure_material, execute_tasks, fear_from_combat, find_path, footing_check,
    manipulation_check, retaliation_system, spawn_humanoid_body, spawn_item_template, strength_check,
    tick_needs, update_hearing, update_sight, update_smell, CheckOutcome, Clock, Door, DoorState,
    Event, EventLog, Fear, Goal, Health, Hearing, Kind, Locomotion, Mood, Perceived, Position,
    Pos, RetaliateOnAttack, Scenario, Sight, Stats, Task, TaskQueue, Voxel, VoxelWorld,
};

const HOUSE_X_MIN: i32 = 1;
const HOUSE_X_MAX: i32 = 14;
const HOUSE_Y_MIN: i32 = 1;
const HOUSE_Y_MAX: i32 = 10;
const SOUTH_WALL_Y: i32 = HOUSE_Y_MAX + 1; // 11
const FRONT_DOOR: Pos = Pos::new(7, SOUTH_WALL_Y, 0);
const OUTSIDE_DOOR: Pos = Pos::new(7, 14, 0);

// Room layout (interior, between perimeter walls):
//
//   x=1..=14, y=1..=10
//   inner vertical wall at x=7, y=1..=5  — splits north into two
//   inner horizontal wall at y=6, x=1..=14 — splits N from S floor
//   doors:
//     (3, 6)  living-room → master-bedroom
//     (10, 6) living-room → kid's-room
//     (7, 6)  living-room → hallway upstairs (placeholder, not used)
//     (7, 11) front door (south, into living)
//
// The west bedroom holds the parents and the safe (locked dresser).
// The east bedroom is the kid's. The southern living/kitchen area is
// where invaders enter and where most combat starts.

const NW_INNER_WALL_X: i32 = 7;
const INNER_DIVIDER_Y: i32 = 6;
const MASTER_DOOR: Pos = Pos::new(3, INNER_DIVIDER_Y, 0);
const KID_DOOR: Pos = Pos::new(11, INNER_DIVIDER_Y, 0);
const SAFE_POS: Pos = Pos::new(2, 2, 0); // dresser in master bedroom
const TV_POS: Pos = Pos::new(7, 9, 0);
const SOFA_POS: Pos = Pos::new(4, 8, 0);
const TABLE_POS: Pos = Pos::new(10, 8, 0);
const FRIDGE_POS: Pos = Pos::new(13, 9, 0);
const FIREPLACE_POS: Pos = Pos::new(13, 7, 0);

// Spawn positions
const FATHER_POS: Pos = Pos::new(5, 9, 0); // living room
const MOTHER_POS: Pos = Pos::new(3, 3, 0); // master bedroom
const TEEN_POS: Pos = Pos::new(10, 3, 0); // kid's room
const CHILD_POS: Pos = Pos::new(12, 4, 0); // kid's room
const BRUTE_START: Pos = Pos::new(6, 13, 0);
const BURGLAR1_START: Pos = Pos::new(8, 13, 0);
const BURGLAR2_START: Pos = Pos::new(7, 13, 0);

const FAMILY: &str = "family";
const INVADER: &str = "invader";

#[derive(Component)]
pub struct Family;
#[derive(Component)]
pub struct Invader;
#[derive(Component)]
pub struct Departed;
#[derive(Component)]
pub struct EnteredHouse;
#[derive(Component)]
pub struct Furniture;
#[derive(Component)]
pub struct Valuable;

#[derive(Default)]
pub struct MansionInvasion;

impl Scenario for MansionInvasion {
    fn name(&self) -> &str {
        "mansion home invasion"
    }

    fn setup(&mut self, world: &mut World) {
        let wood = ensure_material(world, "wood").expect("wood");
        let grass = ensure_material(world, "grass").expect("grass");

        // Outdoor lawn
        fill_region_logged(world, Pos::new(-1, -1, 0), Pos::new(16, 15, 0), Voxel::floor(grass));
        // Wooden floor inside
        fill_region_logged(
            world,
            Pos::new(HOUSE_X_MIN, HOUSE_Y_MIN, 0),
            Pos::new(HOUSE_X_MAX, HOUSE_Y_MAX, 0),
            Voxel::floor(wood),
        );

        // Outer walls
        let wall = Voxel::wall(wood);
        {
            let mut vw = world.resource_mut::<VoxelWorld>();
            for x in (HOUSE_X_MIN - 1)..=(HOUSE_X_MAX + 1) {
                vw.set_voxel(Pos::new(x, HOUSE_Y_MIN - 1, 0), wall);
                if x != FRONT_DOOR.x {
                    vw.set_voxel(Pos::new(x, SOUTH_WALL_Y, 0), wall);
                }
            }
            for y in HOUSE_Y_MIN..=HOUSE_Y_MAX {
                vw.set_voxel(Pos::new(HOUSE_X_MIN - 1, y, 0), wall);
                vw.set_voxel(Pos::new(HOUSE_X_MAX + 1, y, 0), wall);
            }

            // Inner divider between bedrooms (y < INNER_DIVIDER_Y)
            // and living/kitchen (y > INNER_DIVIDER_Y).
            for x in HOUSE_X_MIN..=HOUSE_X_MAX {
                if x != MASTER_DOOR.x && x != KID_DOOR.x {
                    vw.set_voxel(Pos::new(x, INNER_DIVIDER_Y, 0), wall);
                }
            }

            // Inner vertical wall splitting the two bedrooms
            for y in HOUSE_Y_MIN..=(INNER_DIVIDER_Y - 1) {
                vw.set_voxel(Pos::new(NW_INNER_WALL_X, y, 0), wall);
            }
        }

        note(world, "An old two-bedroom mansion. The lights are on, dinner just ended.");

        // Doors
        let front_door = world
            .spawn((
                Position(FRONT_DOOR),
                Kind("front door".into()),
                Door::closed(wood, "front door"),
            ))
            .id();
        let _ = front_door;

        let master_door = world
            .spawn((
                Position(MASTER_DOOR),
                Kind("master bedroom door".into()),
                Door::closed(wood, "master bedroom door"),
            ))
            .id();
        let _ = master_door;

        let kid_door = world
            .spawn((
                Position(KID_DOOR),
                Kind("kid's bedroom door".into()),
                Door::locked(wood, "kid's bedroom door", 14),
            ))
            .id();
        let _ = kid_door;

        // Furniture
        for (pos, label) in [
            (TV_POS, "tv set"),
            (SOFA_POS, "sofa"),
            (TABLE_POS, "dining table"),
            (FRIDGE_POS, "refrigerator"),
            (FIREPLACE_POS, "fireplace"),
        ] {
            world.spawn((Position(pos), Kind(label.into()), Furniture));
        }

        // Valuables — a dresser in the master bedroom holding jewelry
        world.spawn((
            Position(SAFE_POS),
            Kind("locked dresser".into()),
            Furniture,
        ));
        let necklace = spawn_item_template(
            world,
            "kitchen knife",
            ItemSpawnOpts {
                at: Some(SAFE_POS),
                override_label: Some("emerald necklace".into()),
                ..Default::default()
            },
        )
        .expect("library has 'kitchen knife'");
        world.entity_mut(necklace).insert(Valuable);

        let watch = spawn_item_template(
            world,
            "kitchen knife",
            ItemSpawnOpts {
                at: Some(Pos::new(13, 8, 0)),
                override_label: Some("gold pocket watch".into()),
                ..Default::default()
            },
        )
        .expect("library has 'kitchen knife'");
        world.entity_mut(watch).insert(Valuable);

        // ─── Family ────────────────────────────────────────────────
        let father =
            spawn_humanoid_role(world, "father", FATHER_POS, FAMILY, 100, Stats::brute());
        world.entity_mut(father).insert(Family).insert(RetaliateOnAttack);
        let _ = spawn_item_template(
            world,
            "fire poker",
            ItemSpawnOpts { equip_on: Some(father), ..Default::default() },
        );
        let _ = spawn_item_template(
            world,
            "leather jacket",
            ItemSpawnOpts { equip_on: Some(father), ..Default::default() },
        );
        let _ = spawn_item_template(
            world,
            "leather boots",
            ItemSpawnOpts { equip_on: Some(father), ..Default::default() },
        );

        let mother = spawn_humanoid_role(
            world,
            "mother",
            MOTHER_POS,
            FAMILY,
            80,
            Stats { str_: 11, dex: 13, con: 12, int: 13, wis: 13, cha: 12 },
        );
        world.entity_mut(mother).insert(Family).insert(RetaliateOnAttack);
        for item in ["wool shirt", "leather boots", "kitchen knife"] {
            let _ = spawn_item_template(
                world,
                item,
                ItemSpawnOpts { equip_on: Some(mother), ..Default::default() },
            );
        }

        let teen = spawn_humanoid_role(
            world,
            "teen",
            TEEN_POS,
            FAMILY,
            60,
            Stats { str_: 10, dex: 14, con: 11, int: 12, wis: 9, cha: 12 },
        );
        world.entity_mut(teen).insert(Family).insert(RetaliateOnAttack);
        for item in ["hoodie", "rubber boots", "baseball bat"] {
            let _ = spawn_item_template(
                world,
                item,
                ItemSpawnOpts { equip_on: Some(teen), ..Default::default() },
            );
        }

        let child = spawn_humanoid_role(world, "child", CHILD_POS, FAMILY, 30, Stats::child());
        world.entity_mut(child).insert(Family).insert(Locomotion::Sneaking);
        for item in ["cotton t-shirt", "rubber boots"] {
            let _ = spawn_item_template(
                world,
                item,
                ItemSpawnOpts { equip_on: Some(child), ..Default::default() },
            );
        }

        // ─── Invaders ──────────────────────────────────────────────
        let brute = spawn_humanoid_role(world, "brute", BRUTE_START, INVADER, 130, Stats::brute());
        world.entity_mut(brute).insert(Invader);
        for item in ["leather jacket", "leather boots", "steel crowbar"] {
            let _ = spawn_item_template(
                world,
                item,
                ItemSpawnOpts { equip_on: Some(brute), ..Default::default() },
            );
        }

        let burglar_a =
            spawn_humanoid_role(world, "burglar_a", BURGLAR1_START, INVADER, 90, Stats::rogue());
        world.entity_mut(burglar_a).insert(Invader);
        for item in ["hoodie", "rubber boots", "hunting knife"] {
            let _ = spawn_item_template(
                world,
                item,
                ItemSpawnOpts { equip_on: Some(burglar_a), ..Default::default() },
            );
        }

        let burglar_b = spawn_humanoid_role(
            world,
            "burglar_b",
            BURGLAR2_START,
            INVADER,
            85,
            Stats::rogue(),
        );
        world.entity_mut(burglar_b).insert(Invader);
        for item in ["hoodie", "rubber boots", "brass candlestick"] {
            let _ = spawn_item_template(
                world,
                item,
                ItemSpawnOpts { equip_on: Some(burglar_b), ..Default::default() },
            );
        }

        note(
            world,
            "Three figures slip through the front gate. The brute carries a crowbar; the burglars hold a hunting knife and a brass candlestick.",
        );
    }

    fn build_schedule(&mut self) -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                tick_needs,
                derive_mood,
                door_voxel_sync,
                bystander_fear,
                family_planner,
                invader_planner,
                execute_tasks,
                emit_combat_sounds,
                emit_movement_sounds,
                update_sight,
                update_hearing,
                update_smell,
                handle_door_use,
                footing_check,
                retaliation_system,
                fear_from_combat,
                decay_coatings,
                check_invader_departure,
            )
                .chain(),
        );
        schedule
    }

    fn is_complete(&self, world: &mut World) -> bool {
        let mut q = world.query_filtered::<(&Health, Option<&Departed>), With<Invader>>();
        let any_active = q
            .iter(world)
            .any(|(h, dep)| h.is_alive() && dep.is_none());
        !any_active
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

fn bystander_fear(world: &mut World) {
    let alerts: Vec<(Entity, f32)> = {
        let mut q = world.query::<(Entity, &Perceived, &Fear)>();
        q.iter(world)
            .filter_map(|(e, p, _)| {
                p.loudest_violent()
                    .map(|hs| (e, hs.apparent_intensity))
            })
            .collect()
    };
    for (entity, intensity) in alerts {
        if let Some(mut f) = world.get_mut::<Fear>(entity) {
            f.frighten(0.10 + intensity * 0.20);
        }
    }
}

fn family_planner(world: &mut World) {
    // Simple: every family member with RetaliateOnAttack already gets
    // queued attacks via the engine's retaliation_system. Here we just
    // make sure they're not idle indefinitely once they've been hit —
    // already handled. So this planner is mostly a no-op but reserved
    // for future "kid hides" / "father charges" logic.
    let _ = world;
}

fn invader_planner(world: &mut World) {
    let invader_ids: HashSet<Entity> = {
        let mut q = world
            .query_filtered::<Entity, (With<Invader>, Without<Departed>)>();
        q.iter(world).collect()
    };
    let invaders: Vec<(Entity, Pos, Goal, bool)> = {
        let mut q = world.query_filtered::<(Entity, &Position, &Goal, &TaskQueue), (
            With<Invader>,
            Without<Departed>,
        )>();
        q.iter(world)
            .map(|(e, p, g, q)| (e, p.0, g.clone(), q.is_empty()))
            .collect()
    };

    let valuables: Vec<(Entity, Pos)> = {
        let mut q = world.query_filtered::<(Entity, &Position), With<Valuable>>();
        q.iter(world).map(|(e, p)| (e, p.0)).collect()
    };

    // Closed/locked doors block pathfinding via voxel sync; if an
    // invader can't reach a target, switch to "interact with the door"
    // (UseEntity on the door at the boundary).
    let closed_doors: Vec<(Entity, Pos)> = {
        let mut q = world.query::<(Entity, &Position, &Door)>();
        q.iter(world)
            .filter(|(_, _, d)| !d.state.is_passable())
            .map(|(e, p, _)| (e, p.0))
            .collect()
    };

    for (invader, pos, goal, queue_empty) in invaders {
        let alive = world
            .get::<Health>(invader)
            .map(|h| h.is_alive())
            .unwrap_or(false);
        if !alive {
            continue;
        }

        // Step 1: see a non-invader living target?
        let visible_target = world.get::<Perceived>(invader).and_then(|p| {
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
        if let Some(t) = visible_target {
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

        // Step 2: heard a violent sound, investigate
        let noise = world
            .get::<Perceived>(invader)
            .and_then(|p| p.loudest_violent().map(|hs| hs.origin));
        if let Some(origin) = noise {
            let already = matches!(goal, Goal::GoTo(p) if p == origin);
            if !already || queue_empty {
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

        // Step 3: pick a primary destination — nearest valuable, or
        // the front door if everything's looted.
        let target_loot = valuables
            .iter()
            .min_by_key(|(_, vp)| pos.manhattan(*vp))
            .copied();

        let primary_dest = target_loot
            .map(|(_, p)| p)
            .unwrap_or(OUTSIDE_DOOR);

        // Step 3a: can we reach the destination right now?
        let reachable = {
            let vw = world.resource::<VoxelWorld>();
            find_path(vw, pos, primary_dest, 2048).is_some()
        };

        if !reachable {
            // Find the closest closed door we COULD currently reach
            // — that's likely the gateway. If none reachable, just
            // pick the closest by manhattan and walk toward it.
            let door_choice = closed_doors
                .iter()
                .copied()
                .filter_map(|(door, door_pos)| {
                    let vw = world.resource::<VoxelWorld>();
                    // Try to pathfind to a tile adjacent to the door
                    // (the door itself is a wall while closed).
                    let approach = approach_tile(pos, door_pos);
                    find_path(vw, pos, approach, 2048)
                        .map(|_| (door, door_pos, pos.manhattan(door_pos)))
                })
                .min_by_key(|(_, _, d)| *d);
            if let Some((door_entity, door_pos, _)) = door_choice {
                let approach = approach_tile(pos, door_pos);
                if let Some(mut q) = world.get_mut::<TaskQueue>(invader) {
                    q.clear();
                    if pos != approach {
                        q.push(Task::MoveTo(approach));
                    }
                    q.push(Task::UseEntity(door_entity));
                }
                if let Some(mut g) = world.get_mut::<Goal>(invader) {
                    *g = Goal::Tend(door_entity);
                }
                world.entity_mut(invader).insert(Locomotion::Walking);
                continue;
            }
        }

        // Step 4: walk to the target loot, or leave.
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

        // Step 5: nothing left — leave.
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

/// Find a walkable tile next to `door_pos` from the side `from` is on.
/// Used when the door's own tile is impassable (closed/locked).
fn approach_tile(from: Pos, door_pos: Pos) -> Pos {
    let dy = (from.y - door_pos.y).signum();
    let dx = (from.x - door_pos.x).signum();
    if dy != 0 {
        Pos::new(door_pos.x, door_pos.y + dy, door_pos.z)
    } else if dx != 0 {
        Pos::new(door_pos.x + dx, door_pos.y, door_pos.z)
    } else {
        // standing on the door tile (shouldn't happen) — pick a side
        Pos::new(door_pos.x, door_pos.y + 1, door_pos.z)
    }
}

/// React to UseEntity events targeting Doors. Run a manipulation_check
/// to open closed doors and a strength_check to force locked ones.
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
        let door_data = world
            .get::<Door>(target)
            .map(|d| (d.state, d.lock_dc, d.break_dc, d.label.clone()));
        let (state, lock_dc, break_dc, label) = match door_data {
            Some(d) => d,
            None => continue,
        };

        match state {
            DoorState::Open => {
                let actor_label = label_kind(world, user);
                world.resource_mut::<EventLog>().push(
                    tick,
                    Event::Note(format!(
                        "{actor_label}'s hand brushes the {label} but it's already open."
                    )),
                );
            }
            DoorState::Closed => {
                let outcome = manipulation_check(world, user, 5);
                let (success, roll, impossible) = unpack_check(&outcome);
                world.resource_mut::<EventLog>().push(
                    tick,
                    Event::AbilityCheck {
                        actor: user,
                        kind: "open".into(),
                        target: format!("the {label}"),
                        roll,
                        dc: 5,
                        success,
                        impossible,
                    },
                );
                if success {
                    if let Some(mut d) = world.get_mut::<Door>(target) {
                        d.state = DoorState::Open;
                    }
                    let actor_label = label_kind(world, user);
                    world.resource_mut::<EventLog>().push(
                        tick,
                        Event::DoorStateChanged {
                            door: target,
                            new_state: "open".into(),
                            cause: format!("{actor_label} turned the knob"),
                        },
                    );
                }
            }
            DoorState::Locked => {
                // Try to force it: strength check vs break_dc.
                let outcome = strength_check(world, user, break_dc);
                let (success, roll, impossible) = unpack_check(&outcome);
                world.resource_mut::<EventLog>().push(
                    tick,
                    Event::AbilityCheck {
                        actor: user,
                        kind: "force open".into(),
                        target: format!("the {label}"),
                        roll,
                        dc: break_dc,
                        success,
                        impossible,
                    },
                );
                if success {
                    if let Some(mut d) = world.get_mut::<Door>(target) {
                        d.state = DoorState::Broken;
                    }
                    let actor_label = label_kind(world, user);
                    world.resource_mut::<EventLog>().push(
                        tick,
                        Event::DoorStateChanged {
                            door: target,
                            new_state: "broken".into(),
                            cause: format!("{actor_label} kicked it in"),
                        },
                    );
                } else {
                    let _ = lock_dc; // could fall through to lockpick later
                }
            }
            DoorState::Broken => {
                // already broken; no-op
            }
        }
    }
}

fn unpack_check(o: &CheckOutcome) -> (bool, i32, bool) {
    match o {
        CheckOutcome::Success(r) => (true, r.total, false),
        CheckOutcome::Failure(r) => (false, r.total, false),
        CheckOutcome::Impossible => (false, 0, true),
    }
}

fn check_invader_departure(world: &mut World) {
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
        let tick = world.resource::<Clock>().tick;
        world.resource_mut::<EventLog>().push(
            tick,
            Event::Note(format!("{label} steps out the front door and disappears.")),
        );
    }
}

fn label_kind(world: &World, entity: Entity) -> String {
    match world.get::<Kind>(entity) {
        Some(k) => format!("{}#{}", k.0, entity.index()),
        None => format!("entity#{}", entity.index()),
    }
}
