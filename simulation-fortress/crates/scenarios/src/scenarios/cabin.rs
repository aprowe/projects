//! Old retired killer in a cabin in the woods.
//!
//! 30×30 forest clearing with a small cabin (a single room) at the
//! center. An old man sits in his rocking chair on the porch. Around
//! him: oak trees, pines, redwoods, scattered boulders, a brush
//! path snaking out from the cabin to the road. Inside the cabin a
//! locked safe holds the contents of his old life — a shotgun, a
//! pistol, plenty of ammo, kevlar.
//!
//! After ~tick 8 a getaway van pulls up at the road. Three
//! assassins jump out and approach. The old man's planner is
//! deliberately *reactive*: when he hears the engine / sees an
//! armed stranger, his goal flips from "rocking" to "get to the
//! vault", and only THEN to "engage". He doesn't have a hardcoded
//! script — given his very high combat stats, what emerges is that
//! he reaches the safe, equips, walks out, and shreds them.

use fortress_engine::actions::{fill_region_logged, note, spawn_creature};
use fortress_engine::library::{FurnitureSpawnOpts, ItemSpawnOpts};
use fortress_engine::prelude::*;
use fortress_engine::{
    decay_coatings, derive_mood, door_voxel_sync, emit_combat_sounds, emit_movement_sounds,
    ensure_material, execute_tasks, fear_from_combat, find_path, footing_check,
    furniture_emit_system, retaliation_system, spawn_furniture_template, spawn_humanoid_body,
    tick_fire,
    spawn_item_template, tick_needs, tick_status_effects, update_hearing, update_sight,
    update_smell, Ammo, Clock, Door, DoorState, Event, EventLog, Fear, Goal, Health, Hearing,
    Inventory, Kind, Locomotion, Mood, Perceived, Position, Pos, RangedWeapon,
    RetaliateOnAttack, Scenario, Sight, Stats, Task, TaskQueue, Voxel, VoxelWorld, Wearing,
};

const ROAD_Y: i32 = 27;
const VAN_POS: Pos = Pos::new(15, 28, 0);
const CABIN_X_MIN: i32 = 12;
const CABIN_X_MAX: i32 = 18;
const CABIN_Y_MIN: i32 = 10;
const CABIN_Y_MAX: i32 = 16;
const PORCH_POS: Pos = Pos::new(15, 17, 0);
const CABIN_DOOR: Pos = Pos::new(15, 16, 0);
const SAFE_POS: Pos = Pos::new(13, 11, 0);

const OLD_MAN: &str = "retiree";
const ASSASSIN: &str = "assassin";

#[derive(Component)] pub struct OldMan;
#[derive(Component)] pub struct Assassin;
#[derive(Component)] pub struct ArmedUp;

#[derive(Default)]
pub struct CabinAmbush;

impl Scenario for CabinAmbush {
    fn name(&self) -> &str {
        "retired killer's cabin"
    }

    fn setup(&mut self, world: &mut World) {
        // Pre-dawn — sun's still low, cool morning.
        {
            let mut c = world.resource_mut::<Clock>();
            c.start_minute = 5 * 60 + 50;
        }

        // Ground: grass everywhere, road of gravel cutting south,
        // brush path of dirt from cabin to road.
        let grass = ensure_material(world, "grass").expect("grass");
        let gravel = ensure_material(world, "gravel").expect("gravel");
        let dirt = ensure_material(world, "dirt").expect("dirt");
        fill_region_logged(world, Pos::new(0, 0, 0), Pos::new(29, 29, 0), Voxel::floor(grass));
        // Gravel road across the south
        for x in 0..=29 {
            world.resource_mut::<VoxelWorld>().set_voxel(Pos::new(x, ROAD_Y, 0), Voxel::floor(gravel));
            world.resource_mut::<VoxelWorld>().set_voxel(Pos::new(x, ROAD_Y + 1, 0), Voxel::floor(gravel));
        }
        // Brush path from cabin porch (15, 17) to road (15, 26)
        for y in 18..=26 {
            world.resource_mut::<VoxelWorld>().set_voxel(Pos::new(15, y, 0), Voxel::floor(dirt));
        }

        // Cabin: pine floor, log walls, door.
        let pine = ensure_material(world, "pine").expect("pine");
        let oak = ensure_material(world, "oak").expect("oak");
        fill_region_logged(world,
            Pos::new(CABIN_X_MIN, CABIN_Y_MIN, 0),
            Pos::new(CABIN_X_MAX, CABIN_Y_MAX, 0),
            Voxel::floor(pine));
        let log_wall = Voxel::wall(oak);
        {
            let mut vw = world.resource_mut::<VoxelWorld>();
            for x in (CABIN_X_MIN - 1)..=(CABIN_X_MAX + 1) {
                vw.set_voxel(Pos::new(x, CABIN_Y_MIN - 1, 0), log_wall);
                if x != CABIN_DOOR.x {
                    vw.set_voxel(Pos::new(x, CABIN_Y_MAX + 1, 0), log_wall);
                }
            }
            for y in CABIN_Y_MIN..=CABIN_Y_MAX {
                vw.set_voxel(Pos::new(CABIN_X_MIN - 1, y, 0), log_wall);
                vw.set_voxel(Pos::new(CABIN_X_MAX + 1, y, 0), log_wall);
            }
        }
        // Concrete porch tile in front of the door.
        let concrete = ensure_material(world, "concrete").expect("concrete");
        world.resource_mut::<VoxelWorld>().set_voxel(PORCH_POS, Voxel::floor(concrete));

        // Cabin door — left open this morning while he had his coffee.
        spawn_door(world, CABIN_DOOR, oak, "cabin door", DoorKind::Open);

        // ─── interior furniture ───────────────────────────────────
        place(world, "fireplace", Pos::new(13, 10, 0));
        place(world, "armchair",  Pos::new(17, 11, 0));
        place(world, "side table", Pos::new(17, 12, 0));
        place(world, "table lamp", Pos::new(17, 13, 0));
        place(world, "twin bed",  Pos::new(17, 14, 0));
        place(world, "bookshelf", Pos::new(13, 14, 0));
        place(world, "books",     Pos::new(14, 14, 0));
        place_painting(world, "photograph", Pos::new(15, 10, 0));

        // The vault: a locked safe in the corner stocked for war.
        // Lock is high (the old man knows the combination); when he
        // opens it, the contents drop on the floor and he picks them
        // up (visible as Valuables to him because he's a creature
        // observer too — but actually the item-pickup task just
        // needs them on the floor and adjacent).
        place_container(world, "safe", SAFE_POS, &[
            "shotgun", "12-gauge shell", "12-gauge shell", "12-gauge shell",
            "12-gauge shell", "12-gauge shell", "12-gauge shell", "12-gauge shell",
            "12-gauge shell", "kevlar vest", "9mm pistol",
            "9mm round", "9mm round", "9mm round", "9mm round",
            "9mm round", "9mm round", "9mm round", "9mm round",
            "9mm round", "9mm round", "9mm round", "9mm round",
        ]);

        // ─── outside: porch dressing ──────────────────────────────
        place(world, "potted plant", Pos::new(12, 17, 0));
        place(world, "potted plant", Pos::new(18, 17, 0));

        // ─── woods: scattered trees, bushes, rocks ────────────────
        // Generate "natural" placements via deterministic offsets so
        // the layout is reproducible without RNG.
        let trees: &[(&str, i32, i32)] = &[
            ("redwood tree", 2, 2),
            ("redwood tree", 24, 4),
            ("oak tree", 6, 5),
            ("oak tree", 22, 9),
            ("oak tree", 8, 22),
            ("oak tree", 22, 22),
            ("pine tree", 4, 8),
            ("pine tree", 4, 12),
            ("pine tree", 4, 16),
            ("pine tree", 4, 20),
            ("pine tree", 25, 13),
            ("pine tree", 25, 17),
            ("pine tree", 25, 21),
            ("pine tree", 11, 4),
            ("pine tree", 18, 4),
            ("pine tree", 11, 24),
            ("pine tree", 18, 24),
            ("birch tree", 9, 8),
            ("birch tree", 21, 6),
            ("birch tree", 7, 19),
            ("birch tree", 23, 18),
            ("sapling", 10, 19),
            ("sapling", 20, 20),
            ("sapling", 6, 14),
        ];
        for (name, x, y) in trees {
            place(world, name, Pos::new(*x, *y, 0));
        }
        for (x, y) in [(3, 5), (5, 24), (26, 24), (27, 7), (10, 6), (8, 11), (22, 12), (10, 22)] {
            place(world, "bush", Pos::new(x, y, 0));
        }
        for (x, y) in [(7, 7), (24, 7), (3, 18), (26, 19), (11, 22), (19, 22)] {
            place(world, "rock", Pos::new(x, y, 0));
        }
        place(world, "log",   Pos::new(20, 16, 0));
        place(world, "stump", Pos::new(11, 14, 0));
        place(world, "stump", Pos::new(19, 14, 0));
        place(world, "camp fire", Pos::new(12, 18, 0));

        // Mailbox by the road.
        place(world, "mailbox", Pos::new(13, ROAD_Y - 1, 0));

        note(world, "Mist clings to a clearing in the woods. A small log cabin sits at the center; smoke rises lazily from the chimney. An old man in a thick wool coat rocks on the porch with a coffee, watching the light come up through the redwoods.");

        // ─── the old man ──────────────────────────────────────────
        // Very high combat stats — DEX 18, STR 14, WIS 17 — but
        // average HP because he's old. He's an observer of the
        // surrounding woods (long sight, keen hearing).
        let old = humanoid(world, "retiree", PORCH_POS, OLD_MAN, 70,
            Stats { str_: 14, dex: 18, con: 13, int: 14, wis: 17, cha: 12 });
        world.entity_mut(old).insert(OldMan).insert(RetaliateOnAttack)
            .insert(Sight::keen())
            .insert(Hearing::keen());
        equip(world, old, &["wool shirt", "leather boots", "coffee mug"]);

        // ─── the assassins (spawn at van) ─────────────────────────
        // Three rogues with pistols and one with a shotgun. They
        // come in armed and announce themselves by walking up the
        // brush path.
        for (name, dx, dy, weapon, ammo, ammo_count) in [
            ("assassin_alpha", -1, 0, "9mm pistol", "9mm round", 6),
            ("assassin_bravo",  0, 0, "shotgun", "12-gauge shell", 4),
            ("assassin_charlie", 1, 0, "9mm pistol", "9mm round", 6),
        ] {
            let a = humanoid(world, name,
                Pos::new(VAN_POS.x + dx, VAN_POS.y + dy, 0),
                ASSASSIN, 80, Stats::rogue());
            world.entity_mut(a).insert(Assassin);
            equip(world, a, &["balaclava", "trench coat", "leather boots", weapon]);
            for _ in 0..ammo_count {
                let _ = spawn_item_template(world, ammo,
                    ItemSpawnOpts { equip_on: Some(a), ..Default::default() });
            }
        }
        place(world, "getaway van", VAN_POS);

        note(world, "Down the gravel road, an unmarked van slows to a stop. Three figures step out, weapons low.");
    }

    fn build_schedule(&mut self) -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems((
            tick_needs, derive_mood, door_voxel_sync,
            old_man_planner, assassin_planner, execute_tasks, handle_door_use,
        ).chain());
        schedule.add_systems((
            furniture_emit_system, emit_combat_sounds, emit_movement_sounds,
            update_sight, update_hearing, update_smell,
            tick_status_effects, tick_fire, footing_check, retaliation_system,
            fear_from_combat, decay_coatings,
        ).chain().after(handle_door_use));
        schedule
    }

    fn is_complete(&self, world: &mut World) -> bool {
        // Done when all assassins are dead OR the old man is dead.
        let assassins_alive = world.query_filtered::<&Health, With<Assassin>>()
            .iter(world).any(|h| h.is_alive());
        let old_alive = world.query_filtered::<&Health, With<OldMan>>()
            .iter(world).any(|h| h.is_alive());
        !assassins_alive || !old_alive
    }
}

// ─── helpers ──────────────────────────────────────────────────────

#[derive(Copy, Clone)]
enum DoorKind { Open, Closed, Locked(i32) }

fn spawn_door(world: &mut World, pos: Pos, mat: u16, label: &str, kind: DoorKind) {
    let door = match kind {
        DoorKind::Open => { let mut d = Door::closed(mat, label); d.state = DoorState::Open; d }
        DoorKind::Closed => Door::closed(mat, label),
        DoorKind::Locked(dc) => Door::locked(mat, label, dc),
    };
    world.spawn((Position(pos), Kind(label.into()), door));
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

// ─── per-tick systems ─────────────────────────────────────────────

/// Reactive planner: the old man rocks until he sees or hears
/// armed strangers. Then his goal is "get to the safe, equip, fight
/// back". Every step is driven by current perception, not a script.
fn old_man_planner(world: &mut World) {
    let oldman: Vec<(Entity, Pos, bool)> = {
        let mut q = world.query_filtered::<(Entity, &Position, &TaskQueue), With<OldMan>>();
        q.iter(world).map(|(e, p, q)| (e, p.0, q.is_empty())).collect()
    };
    for (old, pos, queue_empty) in oldman {
        if !world.get::<Health>(old).map(|h| h.is_alive()).unwrap_or(false) { continue; }

        let perceived_threat = perceived_threat(world, old);
        let armed = world.get::<ArmedUp>(old).is_some() || has_ranged(world, old);

        // Priority 1: if armed and a target is visible, shoot.
        if armed {
            if let Some(target) = perceived_threat {
                if !matches!(world.get::<Goal>(old), Some(Goal::Kill(t)) if *t == target) {
                    let task = if has_ranged(world, old) && has_ammo(world, old) {
                        Task::Shoot(target)
                    } else {
                        Task::Attack(target)
                    };
                    if let Some(mut q) = world.get_mut::<TaskQueue>(old) {
                        q.clear();
                        q.push(task);
                    }
                    if let Some(mut g) = world.get_mut::<Goal>(old) { *g = Goal::Kill(target); }
                    world.entity_mut(old).insert(Locomotion::Walking);
                }
                continue;
            }
        }

        // Priority 2 (always-on while unarmed): equip whatever's
        // adjacent on the ground. Picking up a ranged weapon flips
        // ArmedUp on the next pass.
        if !armed {
            // Adjacent ranged weapon → pick + equip first.
            let nearby_weapon: Option<Entity> = {
                let mut q = world.query::<(Entity, &Position, &RangedWeapon)>();
                q.iter(world)
                    .filter(|(_, p, _)| pos.chebyshev(p.0) <= 1)
                    .map(|(e, _, _)| e)
                    .next()
            };
            if let Some(w) = nearby_weapon {
                if let Some(mut q) = world.get_mut::<TaskQueue>(old) {
                    q.clear();
                    q.push(Task::PickUp(w));
                    q.push(Task::Equip(w));
                }
                continue;
            }
            // Adjacent ammo → grab a few rounds.
            let nearby_ammo: Vec<Entity> = {
                let mut q = world.query::<(Entity, &Position, &Ammo)>();
                q.iter(world)
                    .filter(|(_, p, _)| pos.chebyshev(p.0) <= 1)
                    .map(|(e, _, _)| e)
                    .take(6)
                    .collect()
            };
            if !nearby_ammo.is_empty() && queue_empty {
                if let Some(mut q) = world.get_mut::<TaskQueue>(old) {
                    for a in nearby_ammo { q.push(Task::PickUp(a)); }
                }
                continue;
            }
        }

        // Priority 3: threat detected and unarmed → race to safe.
        if perceived_threat.is_some() && !armed {
            let approach = approach_tile(pos, SAFE_POS);
            let safe = lookup_kind(world, "safe");
            let safe_unopened = safe
                .and_then(|s| world.get::<fortress_engine::furniture::Container>(s))
                .map(|c| !c.open)
                .unwrap_or(true);
            if queue_empty {
                if let Some(mut q) = world.get_mut::<TaskQueue>(old) {
                    if pos != approach {
                        q.push(Task::MoveTo(approach));
                    }
                    if let (Some(s), true) = (safe, safe_unopened) {
                        q.push(Task::UseEntity(s));
                    }
                }
                if let Some(mut g) = world.get_mut::<Goal>(old) { *g = Goal::Flee(approach); }
                world.entity_mut(old).insert(Locomotion::Running);
            }
            continue;
        }

        // Priority 4: armed but no visible target → grab any
        // ammo/kevlar still on the floor, then mark ArmedUp.
        if armed && world.get::<ArmedUp>(old).is_none() {
            world.entity_mut(old).insert(ArmedUp);
            continue;
        }

        // Priority 5: nothing happening — rock on.
        if queue_empty {
            if let Some(mut q) = world.get_mut::<TaskQueue>(old) {
                q.push(Task::Wait(2));
            }
        }
    }
}

fn assassin_planner(world: &mut World) {
    let cabin_porch = PORCH_POS;
    let assassins: Vec<(Entity, Pos, bool)> = {
        let mut q = world.query_filtered::<(Entity, &Position, &TaskQueue), With<Assassin>>();
        q.iter(world).map(|(e, p, q)| (e, p.0, q.is_empty())).collect()
    };
    for (a, pos, queue_empty) in assassins {
        if !world.get::<Health>(a).map(|h| h.is_alive()).unwrap_or(false) { continue; }

        // 1) Visible old man → shoot.
        let target = world.get::<Perceived>(a).and_then(|p|
            p.seen.iter().find(|s| world.get::<OldMan>(s.entity).is_some()).map(|s| s.entity)
        );
        if let Some(t) = target {
            let task = if has_ammo(world, a) { Task::Shoot(t) } else { Task::Attack(t) };
            if !matches!(world.get::<Goal>(a), Some(Goal::Kill(e)) if *e == t) {
                if let Some(mut q) = world.get_mut::<TaskQueue>(a) {
                    q.clear(); q.push(task);
                }
                if let Some(mut g) = world.get_mut::<Goal>(a) { *g = Goal::Kill(t); }
                world.entity_mut(a).insert(Locomotion::Running);
            }
            continue;
        }

        // 2) March on the cabin porch.
        if queue_empty {
            if let Some(mut q) = world.get_mut::<TaskQueue>(a) {
                q.push(Task::MoveTo(cabin_porch));
            }
            if let Some(mut g) = world.get_mut::<Goal>(a) { *g = Goal::GoTo(cabin_porch); }
            world.entity_mut(a).insert(Locomotion::Walking);
        }
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
        if world.get::<Container>(target).is_some() {
            let (open, locked, lock_dc, kind, pos) = {
                let c = world.get::<Container>(target).unwrap();
                let p = world.get::<Position>(target).map(|p| p.0).unwrap_or_default();
                let k = world.get::<Kind>(target).map(|k| k.0.clone()).unwrap_or_else(|| "container".into());
                (c.open, c.locked, c.lock_dc, k, p)
            };
            if open { continue; }
            if locked {
                // The old man knows the combination — let him bypass
                // with a manipulation check (DEX-based) instead of a
                // strength check.
                let outcome = if world.get::<OldMan>(user).is_some() {
                    manipulation_check(world, user, lock_dc / 2)
                } else {
                    strength_check(world, user, lock_dc)
                };
                let success = matches!(outcome, CheckOutcome::Success(_));
                if !success { continue; }
            }
            let items: Vec<Entity> = {
                let mut c = world.get_mut::<Container>(target).unwrap();
                c.open = true;
                std::mem::take(&mut c.items)
            };
            for it in &items { world.entity_mut(*it).insert(Position(pos)); }
            let label = label_kind(world, user);
            world.resource_mut::<EventLog>().push(tick,
                Event::Note(format!("{label} flips open the {kind} — {} items spill onto the floor.", items.len())));
            continue;
        }
        if let Some(d) = world.get::<Door>(target).cloned() {
            match d.state {
                DoorState::Open => {}
                DoorState::Closed => {
                    let outcome = manipulation_check(world, user, 5);
                    if matches!(outcome, CheckOutcome::Success(_)) {
                        if let Some(mut dd) = world.get_mut::<Door>(target) { dd.state = DoorState::Open; }
                    }
                }
                DoorState::Locked => {
                    let outcome = strength_check(world, user, d.break_dc);
                    if matches!(outcome, CheckOutcome::Success(_)) {
                        if let Some(mut dd) = world.get_mut::<Door>(target) { dd.state = DoorState::Broken; }
                    }
                }
                DoorState::Broken => {}
            }
        }
    }
}

fn perceived_threat(world: &World, who: Entity) -> Option<Entity> {
    let p = world.get::<Perceived>(who)?;
    p.seen.iter()
        .find(|s| world.get::<Assassin>(s.entity).is_some())
        .map(|s| s.entity)
}

fn has_ranged(world: &World, actor: Entity) -> bool {
    use fortress_engine::BodySlot;
    let item = match world.get::<Wearing>(actor).and_then(|w| w.get(BodySlot::MainHand)) {
        Some(e) => e, None => return false,
    };
    world.get::<RangedWeapon>(item).is_some()
}

fn has_ammo(world: &World, actor: Entity) -> bool {
    world.get::<Inventory>(actor).map(|inv|
        inv.0.iter().any(|e| world.get::<Ammo>(*e).is_some())
    ).unwrap_or(false)
}

fn approach_tile(from: Pos, target: Pos) -> Pos {
    let dy = (from.y - target.y).signum();
    let dx = (from.x - target.x).signum();
    if dy != 0 { Pos::new(target.x, target.y + dy, target.z) }
    else if dx != 0 { Pos::new(target.x + dx, target.y, target.z) }
    else { Pos::new(target.x, target.y + 1, target.z) }
}

fn lookup_kind(world: &mut World, name: &str) -> Option<Entity> {
    let mut q = world.query::<(Entity, &Kind)>();
    q.iter(world).find(|(_, k)| k.0 == name).map(|(e, _)| e)
}

fn label_kind(world: &World, entity: Entity) -> String {
    match world.get::<Kind>(entity) {
        Some(k) => format!("{}#{}", k.0, entity.index()),
        None => format!("entity#{}", entity.index()),
    }
}

#[allow(dead_code)]
fn _silence(_: &VoxelWorld) { let _ = find_path; }
