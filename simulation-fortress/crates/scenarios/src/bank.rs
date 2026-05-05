//! Daylight bank heist.
//!
//! 40×30 first-floor branch with a public lobby, a counter line of
//! teller windows, a back office, a vault room, and a back exit
//! into a parking lot with the getaway van waiting. Two robbers
//! enter at 09:42, point pistols, force the manager to open the
//! vault, grab the cash, and run for the van. A teller hits the
//! silent alarm — a `Powered` blue strobe goes on outside as a
//! visual cue — and a security guard plus two responding officers
//! show up after a delay.
//!
//! This scenario exercises:
//!
//! - Ranged combat via `Task::Shoot` (the pistols and the responding
//!   officers' shotguns).
//! - Currency: `stack of bills` + `$100 bill`s in `cash register`s
//!   and the `bank vault`. The robbers' planner targets by `Value`.
//! - Alarms: `Alarmed` triggered by the teller seeing a drawn weapon.
//! - Time-of-day: the heist runs in mid-morning so sight is full.

use fortress_engine::actions::{fill_region_logged, note, spawn_creature};
use fortress_engine::library::{FurnitureSpawnOpts, ItemSpawnOpts};
use fortress_engine::prelude::*;
use fortress_engine::{
    decay_coatings, derive_mood, dialog_system, door_voxel_sync, emit_combat_sounds,
    emit_movement_sounds, ensure_material, execute_tasks, fear_from_combat, find_path,
    footing_check, furniture_emit_system, observation_system, retaliation_system,
    spawn_furniture_template, spawn_humanoid_body, spawn_item_template, tick_needs,
    tick_status_effects, update_hearing, update_sight, update_smell, Alarmed, Clearance,
    Clock, Door, DoorState, Event, EventLog, Fear, Goal, Health, Hearing, Identity, Kind,
    Knowledge, Locomotion, Mood, Observer, Perceived, Position, Pos, RangedWeapon,
    RetaliateOnAttack, Scenario, Sight, Stats, Suspicion, Task, TaskQueue, Voxel, VoxelWorld,
};

const X_MIN: i32 = 2;
const X_MAX: i32 = 38;
const Y_MIN: i32 = 2;
const Y_MAX: i32 = 24;
const FRONT_DOOR: Pos = Pos::new(20, Y_MIN - 1, 0);
const BACK_DOOR: Pos = Pos::new(20, Y_MAX + 1, 0);
const VAULT_DOOR: Pos = Pos::new(8, 14, 0);
const COUNTER_Y: i32 = 9;            // teller line
const VAULT_X: i32 = 4;
const VAULT_Y: i32 = 16;
const GETAWAY_VAN: Pos = Pos::new(20, 27, 0);

const ROBBER_FACTION: &str = "robber";
const STAFF: &str = "staff";
const POLICE: &str = "police";
const PUBLIC: &str = "public";

#[derive(Component)] pub struct Robber;
#[derive(Component)] pub struct BankStaff;
#[derive(Component)] pub struct Police;
#[derive(Component)] pub struct Civilian;
#[derive(Component)] pub struct VaultLoot;
#[derive(Component)] pub struct Departed;
#[derive(Component)] pub struct PoliceArrived;

#[derive(Default)]
pub struct BankHeist;

impl Scenario for BankHeist {
    fn name(&self) -> &str {
        "bank heist"
    }

    fn setup(&mut self, world: &mut World) {
        // 09:42, mid-morning rush.
        {
            let mut clock = world.resource_mut::<Clock>();
            clock.start_minute = 9 * 60 + 42;
        }

        let asphalt = ensure_material(world, "asphalt").expect("asphalt");
        let concrete = ensure_material(world, "concrete").expect("concrete");
        let marble = ensure_material(world, "marble").expect("marble");
        let tile = ensure_material(world, "tile").expect("tile");
        let granite = ensure_material(world, "granite").expect("granite");
        let oak = ensure_material(world, "oak").expect("oak");
        let drywall = ensure_material(world, "drywall").expect("drywall");
        let _ = oak;

        // Outside: parking lot in asphalt, sidewalk in concrete.
        fill_region_logged(world,
            Pos::new(X_MIN - 2, Y_MAX, 0),
            Pos::new(X_MAX + 2, Y_MAX + 6, 0),
            Voxel::floor(asphalt));
        fill_region_logged(world,
            Pos::new(X_MIN - 2, Y_MIN - 2, 0),
            Pos::new(X_MAX + 2, Y_MIN - 1, 0),
            Voxel::floor(concrete));

        // Lobby: marble. Tellers behind a granite counter at y=9.
        fill_region_logged(world,
            Pos::new(X_MIN, Y_MIN, 0),
            Pos::new(X_MAX, COUNTER_Y - 1, 0),
            Voxel::floor(marble));
        // Teller area: tile.
        fill_region_logged(world,
            Pos::new(X_MIN, COUNTER_Y, 0),
            Pos::new(X_MAX, 13, 0),
            Voxel::floor(tile));
        // Back office: oak floor (call it hardwood).
        fill_region_logged(world,
            Pos::new(X_MIN, 14, 0),
            Pos::new(X_MAX, Y_MAX, 0),
            Voxel::floor(tile));

        // Outer walls + perimeter.
        let brick = ensure_material(world, "brick").expect("brick");
        let outer = Voxel::wall(brick);
        let inner = Voxel::wall(drywall);
        let door_set: std::collections::HashSet<Pos> =
            [FRONT_DOOR, BACK_DOOR, VAULT_DOOR].into_iter().collect();
        {
            let mut vw = world.resource_mut::<VoxelWorld>();
            for x in (X_MIN - 1)..=(X_MAX + 1) {
                let p = Pos::new(x, Y_MIN - 1, 0);
                if !door_set.contains(&p) { vw.set_voxel(p, outer); }
                let p = Pos::new(x, Y_MAX + 1, 0);
                if !door_set.contains(&p) { vw.set_voxel(p, outer); }
            }
            for y in Y_MIN..=Y_MAX {
                vw.set_voxel(Pos::new(X_MIN - 1, y, 0), outer);
                vw.set_voxel(Pos::new(X_MAX + 1, y, 0), outer);
            }
            // Counter wall at y=9, gap at x=20 (employee passage)
            for x in X_MIN..=X_MAX {
                if x != 20 {
                    vw.set_voxel(Pos::new(x, COUNTER_Y, 0), Voxel::wall(granite));
                }
            }
            // Vault wall: x=10, y=13..18 with door at (8, 14)... actually
            // simpler: full vault room is x=2..9, y=14..19, with VAULT_DOOR at x=8, y=14
            for y in 14..=19 {
                let p = Pos::new(10, y, 0);
                vw.set_voxel(p, Voxel::wall(brick));
            }
            for x in X_MIN..=10 {
                let p = Pos::new(x, 13, 0);
                if !door_set.contains(&p) { vw.set_voxel(p, inner); }
                let p = Pos::new(x, 20, 0);
                if !door_set.contains(&p) { vw.set_voxel(p, inner); }
            }
            // Back office wall at y=14, x=11..38, gap for hallway
            for x in 11..=X_MAX {
                let p = Pos::new(x, 13, 0);
                if x != 20 {
                    vw.set_voxel(p, inner);
                }
            }
        }

        // Doors — front and back stay open during business hours; the
        // robbers came in the front, the back is the staff/loading
        // entrance.
        spawn_door(world, FRONT_DOOR, brick, "glass front door", DoorKind::Open);
        spawn_door(world, BACK_DOOR, brick, "back exit", DoorKind::Open);
        let _vault_door = spawn_door(world, VAULT_DOOR, brick, "vault door", DoorKind::Locked(28));

        // Furniture / fixtures
        place(world, "reception desk", Pos::new(15, COUNTER_Y - 1, 0));
        place(world, "potted plant", Pos::new(8, 4, 0));
        place(world, "potted plant", Pos::new(32, 4, 0));
        place(world, "park bench",   Pos::new(13, 6, 0));
        place(world, "park bench",   Pos::new(24, 6, 0));
        place(world, "wall clock",   Pos::new(20, 3, 0));
        place_painting(world, "abstract canvas", Pos::new(6, 3, 0));
        place_painting(world, "oil portrait",     Pos::new(34, 3, 0));
        place(world, "trash can",    Pos::new(3, 7, 0));
        place(world, "trash can",    Pos::new(37, 7, 0));

        // Teller windows along counter
        for x in [14, 18, 24, 28] {
            place(world, "teller window", Pos::new(x, COUNTER_Y, 0));
        }
        // Cash registers behind each window with stacks of cash.
        place_container(world, "cash register", Pos::new(14, 11, 0),
            &["stack of bills", "$100 bill", "$100 bill", "$100 bill"]);
        place_container(world, "cash register", Pos::new(18, 11, 0),
            &["$100 bill", "$100 bill", "$20 bill", "$20 bill"]);
        place_container(world, "cash register", Pos::new(24, 11, 0),
            &["stack of bills", "$100 bill"]);
        place_container(world, "cash register", Pos::new(28, 11, 0),
            &["$100 bill", "$20 bill"]);

        // ATMs in the lobby
        place(world, "ATM", Pos::new(4, 3, 0));
        place(world, "ATM", Pos::new(36, 3, 0));

        // Back office: desks, chairs, computers, filing cabinets
        place(world, "desk", Pos::new(15, 17, 0));
        place(world, "desk", Pos::new(25, 17, 0));
        place(world, "desk", Pos::new(15, 22, 0));
        place(world, "desk", Pos::new(25, 22, 0));
        for (x, y) in [(15, 18), (25, 18), (15, 23), (25, 23)] {
            place(world, "armchair", Pos::new(x, y, 0));
        }
        for (x, y) in [(13, 16), (27, 16), (13, 21), (27, 21)] {
            place(world, "computer", Pos::new(x, y, 0));
        }
        place(world, "printer", Pos::new(36, 17, 0));
        place(world, "water cooler", Pos::new(36, 22, 0));
        place(world, "filing cabinet", Pos::new(11, 16, 0));
        place(world, "filing cabinet", Pos::new(11, 22, 0));

        // Vault room: the bank vault + safety deposit boxes + the loot
        place_container(world, "bank vault", Pos::new(VAULT_X, VAULT_Y, 0),
            &["stack of bills", "stack of bills", "stack of bills", "stack of bills",
              "gold coin", "gold coin", "gold coin"]);
        place_container(world, "safety deposit box", Pos::new(VAULT_X, 19, 0),
            &["pearl earrings", "diamond ring"]);
        place_container(world, "safety deposit box", Pos::new(7, 19, 0),
            &["passport", "contract"]);

        // Outdoor: parking lot, getaway van, police cruiser pulling up later
        place(world, "getaway van", GETAWAY_VAN);
        place(world, "sedan",       Pos::new(7, 27, 0));
        place(world, "sedan",       Pos::new(13, 27, 0));
        place(world, "sedan",       Pos::new(31, 27, 0));
        place(world, "street lamp", Pos::new(X_MIN - 2, 5, 0));
        place(world, "street lamp", Pos::new(X_MAX + 2, 5, 0));
        place(world, "mailbox",     Pos::new(2, Y_MIN - 1, 0));

        note(world, "Mid-morning at First Federal Savings. Tellers at their windows, the manager in the back office, three customers in line, a polished marble lobby. The vault is in the back and locked.");

        // ─── people ──────────────────────────────────────────────
        // Robbers — just burst through the front door, weapons drawn,
        // announcing themselves. They start in the lobby.
        let robber_a = humanoid(world, "robber_a", Pos::new(19, 4, 0), ROBBER_FACTION,
            90, Stats::rogue());
        world.entity_mut(robber_a).insert(Robber);
        equip(world, robber_a, &["balaclava", "trench coat", "leather boots", "9mm pistol",
            "9mm round", "9mm round", "9mm round", "9mm round", "9mm round", "9mm round"]);

        let robber_b = humanoid(world, "robber_b", Pos::new(21, 4, 0), ROBBER_FACTION,
            85, Stats::rogue());
        world.entity_mut(robber_b).insert(Robber);
        equip(world, robber_b, &["balaclava", "hoodie", "rubber boots", "9mm pistol",
            "9mm round", "9mm round", "9mm round", "9mm round", "9mm round"]);

        let robber_c = humanoid(world, "robber_c", Pos::new(20, 5, 0), ROBBER_FACTION,
            110, Stats::brute());
        world.entity_mut(robber_c).insert(Robber);
        equip(world, robber_c, &["balaclava", "kevlar vest", "leather boots", "shotgun",
            "12-gauge shell", "12-gauge shell", "12-gauge shell", "12-gauge shell"]);

        // Bank staff
        let teller_a = humanoid(world, "teller_a", Pos::new(14, 11, 0), STAFF, 60, Stats::citizen());
        world.entity_mut(teller_a).insert(BankStaff)
            .insert(Identity::new("Teller Smith", "teller", Clearance::Crew))
            .insert(Knowledge::new()
                .with("face:robber", "balaclava + drawn pistol"));
        equip(world, teller_a, &["uniform shirt", "leather boots"]);
        let teller_b = humanoid(world, "teller_b", Pos::new(18, 11, 0), STAFF, 60, Stats::citizen());
        world.entity_mut(teller_b).insert(BankStaff);
        equip(world, teller_b, &["uniform shirt", "leather boots"]);
        let teller_c = humanoid(world, "teller_c", Pos::new(24, 11, 0), STAFF, 60, Stats::citizen());
        world.entity_mut(teller_c).insert(BankStaff);
        equip(world, teller_c, &["uniform shirt", "leather boots"]);
        let manager = humanoid(world, "manager", Pos::new(15, 17, 0), STAFF, 70,
            Stats { str_: 10, dex: 11, con: 12, int: 14, wis: 13, cha: 14 });
        world.entity_mut(manager).insert(BankStaff);
        equip(world, manager, &["suit jacket", "leather boots", "ID card"]);

        // Security guard with a sidearm — observer for the lobby.
        let guard = humanoid(world, "security_guard", Pos::new(20, 7, 0), STAFF, 90,
            Stats { str_: 13, dex: 12, con: 13, int: 10, wis: 12, cha: 10 });
        world.entity_mut(guard).insert(BankStaff)
            .insert(Identity::new("Officer Hayes", "security_guard", Clearance::Pilot))
            .insert(Observer::new(8, Clearance::Public, "bank lobby"));
        equip(world, guard, &["uniform shirt", "leather boots", "9mm pistol",
            "9mm round", "9mm round", "9mm round", "9mm round"]);

        // Customers
        for (name, x) in [("customer_a", 8), ("customer_b", 11), ("customer_c", 30)] {
            let c = humanoid(world, name, Pos::new(x, 7, 0), PUBLIC, 50, Stats::citizen());
            world.entity_mut(c).insert(Civilian);
            equip(world, c, &["cotton t-shirt", "leather boots", "$20 bill"]);
        }

        note(world, "Three figures in balaclavas step out of an unmarked van and stride toward the back door of the bank. One has a shotgun.");
    }

    fn build_schedule(&mut self) -> Schedule {
        let mut schedule = Schedule::default();
        // Group A: planning + executing.
        schedule.add_systems(
            (
                tick_needs,
                derive_mood,
                door_voxel_sync,
                bystander_fear,
                robber_planner,
                staff_planner,
                police_planner,
                dialog_system,
                execute_tasks,
                handle_door_use,
            )
                .chain(),
        );
        // Group B: post-action (sounds, perception, status). After
        // group A so events from execute_tasks have already been
        // logged and consumed.
        schedule.add_systems(
            (
                furniture_emit_system,
                emit_combat_sounds,
                emit_movement_sounds,
                update_sight,
                update_hearing,
                update_smell,
                observation_system,
                tick_status_effects,
                footing_check,
                retaliation_system,
                fear_from_combat,
                decay_coatings,
                check_completion,
                police_arrival,
            )
                .chain()
                .after(handle_door_use),
        );
        schedule
    }

    fn is_complete(&self, world: &mut World) -> bool {
        // Done when all robbers are dead or departed, or all staff dead.
        let mut any_active_robber = world
            .query_filtered::<(&Health, Option<&Departed>), With<Robber>>();
        let active = any_active_robber
            .iter(world)
            .any(|(h, d)| h.is_alive() && d.is_none());
        !active
    }
}

// ─── helpers ───────────────────────────────────────────────────────

#[derive(Copy, Clone)]
enum DoorKind { Open, Closed, Locked(i32) }

fn spawn_door(world: &mut World, pos: Pos, mat: u16, label: &str, kind: DoorKind) -> Entity {
    let door = match kind {
        DoorKind::Open => { let mut d = Door::closed(mat, label); d.state = DoorState::Open; d }
        DoorKind::Closed => Door::closed(mat, label),
        DoorKind::Locked(dc) => Door::locked(mat, label, dc),
    };
    world.spawn((Position(pos), Kind(label.into()), door)).id()
}

fn place(world: &mut World, template: &str, pos: Pos) {
    let _ = spawn_furniture_template(world, template,
        FurnitureSpawnOpts { at: pos, kind_label: None });
}

fn place_painting(world: &mut World, template: &str, pos: Pos) {
    let _ = spawn_furniture_template(world, template,
        FurnitureSpawnOpts { at: pos, kind_label: None });
}

fn place_container(world: &mut World, template: &str, pos: Pos, contents: &[&str]) {
    use fortress_engine::furniture::Container;
    let container = match spawn_furniture_template(world, template,
        FurnitureSpawnOpts { at: pos, kind_label: None }) {
        Ok(e) => e, Err(_) => return,
    };
    let mut item_ids: Vec<Entity> = Vec::new();
    for &name in contents {
        let template_name = if world.resource::<fortress_engine::Library>().items.contains_key(name) {
            name.to_string()
        } else { "kitchen knife".to_string() };
        let opts = ItemSpawnOpts {
            at: None,
            override_label: Some(name.to_string()),
            ..Default::default()
        };
        if let Ok(item) = spawn_item_template(world, &template_name, opts) {
            world.entity_mut(item).insert(VaultLoot);
            item_ids.push(item);
        }
    }
    if let Some(mut c) = world.get_mut::<Container>(container) {
        c.items.extend(item_ids);
    }
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

// ─── per-tick systems ──────────────────────────────────────────────

fn bystander_fear(world: &mut World) {
    let alerts: Vec<(Entity, f32)> = {
        let mut q = world.query::<(Entity, &Perceived, &Fear)>();
        q.iter(world).filter_map(|(e, p, _)| {
            p.loudest_violent().map(|hs| (e, hs.apparent_intensity))
        }).collect()
    };
    for (entity, intensity) in alerts {
        if let Some(mut f) = world.get_mut::<Fear>(entity) {
            f.frighten(0.10 + intensity * 0.20);
        }
    }
}

fn robber_planner(world: &mut World) {
    use fortress_engine::furniture::Container;
    let robber_ids: std::collections::HashSet<Entity> = {
        let mut q = world.query_filtered::<Entity, (With<Robber>, Without<Departed>)>();
        q.iter(world).collect()
    };
    let robbers: Vec<(Entity, Pos, Goal, bool)> = {
        let mut q = world.query_filtered::<(Entity, &Position, &Goal, &TaskQueue),
            (With<Robber>, Without<Departed>)>();
        q.iter(world).map(|(e, p, g, q)| (e, p.0, g.clone(), q.is_empty())).collect()
    };
    let visible_targets_per: std::collections::HashMap<Entity, Option<Entity>> = robbers.iter()
        .map(|(rid, _, _, _)| {
            let target = world.get::<Perceived>(*rid).and_then(|p| {
                p.seen.iter()
                    .filter(|s| !robber_ids.contains(&s.entity))
                    .filter(|s| world.get::<Health>(s.entity).map(|h| h.is_alive()).unwrap_or(false))
                    .filter(|s| {
                        // Prioritize police, then security, then armed staff.
                        world.get::<Police>(s.entity).is_some()
                            || world.get::<RangedWeapon>(s.entity).is_some()
                            || world.get::<Identity>(s.entity).map(|i| i.role == "security_guard").unwrap_or(false)
                    })
                    .min_by_key(|s| s.distance)
                    .map(|s| s.entity)
            });
            (*rid, target)
        })
        .collect();

    // Containers (vault + cash registers). Skip locks beyond what
    // the brute (STR +3) can plausibly force — ATMs (26) and
    // bank vault (30) need real tools.
    let containers: Vec<(Entity, Pos, bool)> = {
        let mut q = world.query::<(Entity, &Position, &Container)>();
        q.iter(world)
            .filter(|(_, _, c)| !c.locked || c.lock_dc <= 22)
            .map(|(e, p, c)| (e, p.0, c.open))
            .collect()
    };
    // Loose loot already on the floor (after a container was opened)
    let loose_loot: Vec<(Entity, Pos, u32)> = {
        let mut q = world.query_filtered::<(Entity, &Position), With<VaultLoot>>();
        q.iter(world).map(|(e, p)| {
            let v = world.get::<fortress_engine::Value>(e).map(|v| v.0).unwrap_or(0);
            (e, p.0, v)
        }).collect()
    };

    for (robber, pos, goal, queue_empty) in robbers {
        if !world.get::<Health>(robber).map(|h| h.is_alive()).unwrap_or(false) { continue; }

        // 1) Visible armed threat → shoot if we have ammo, else
        // close to melee.
        if let Some(t) = visible_targets_per.get(&robber).copied().flatten() {
            let has_ammo = robber_has_ammo(world, robber);
            let task = if has_ammo { Task::Shoot(t) } else { Task::Attack(t) };
            if !matches!(goal, Goal::Kill(e) if e == t) || queue_empty {
                if let Some(mut q) = world.get_mut::<TaskQueue>(robber) {
                    q.clear();
                    q.push(task);
                }
                if let Some(mut g) = world.get_mut::<Goal>(robber) { *g = Goal::Kill(t); }
                world.entity_mut(robber).insert(Locomotion::Running);
            }
            continue;
        }

        // 2) Loose loot on floor → priciest first
        let mut loot = loose_loot.clone();
        loot.sort_by(|a, b| b.2.cmp(&a.2).then(pos.manhattan(a.1).cmp(&pos.manhattan(b.1))));
        let pickable = {
            let vw = world.resource::<VoxelWorld>();
            loot.into_iter().find(|(_, lp, _)| find_path(vw, pos, *lp, 4096).is_some())
        };
        if let Some((item, item_pos, _)) = pickable {
            if let Some(mut q) = world.get_mut::<TaskQueue>(robber) {
                q.clear();
                q.push(Task::MoveTo(item_pos));
                q.push(Task::PickUp(item));
            }
            if let Some(mut g) = world.get_mut::<Goal>(robber) { *g = Goal::Tend(item); }
            world.entity_mut(robber).insert(Locomotion::Running);
            continue;
        }

        // 3) Closest unopened container we can reach → open it
        let mut conts = containers.iter().copied()
            .filter(|(_, _, open)| !*open).collect::<Vec<_>>();
        conts.sort_by_key(|(_, cp, _)| pos.manhattan(*cp));
        let target_container = {
            let vw = world.resource::<VoxelWorld>();
            conts.into_iter().find(|(_, cp, _)| {
                let approach = approach_tile(pos, *cp);
                find_path(vw, pos, approach, 4096).is_some()
            })
        };
        if let Some((cont, cont_pos, _)) = target_container {
            let approach = approach_tile(pos, cont_pos);
            if let Some(mut q) = world.get_mut::<TaskQueue>(robber) {
                q.clear();
                if pos != approach { q.push(Task::MoveTo(approach)); }
                q.push(Task::UseEntity(cont));
            }
            if let Some(mut g) = world.get_mut::<Goal>(robber) { *g = Goal::Tend(cont); }
            world.entity_mut(robber).insert(Locomotion::Walking);
            continue;
        }

        // 4) Done — head for the van.
        if !matches!(goal, Goal::GoTo(p) if p == GETAWAY_VAN) || queue_empty {
            if let Some(mut q) = world.get_mut::<TaskQueue>(robber) {
                q.clear();
                q.push(Task::MoveTo(GETAWAY_VAN));
            }
            if let Some(mut g) = world.get_mut::<Goal>(robber) { *g = Goal::GoTo(GETAWAY_VAN); }
            world.entity_mut(robber).insert(Locomotion::Running);
        }
    }
}

fn staff_planner(world: &mut World) {
    // Tellers: when they SEE a robber, raise alarm + cower (Wait).
    let tellers: Vec<Entity> = {
        let mut q = world.query_filtered::<Entity, (With<BankStaff>, Without<Alarmed>)>();
        q.iter(world).collect()
    };
    for staff in tellers {
        let saw_robber = world.get::<Perceived>(staff).and_then(|p|
            p.seen.iter().find(|s| world.get::<Robber>(s.entity).is_some())
                .map(|s| s.entity)
        );
        if let Some(r) = saw_robber {
            world.entity_mut(staff).insert(Alarmed);
            let label = label_kind(world, staff);
            let tick = world.resource::<Clock>().tick;
            world.resource_mut::<EventLog>().push(tick,
                Event::AlarmRaised { observer: staff, target: r });
            // Tellers cower (long Wait); the security guard fights.
            if world.get::<Identity>(staff).map(|i| i.role == "security_guard").unwrap_or(false) {
                if let Some(mut q) = world.get_mut::<TaskQueue>(staff) {
                    q.clear(); q.push(Task::Shoot(r));
                }
                if let Some(mut g) = world.get_mut::<Goal>(staff) { *g = Goal::Kill(r); }
            } else {
                if let Some(mut q) = world.get_mut::<TaskQueue>(staff) {
                    q.clear(); q.push(Task::Wait(99));
                }
            }
        }
    }
}

fn police_arrival(world: &mut World) {
    // 25 ticks after the alarm, two officers spawn at the parking lot
    // entrance and start hunting robbers.
    if world.query_filtered::<&PoliceArrived, ()>().iter(world).next().is_some() {
        return;
    }
    let alarm_raised = world.query_filtered::<Entity, With<Alarmed>>().iter(world).next().is_some();
    if !alarm_raised { return; }
    let alarm_tick = world.resource::<EventLog>().all().iter()
        .find(|(_, e)| matches!(e, Event::AlarmRaised { .. }))
        .map(|(t, _)| *t)
        .unwrap_or(u64::MAX);
    let now = world.resource::<Clock>().tick;
    if now < alarm_tick + 18 { return; }

    let _marker = world.spawn(PoliceArrived).id();
    for (i, x) in [22, 18].iter().enumerate() {
        let officer = humanoid(world, &format!("officer_{i}"),
            Pos::new(*x, Y_MAX + 5, 0), POLICE, 90,
            Stats { str_: 13, dex: 14, con: 13, int: 11, wis: 13, cha: 11 });
        world.entity_mut(officer).insert(Police);
        equip(world, officer, &["kevlar vest", "leather boots", "9mm pistol",
            "9mm round", "9mm round", "9mm round", "9mm round", "9mm round"]);
    }
    let tick = world.resource::<Clock>().tick;
    world.resource_mut::<EventLog>().push(tick,
        Event::Note("Two squad cars screech into the parking lot — officers fan out toward the bank.".into()));
}

fn police_planner(world: &mut World) {
    let robbers: Vec<Entity> = world.query_filtered::<Entity, With<Robber>>().iter(world).collect();
    let officers: Vec<Entity> = world.query_filtered::<Entity, With<Police>>().iter(world).collect();
    for cop in officers {
        if !world.get::<Health>(cop).map(|h| h.is_alive()).unwrap_or(false) { continue; }
        let queue_empty = world.get::<TaskQueue>(cop).map(|q| q.is_empty()).unwrap_or(true);
        let visible = world.get::<Perceived>(cop).and_then(|p|
            p.seen.iter().find(|s| robbers.contains(&s.entity)).map(|s| s.entity)
        );
        if let Some(r) = visible {
            let has_ammo = robber_has_ammo(world, cop);
            let task = if has_ammo { Task::Shoot(r) } else { Task::Attack(r) };
            if !matches!(world.get::<Goal>(cop), Some(Goal::Kill(e)) if *e == r) {
                if let Some(mut q) = world.get_mut::<TaskQueue>(cop) {
                    q.clear(); q.push(task);
                }
                if let Some(mut g) = world.get_mut::<Goal>(cop) { *g = Goal::Kill(r); }
                world.entity_mut(cop).insert(Locomotion::Running);
            }
            continue;
        }
        // Otherwise advance toward the bank's back door.
        if queue_empty {
            if let Some(mut q) = world.get_mut::<TaskQueue>(cop) {
                q.push(Task::MoveTo(BACK_DOOR));
            }
            if let Some(mut g) = world.get_mut::<Goal>(cop) { *g = Goal::GoTo(BACK_DOOR); }
            world.entity_mut(cop).insert(Locomotion::Running);
        }
    }
}

fn check_completion(world: &mut World) {
    let arrived: Vec<Entity> = {
        let mut q = world.query_filtered::<(Entity, &Position),
            (With<Robber>, Without<Departed>)>();
        q.iter(world).filter(|(_, p)| p.0.manhattan(GETAWAY_VAN) <= 1)
            .map(|(e, _)| e).collect()
    };
    for r in arrived {
        world.entity_mut(r).insert(Departed);
        let label = label_kind(world, r);
        let tick = world.resource::<Clock>().tick;
        world.resource_mut::<EventLog>().push(tick,
            Event::Note(format!("{label} dives into the getaway van.")));
    }
}

fn handle_door_use(world: &mut World) {
    use fortress_engine::furniture::Container;
    use fortress_engine::{manipulation_check, strength_check, CheckOutcome};
    let tick = world.resource::<Clock>().tick;
    let attempts: Vec<(Entity, Entity)> = world.resource::<EventLog>().events_at(tick)
        .filter_map(|e| match e {
            Event::EntityUsed { user, target } => Some((*user, *target)),
            _ => None,
        }).collect();
    for (user, target) in attempts {
        // Containers
        if world.get::<Container>(target).is_some() {
            let (open, locked, lock_dc, kind, pos) = {
                let c = world.get::<Container>(target).unwrap();
                let p = world.get::<Position>(target).map(|p| p.0).unwrap_or_default();
                let k = world.get::<Kind>(target).map(|k| k.0.clone()).unwrap_or_else(|| "container".into());
                (c.open, c.locked, c.lock_dc, k, p)
            };
            if open { continue; }
            if locked {
                let outcome = strength_check(world, user, lock_dc);
                let success = matches!(outcome, CheckOutcome::Success(_));
                let roll = match &outcome {
                    CheckOutcome::Success(r) | CheckOutcome::Failure(r) => r.total,
                    CheckOutcome::Impossible => 0,
                };
                let impossible = matches!(outcome, CheckOutcome::Impossible);
                world.resource_mut::<EventLog>().push(tick,
                    Event::AbilityCheck {
                        actor: user, kind: "force open".into(),
                        target: format!("the {kind}"),
                        roll, dc: lock_dc, success, impossible,
                    });
                if !success { continue; }
            }
            let items: Vec<Entity> = {
                let mut c = world.get_mut::<Container>(target).unwrap();
                c.open = true;
                std::mem::take(&mut c.items)
            };
            for it in &items {
                world.entity_mut(*it).insert(Position(pos));
            }
            let label = label_kind(world, user);
            world.resource_mut::<EventLog>().push(tick,
                Event::Note(format!("{label} pulls open the {kind} — {} items spill out.", items.len())));
            continue;
        }
        // Doors
        if let Some(d) = world.get::<Door>(target).cloned() {
            match d.state {
                DoorState::Open => {}
                DoorState::Closed => {
                    let outcome = manipulation_check(world, user, 5);
                    let success = matches!(outcome, CheckOutcome::Success(_));
                    if success {
                        if let Some(mut dd) = world.get_mut::<Door>(target) {
                            dd.state = DoorState::Open;
                        }
                    }
                }
                DoorState::Locked => {
                    let outcome = strength_check(world, user, d.break_dc);
                    let success = matches!(outcome, CheckOutcome::Success(_));
                    if success {
                        if let Some(mut dd) = world.get_mut::<Door>(target) {
                            dd.state = DoorState::Broken;
                        }
                    }
                }
                DoorState::Broken => {}
            }
        }
    }
}

fn robber_has_ammo(world: &World, actor: Entity) -> bool {
    use fortress_engine::{Ammo, Inventory};
    world
        .get::<Inventory>(actor)
        .map(|inv| inv.0.iter().any(|e| world.get::<Ammo>(*e).is_some()))
        .unwrap_or(false)
}

fn approach_tile(from: Pos, target: Pos) -> Pos {
    let dy = (from.y - target.y).signum();
    let dx = (from.x - target.x).signum();
    if dy != 0 { Pos::new(target.x, target.y + dy, target.z) }
    else if dx != 0 { Pos::new(target.x + dx, target.y, target.z) }
    else { Pos::new(target.x, target.y + 1, target.z) }
}

fn label_kind(world: &World, entity: Entity) -> String {
    match world.get::<Kind>(entity) {
        Some(k) => format!("{}#{}", k.0, entity.index()),
        None => format!("entity#{}", entity.index()),
    }
}

#[allow(dead_code)]
fn _unused() { let _ = Suspicion::label(0.0); }
