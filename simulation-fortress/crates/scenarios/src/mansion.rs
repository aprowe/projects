//! A 60×45 luxury-mansion home invasion. The big version of the
//! `family_home` scenario:
//!
//! - Multi-wing footprint (foyer / formal living / dining / kitchen
//!   / family room / master suite / kids' wing / guest room / study /
//!   laundry) plus front porch, driveway, and back deck.
//! - Carpet, hardwood, tile, marble — mixed flooring per room.
//! - Powered furniture: TV chattering in the family room, stereo in
//!   the living room, fireplace lit in winter, stove off, fridge
//!   humming, ceiling fans whirring. Their ambient sound propagates
//!   to anyone with `Hearing`, so a sneaky burglar can pick a
//!   covered approach.
//! - Wardrobes, dressers, safes, china cabinets — all `Container`s
//!   that the burglars can rifle through.
//! - Windows: closed in the front, the back sliding glass door is
//!   slightly ajar — that's how the invaders get in.
//! - Paintings, photos, rugs scattered through the rooms.
//! - 6 family members (father, mother, teen, child, infant, elder)
//!   plus a labrador. 4 invaders (a brute and three burglars).
//!
//! All RNG flows through the seeded engine `Rng`; combat is full
//! D&D-style (d20 vs AC, dice damage, crits, manipulation checks).

use std::collections::HashSet;

use fortress_engine::actions::{fill_region_logged, note, spawn_creature};
use fortress_engine::library::{FurnitureSpawnOpts, ItemSpawnOpts};
use fortress_engine::prelude::*;
use fortress_engine::anatomy::{apply_body_plan, quadruped_body_plan};
use fortress_engine::{
    decay_coatings, derive_mood, door_voxel_sync, emit_combat_sounds, emit_movement_sounds,
    ensure_material, execute_tasks, fear_from_combat, find_path, footing_check,
    furniture_emit_system, manipulation_check, retaliation_system, spawn_furniture_template,
    spawn_humanoid_body, spawn_item_template, strength_check, tick_needs, update_hearing,
    update_sight, update_smell, CheckOutcome, Clock, Door, DoorState, Event, EventLog, Fear,
    Goal, Health, Hearing, Kind, Locomotion, Mood, Perceived, Position, Pos,
    RetaliateOnAttack, Scenario, Sight, Stats, Task, TaskQueue, Voxel, VoxelWorld,
};

// ─── footprint ─────────────────────────────────────────────────────
// Outer envelope (lawn included): x in 0..=59, y in 0..=44.
// Mansion proper: x in 2..=57, y in 7..=37.
const HOUSE_X_MIN: i32 = 2;
const HOUSE_X_MAX: i32 = 57;
const HOUSE_Y_MIN: i32 = 7;
const HOUSE_Y_MAX: i32 = 37;

// Front porch: y=6 (the line of the front facade).
const PORCH_Y: i32 = 6;
const FRONT_DOOR: Pos = Pos::new(29, PORCH_Y, 0);

// Back deck: y in HOUSE_Y_MAX+1 .. HOUSE_Y_MAX+4
const DECK_Y_MIN: i32 = HOUSE_Y_MAX + 1; // 38
const DECK_Y_MAX: i32 = 41;
// Sliding glass door sits on the back wall.
const SLIDING_DOOR: Pos = Pos::new(29, HOUSE_Y_MAX + 1, 0); // (29, 38)
const BACK_YARD_END: Pos = Pos::new(29, 44, 0);

// Inner divider walls. Two big horizontal lines split the floor
// into north (front rooms) / middle (foyer + living + bedrooms) /
// south (kitchen + family room + bedrooms behind).
const DIV_NORTH_Y: i32 = 14; // line between front rooms (foyer/dining) and middle
const DIV_SOUTH_Y: i32 = 25; // line between middle and back rooms

// Vertical dividers
const WEST_WING_X: i32 = 18;  // dining/kitchen wing east edge
const EAST_WING_X: i32 = 41;  // master/kids wing west edge

// ─── doors ──────────────────────────────────────────────────────────
const DINING_DOOR: Pos = Pos::new(WEST_WING_X, 11, 0);
const KITCHEN_DOOR: Pos = Pos::new(WEST_WING_X, 20, 0);
const PANTRY_DOOR: Pos = Pos::new(10, DIV_SOUTH_Y, 0);
const LAUNDRY_DOOR: Pos = Pos::new(15, DIV_SOUTH_Y, 0);
const GUEST_DOOR: Pos = Pos::new(WEST_WING_X, 31, 0);
const MASTER_DOOR: Pos = Pos::new(EAST_WING_X, 11, 0);
const MASTER_BATH_DOOR: Pos = Pos::new(46, 19, 0);
const MASTER_CLOSET_DOOR: Pos = Pos::new(53, 19, 0);
const KID_DOOR: Pos = Pos::new(EAST_WING_X, 30, 0);
const KID_BATH_DOOR: Pos = Pos::new(51, 31, 0);
const STUDY_DOOR: Pos = Pos::new(34, DIV_NORTH_Y, 0);
const FAMILY_DOOR: Pos = Pos::new(29, DIV_SOUTH_Y, 0);

// ─── rooms (for floor materials + spawn anchors) ───────────────────
// Each tuple: (label, x_min, x_max, y_min, y_max, floor_mat).
const ROOMS: &[(&str, i32, i32, i32, i32, &str)] = &[
    ("foyer",          19, 40, HOUSE_Y_MIN, DIV_NORTH_Y - 1, "marble"),
    ("dining_room",    HOUSE_X_MIN, WEST_WING_X - 1, HOUSE_Y_MIN, DIV_NORTH_Y - 1, "hardwood_floor"),
    ("master_bedroom", EAST_WING_X + 1, HOUSE_X_MAX, HOUSE_Y_MIN, 18, "carpet"),
    ("master_bath",    EAST_WING_X + 1, 51, 19, 24, "tile"),
    ("master_closet",  52, HOUSE_X_MAX, 19, 24, "carpet"),
    ("formal_living",  19, 33, DIV_NORTH_Y, DIV_SOUTH_Y - 1, "hardwood_floor"),
    ("study",          34, 40, DIV_NORTH_Y, DIV_SOUTH_Y - 1, "hardwood_floor"),
    ("kitchen",        HOUSE_X_MIN, WEST_WING_X - 1, DIV_NORTH_Y, DIV_SOUTH_Y - 1, "tile"),
    ("pantry",         HOUSE_X_MIN, 10, DIV_SOUTH_Y, 30, "linoleum"),
    ("laundry",        11, WEST_WING_X - 1, DIV_SOUTH_Y, 30, "linoleum"),
    ("guest_room",     HOUSE_X_MIN, WEST_WING_X - 1, 31, HOUSE_Y_MAX, "carpet"),
    ("family_room",    19, 40, DIV_SOUTH_Y, HOUSE_Y_MAX, "carpet"),
    ("kid_bedroom",    EAST_WING_X + 1, 50, 26, HOUSE_Y_MAX, "carpet"),
    ("kid_bath",       51, HOUSE_X_MAX, 26, 30, "tile"),
    ("playroom",       51, HOUSE_X_MAX, 31, HOUSE_Y_MAX, "carpet"),
];

// ─── factions ──────────────────────────────────────────────────────
const FAMILY: &str = "family";
const INVADER: &str = "invader";

#[derive(Component)] pub struct Family;
#[derive(Component)] pub struct Invader;
#[derive(Component)] pub struct Departed;
#[derive(Component)] pub struct EnteredHouse;
#[derive(Component)] pub struct Valuable;

#[derive(Default)]
pub struct MansionInvasion;

impl Scenario for MansionInvasion {
    fn name(&self) -> &str {
        "mansion home invasion"
    }

    fn setup(&mut self, world: &mut World) {
        // ─── exterior ground ────────────────────────────────────────
        let grass = ensure_material(world, "grass").expect("grass");
        fill_region_logged(
            world,
            Pos::new(-2, -2, 0),
            Pos::new(61, 46, 0),
            Voxel::floor(grass),
        );
        // Driveway + front walk
        let asphalt = ensure_material(world, "asphalt").expect("asphalt");
        fill_region_logged(
            world,
            Pos::new(27, 0, 0),
            Pos::new(31, PORCH_Y - 1, 0),
            Voxel::floor(asphalt),
        );
        let concrete = ensure_material(world, "concrete").expect("concrete");
        // Porch surface
        fill_region_logged(
            world,
            Pos::new(25, PORCH_Y, 0),
            Pos::new(33, PORCH_Y, 0),
            Voxel::floor(concrete),
        );
        // Back deck
        let deck = ensure_material(world, "wood").expect("wood");
        fill_region_logged(
            world,
            Pos::new(20, DECK_Y_MIN, 0),
            Pos::new(38, DECK_Y_MAX, 0),
            Voxel::floor(deck),
        );

        // ─── room floors ────────────────────────────────────────────
        for (_, x_min, x_max, y_min, y_max, floor_mat) in ROOMS {
            let mat = ensure_material(world, floor_mat)
                .unwrap_or_else(|_| ensure_material(world, "wood").expect("wood fallback"));
            fill_region_logged(
                world,
                Pos::new(*x_min, *y_min, 0),
                Pos::new(*x_max, *y_max, 0),
                Voxel::floor(mat),
            );
        }

        // ─── exterior + interior walls ──────────────────────────────
        let drywall = ensure_material(world, "drywall").expect("drywall");
        let brick = ensure_material(world, "brick").expect("brick");
        let outer = Voxel::wall(brick);
        let inner = Voxel::wall(drywall);

        let door_set: HashSet<Pos> = [
            FRONT_DOOR, SLIDING_DOOR, DINING_DOOR, KITCHEN_DOOR, PANTRY_DOOR, LAUNDRY_DOOR,
            GUEST_DOOR, MASTER_DOOR, MASTER_BATH_DOOR, MASTER_CLOSET_DOOR, KID_DOOR,
            KID_BATH_DOOR, STUDY_DOOR, FAMILY_DOOR,
        ]
        .into_iter()
        .collect();

        {
            let mut vw = world.resource_mut::<VoxelWorld>();
            // Outer envelope: brick walls at x = HOUSE_X_MIN-1, x = HOUSE_X_MAX+1,
            // y = HOUSE_Y_MIN-1 (front), y = HOUSE_Y_MAX+1 (back).
            for x in (HOUSE_X_MIN - 1)..=(HOUSE_X_MAX + 1) {
                let front = Pos::new(x, HOUSE_Y_MIN - 1, 0);
                if !door_set.contains(&front) {
                    vw.set_voxel(front, outer);
                }
                let back = Pos::new(x, HOUSE_Y_MAX + 1, 0);
                if !door_set.contains(&back) {
                    vw.set_voxel(back, outer);
                }
            }
            for y in HOUSE_Y_MIN..=HOUSE_Y_MAX {
                vw.set_voxel(Pos::new(HOUSE_X_MIN - 1, y, 0), outer);
                vw.set_voxel(Pos::new(HOUSE_X_MAX + 1, y, 0), outer);
            }

            // Interior dividers
            // Two horizontal lines (DIV_NORTH_Y, DIV_SOUTH_Y)
            for y in [DIV_NORTH_Y, DIV_SOUTH_Y] {
                for x in HOUSE_X_MIN..=HOUSE_X_MAX {
                    let p = Pos::new(x, y, 0);
                    if !door_set.contains(&p) {
                        vw.set_voxel(p, inner);
                    }
                }
            }
            // Two vertical lines (WEST_WING_X, EAST_WING_X)
            for x in [WEST_WING_X, EAST_WING_X] {
                for y in HOUSE_Y_MIN..=HOUSE_Y_MAX {
                    let p = Pos::new(x, y, 0);
                    if !door_set.contains(&p) {
                        vw.set_voxel(p, inner);
                    }
                }
            }

            // Master suite sub-divisions
            // - vertical wall at x=51 from y=19 to y=24 (master closet)
            for y in 19..=24 {
                let p = Pos::new(51, y, 0);
                if !door_set.contains(&p) {
                    vw.set_voxel(p, inner);
                }
            }
            // - horizontal at y=18 (master bed -> master bath)
            for x in (EAST_WING_X + 1)..=HOUSE_X_MAX {
                let p = Pos::new(x, 18, 0);
                if !door_set.contains(&p) {
                    vw.set_voxel(p, inner);
                }
            }
            // - horizontal at y=24 (master bath/closet -> kids hallway)
            for x in (EAST_WING_X + 1)..=HOUSE_X_MAX {
                let p = Pos::new(x, 24, 0);
                if !door_set.contains(&p) {
                    vw.set_voxel(p, inner);
                }
            }
            // - horizontal at y=25 (kids hallway -> kid bedroom/bath)
            for x in (EAST_WING_X + 1)..=HOUSE_X_MAX {
                let p = Pos::new(x, 25, 0);
                if !door_set.contains(&p) {
                    vw.set_voxel(p, inner);
                }
            }
            // - vertical at x=51 from y=26 to y=30 (kid_bath separator)
            for y in 26..=30 {
                let p = Pos::new(51, y, 0);
                if !door_set.contains(&p) {
                    vw.set_voxel(p, inner);
                }
            }
            // - horizontal at y=30 inside east wing (kid bath ceiling vs playroom)
            for x in 52..=HOUSE_X_MAX {
                let p = Pos::new(x, 30, 0);
                if !door_set.contains(&p) {
                    vw.set_voxel(p, inner);
                }
            }

            // Pantry/laundry split at x=11 from y=DIV_SOUTH_Y..=30
            for y in (DIV_SOUTH_Y + 1)..=30 {
                let p = Pos::new(11, y, 0);
                if !door_set.contains(&p) {
                    vw.set_voxel(p, inner);
                }
            }
            // Pantry/laundry south wall at y=31 (separates from guest)
            for x in HOUSE_X_MIN..=(WEST_WING_X - 1) {
                let p = Pos::new(x, 31, 0);
                if !door_set.contains(&p) {
                    vw.set_voxel(p, inner);
                }
            }

            // Study/living split at x=34 from DIV_NORTH_Y..=DIV_SOUTH_Y
            for y in (DIV_NORTH_Y + 1)..=(DIV_SOUTH_Y - 1) {
                let p = Pos::new(34, y, 0);
                if !door_set.contains(&p) {
                    vw.set_voxel(p, inner);
                }
            }

            // Foyer side walls — keep narrow corridor through to living
            // (actually foyer opens directly into living through 14-y line)
        }

        note(
            world,
            "An estate at the end of a long driveway. A two-story brick mansion, lights on in the kitchen and family room, soft music drifting through the open back door.",
        );

        // ─── doors ──────────────────────────────────────────────────
        let oak = ensure_material(world, "oak").expect("oak");
        spawn_door(world, FRONT_DOOR, oak, "front door", DoorKind::Closed);
        spawn_door(world, DINING_DOOR, oak, "dining room door", DoorKind::Open);
        spawn_door(world, KITCHEN_DOOR, oak, "kitchen door", DoorKind::Open);
        spawn_door(world, PANTRY_DOOR, oak, "pantry door", DoorKind::Closed);
        spawn_door(world, LAUNDRY_DOOR, oak, "laundry door", DoorKind::Closed);
        spawn_door(world, GUEST_DOOR, oak, "guest room door", DoorKind::Closed);
        spawn_door(world, MASTER_DOOR, oak, "master bedroom door", DoorKind::Closed);
        spawn_door(world, MASTER_BATH_DOOR, oak, "master bath door", DoorKind::Open);
        spawn_door(world, MASTER_CLOSET_DOOR, oak, "master closet door", DoorKind::Closed);
        spawn_door(world, KID_DOOR, oak, "kid's bedroom door", DoorKind::Locked(14));
        spawn_door(world, KID_BATH_DOOR, oak, "kid's bath door", DoorKind::Open);
        spawn_door(world, STUDY_DOOR, oak, "study door", DoorKind::Open);
        spawn_door(world, FAMILY_DOOR, oak, "family room door", DoorKind::Open);

        // ─── windows ────────────────────────────────────────────────
        // Front facade has windows flanking the front door
        for &(x, label) in &[(8, "kitchen window"), (14, "dining window"),
                              (22, "foyer window"), (36, "study window"),
                              (44, "master bedroom window"), (54, "master closet window")] {
            spawn_window(world, Pos::new(x, HOUSE_Y_MIN - 1, 0), label, true);
        }
        // Back facade
        for &(x, label) in &[(8, "kitchen back window"), (15, "laundry window"),
                              (44, "kid bedroom window"), (54, "playroom window")] {
            spawn_window(world, Pos::new(x, HOUSE_Y_MAX + 1, 0), label, true);
        }
        // The sliding glass door — left ajar! This is the breach point.
        let _slider = spawn_furniture_template(
            world,
            "open sliding door",
            FurnitureSpawnOpts {
                at: SLIDING_DOOR,
                kind_label: Some("sliding glass door".into()),
            },
        );

        // ─── rugs (room dressing first) ─────────────────────────────
        place(world, "persian rug", Pos::new(28, 17, 0));
        place(world, "persian rug", Pos::new(26, 30, 0));
        place(world, "kitchen mat", Pos::new(7, 22, 0));
        place(world, "bath mat",   Pos::new(46, 22, 0));

        // ─── foyer ──────────────────────────────────────────────────
        place(world, "chandelier", Pos::new(29, 10, 0));
        place(world, "bench",      Pos::new(22, 9, 0));
        place(world, "side table", Pos::new(36, 9, 0));
        place(world, "vase",       Pos::new(36, 8, 0));
        place(world, "potted plant", Pos::new(22, 13, 0));
        place(world, "wall clock", Pos::new(29, 8, 0));
        place_painting(world, "oil portrait", Pos::new(33, 8, 0));

        // ─── dining room ────────────────────────────────────────────
        place(world, "dining table", Pos::new(10, 11, 0));
        for (x, y) in [(8, 10), (12, 10), (8, 12), (12, 12), (10, 9), (10, 13)] {
            place(world, "dining chair", Pos::new(x, y, 0));
        }
        place(world, "china cabinet", Pos::new(4, 9, 0));
        place(world, "chandelier",    Pos::new(10, 11, 0));
        place_painting(world, "abstract canvas", Pos::new(15, 8, 0));

        // ─── kitchen ────────────────────────────────────────────────
        place(world, "refrigerator",  Pos::new(4, 16, 0));
        place_powered(world, "stove on", Pos::new(4, 19, 0), "gas stove (front-left burner on)");
        place(world, "microwave",     Pos::new(4, 21, 0));
        place(world, "dishwasher",    Pos::new(4, 23, 0));
        place(world, "kitchen sink",  Pos::new(15, 16, 0));
        place(world, "kitchen island", Pos::new(10, 19, 0));
        place(world, "dining chair",  Pos::new(9, 18, 0));
        place(world, "dining chair",  Pos::new(11, 18, 0));
        place(world, "ceiling fan",   Pos::new(10, 22, 0));
        place(world, "potted plant",  Pos::new(15, 23, 0));

        // ─── pantry ─────────────────────────────────────────────────
        for y in 26..=29 {
            place(world, "bookshelf", Pos::new(2, y, 0));
        }

        // ─── laundry ────────────────────────────────────────────────
        place(world, "washer", Pos::new(12, 26, 0));
        place(world, "dryer",  Pos::new(14, 26, 0));
        place(world, "filing cabinet", Pos::new(17, 26, 0));

        // ─── formal living room ─────────────────────────────────────
        place(world, "sofa",       Pos::new(25, 18, 0));
        place(world, "sofa",       Pos::new(31, 18, 0));
        place(world, "armchair",   Pos::new(22, 22, 0));
        place(world, "armchair",   Pos::new(33, 22, 0));
        place(world, "coffee table", Pos::new(28, 20, 0));
        place(world, "fireplace",  Pos::new(29, 24, 0));
        place_powered(world, "stereo", Pos::new(20, 16, 0), "Marantz stereo (vinyl playing)");
        place(world, "floor lamp", Pos::new(20, 22, 0));
        place(world, "floor lamp", Pos::new(33, 17, 0));
        place_painting(world, "abstract canvas", Pos::new(28, 15, 0));
        place_painting(world, "painting", Pos::new(20, 19, 0));
        place(world, "potted plant", Pos::new(33, 24, 0));

        // ─── study ──────────────────────────────────────────────────
        place(world, "desk",       Pos::new(37, 17, 0));
        place(world, "armchair",   Pos::new(37, 19, 0));
        place(world, "bookshelf",  Pos::new(35, 15, 0));
        place(world, "bookshelf",  Pos::new(36, 15, 0));
        place(world, "books",      Pos::new(37, 15, 0));
        place(world, "table lamp", Pos::new(37, 16, 0));
        place_container(
            world, "safe", Pos::new(40, 23, 0),
            &["gold pocket watch", "diamond ring", "stack of cash"],
        );
        place_painting(world, "oil portrait", Pos::new(35, 16, 0));

        // ─── master bedroom ─────────────────────────────────────────
        place(world, "king bed",   Pos::new(48, 9, 0));
        place(world, "nightstand", Pos::new(46, 9, 0));
        place(world, "nightstand", Pos::new(50, 9, 0));
        place(world, "table lamp", Pos::new(46, 10, 0));
        place(world, "table lamp", Pos::new(50, 10, 0));
        place(world, "dresser",    Pos::new(54, 12, 0));
        place_container(
            world, "wardrobe", Pos::new(42, 9, 0),
            &["leather jacket", "wool shirt"],
        );
        place_container(
            world, "locked dresser", Pos::new(43, 16, 0),
            &["emerald necklace", "silver bracelet"],
        );
        place(world, "ceiling fan", Pos::new(48, 13, 0));
        place(world, "armchair",    Pos::new(54, 15, 0));
        place(world, "ottoman",     Pos::new(54, 16, 0));
        place_painting(world, "painting", Pos::new(48, 8, 0));

        // ─── master bath ────────────────────────────────────────────
        place(world, "bathtub",        Pos::new(43, 21, 0));
        place(world, "shower",         Pos::new(46, 21, 0));
        place(world, "toilet",         Pos::new(50, 23, 0));
        place(world, "bathroom sink",  Pos::new(43, 23, 0));
        place(world, "mirror",         Pos::new(43, 24, 0));

        // ─── master closet ──────────────────────────────────────────
        for x in 53..=55 {
            place_container(world, "wardrobe", Pos::new(x, 21, 0), &["wool shirt"]);
        }
        place(world, "wall clock", Pos::new(56, 21, 0));

        // ─── kid bedroom ────────────────────────────────────────────
        place(world, "twin bed",     Pos::new(43, 28, 0));
        place(world, "twin bed",     Pos::new(48, 28, 0));
        place(world, "nightstand",   Pos::new(45, 28, 0));
        place(world, "table lamp",   Pos::new(45, 29, 0));
        place(world, "dresser",      Pos::new(50, 36, 0));
        place(world, "bookshelf",    Pos::new(43, 36, 0));
        place(world, "books",        Pos::new(44, 36, 0));
        place(world, "potted plant", Pos::new(48, 36, 0));
        place_painting(world, "photograph", Pos::new(46, 27, 0));

        // ─── kid bath ───────────────────────────────────────────────
        place(world, "toilet",        Pos::new(53, 27, 0));
        place(world, "bathroom sink", Pos::new(55, 27, 0));
        place(world, "shower",        Pos::new(53, 29, 0));

        // ─── playroom (kid's wing) ──────────────────────────────────
        place(world, "sofa",       Pos::new(54, 33, 0));
        place_powered(world, "tv set", Pos::new(54, 31, 0), "playroom TV (cartoons)");
        place(world, "books",      Pos::new(53, 36, 0));
        place(world, "ottoman",    Pos::new(56, 33, 0));
        place(world, "potted plant", Pos::new(56, 36, 0));

        // ─── family room (rear of foyer/living wall) ────────────────
        place(world, "sofa",        Pos::new(25, 28, 0));
        place(world, "sofa",        Pos::new(33, 28, 0));
        place(world, "recliner",    Pos::new(20, 30, 0));
        place(world, "armchair",    Pos::new(38, 30, 0));
        place(world, "coffee table", Pos::new(29, 30, 0));
        place_powered(world, "tv set", Pos::new(29, 26, 0), "family room TV (sports)");
        place(world, "ceiling fan", Pos::new(29, 33, 0));
        place(world, "table lamp",  Pos::new(20, 28, 0));
        place(world, "table lamp",  Pos::new(38, 28, 0));
        place_painting(world, "abstract canvas", Pos::new(25, 35, 0));
        place_painting(world, "photograph", Pos::new(33, 35, 0));
        place(world, "potted plant", Pos::new(20, 36, 0));
        place(world, "potted plant", Pos::new(38, 36, 0));

        // ─── guest room ─────────────────────────────────────────────
        place(world, "queen bed",   Pos::new(8, 34, 0));
        place(world, "nightstand",  Pos::new(6, 34, 0));
        place(world, "table lamp",  Pos::new(6, 35, 0));
        place(world, "wardrobe",    Pos::new(15, 35, 0));
        place(world, "dresser",     Pos::new(15, 36, 0));
        place_painting(world, "painting", Pos::new(11, 32, 0));

        // ─── exterior dressing ──────────────────────────────────────
        place(world, "mailbox",       Pos::new(33, 0, 0));
        place(world, "garden gnome",  Pos::new(5, 5, 0));
        place(world, "garden gnome",  Pos::new(55, 5, 0));
        place(world, "potted plant",  Pos::new(25, PORCH_Y, 0));
        place(world, "potted plant",  Pos::new(33, PORCH_Y, 0));
        // Back deck
        place(world, "grill",         Pos::new(22, DECK_Y_MIN + 1, 0));
        place(world, "patio table",   Pos::new(29, DECK_Y_MIN + 1, 0));
        place(world, "patio chair",   Pos::new(28, DECK_Y_MIN + 2, 0));
        place(world, "patio chair",   Pos::new(30, DECK_Y_MIN + 2, 0));
        place(world, "hammock",       Pos::new(36, DECK_Y_MIN + 2, 0));
        place(world, "pool",          Pos::new(45, DECK_Y_MAX + 2, 0));
        place(world, "hot tub",       Pos::new(15, DECK_Y_MAX + 2, 0));

        // ─── Family ─────────────────────────────────────────────────
        // Father on the family-room sofa watching TV; mother in
        // kitchen cooking; teen in playroom; child in kid bedroom;
        // infant in master; elder in guest room. Lab dog at the
        // back of the family room rug.
        let father = humanoid(world, "father", Pos::new(31, 28, 0), FAMILY, 100,
            Stats { str_: 14, dex: 11, con: 13, int: 11, wis: 11, cha: 11 });
        world.entity_mut(father).insert(Family).insert(RetaliateOnAttack);
        equip(world, father, &["fire poker", "leather jacket", "leather boots"]);

        let mother = humanoid(world, "mother", Pos::new(10, 22, 0), FAMILY, 80,
            Stats { str_: 11, dex: 13, con: 12, int: 13, wis: 13, cha: 12 });
        world.entity_mut(mother).insert(Family).insert(RetaliateOnAttack);
        equip(world, mother, &["wool shirt", "leather boots", "kitchen knife"]);

        let teen = humanoid(world, "teen", Pos::new(54, 33, 0), FAMILY, 60,
            Stats { str_: 10, dex: 14, con: 11, int: 12, wis: 9, cha: 12 });
        world.entity_mut(teen).insert(Family).insert(RetaliateOnAttack);
        equip(world, teen, &["hoodie", "rubber boots", "baseball bat"]);

        let child = humanoid(world, "child", Pos::new(47, 30, 0), FAMILY, 30, Stats::child());
        world.entity_mut(child).insert(Family).insert(Locomotion::Sneaking);
        equip(world, child, &["cotton t-shirt", "rubber boots"]);

        let infant = humanoid(world, "infant", Pos::new(52, 12, 0), FAMILY, 15, Stats::child());
        world.entity_mut(infant).insert(Family).insert(Locomotion::Sneaking);
        equip(world, infant, &["cotton t-shirt"]);

        let elder = humanoid(world, "elder", Pos::new(8, 34, 0), FAMILY, 50, Stats::elder());
        world.entity_mut(elder).insert(Family);
        equip(world, elder, &["wool shirt", "leather boots"]);

        let lab = spawn_creature(world, "labrador", Pos::new(29, 30, 0), 60, Some(FAMILY));
        let plan = quadruped_body_plan();
        apply_body_plan(world, lab, &plan);
        world
            .entity_mut(lab)
            .insert(Stats::rogue())
            .insert(TaskQueue::default())
            .insert(Goal::default())
            .insert(Mood::default())
            .insert(Fear::calm())
            .insert(Sight::normal())
            .insert(Hearing::normal())
            .insert(Perceived::default())
            .insert(Family)
            .insert(RetaliateOnAttack);

        // ─── Invaders (entering through the open sliding glass door) ─
        // Stage them on the back deck, just outside.
        let brute = humanoid(world, "brute", Pos::new(28, DECK_Y_MIN + 2, 0), INVADER, 130, Stats::brute());
        world.entity_mut(brute).insert(Invader);
        equip(world, brute, &["leather jacket", "leather boots", "steel crowbar"]);

        let burglar_a = humanoid(world, "burglar_a", Pos::new(30, DECK_Y_MIN + 2, 0), INVADER, 90, Stats::rogue());
        world.entity_mut(burglar_a).insert(Invader);
        equip(world, burglar_a, &["hoodie", "rubber boots", "hunting knife"]);

        let burglar_b = humanoid(world, "burglar_b", Pos::new(29, DECK_Y_MIN + 3, 0), INVADER, 85, Stats::rogue());
        world.entity_mut(burglar_b).insert(Invader);
        equip(world, burglar_b, &["hoodie", "rubber boots", "brass candlestick"]);

        let burglar_c = humanoid(world, "burglar_c", Pos::new(27, DECK_Y_MIN + 3, 0), INVADER, 85, Stats::rogue());
        world.entity_mut(burglar_c).insert(Invader);
        equip(world, burglar_c, &["hoodie", "leather boots", "frying pan"]);

        // Mark valuables (paintings + safe contents) as targets
        mark_valuables(world);

        note(
            world,
            "Four figures step over the deck railing: a slab of muscle with a crowbar, two rogues with knives, and a fourth carrying a frying pan like a club. The sliding glass door is already open — the family didn't lock up.",
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
                furniture_emit_system,
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

// ─── building helpers ──────────────────────────────────────────────

#[derive(Copy, Clone, Debug)]
enum DoorKind {
    Open,
    Closed,
    Locked(i32),
}

fn spawn_door(world: &mut World, pos: Pos, mat: u16, label: &str, kind: DoorKind) {
    let door = match kind {
        DoorKind::Open => {
            let mut d = Door::closed(mat, label);
            d.state = DoorState::Open;
            d
        }
        DoorKind::Closed => Door::closed(mat, label),
        DoorKind::Locked(dc) => Door::locked(mat, label, dc),
    };
    world.spawn((Position(pos), Kind(label.into()), door));
}

fn spawn_window(world: &mut World, pos: Pos, label: &str, closed: bool) {
    // Override the wall voxel with glass so the window is visible.
    let glass = ensure_material(world, "glass").expect("glass");
    {
        let mut vw = world.resource_mut::<VoxelWorld>();
        // closed window = wall (impassable but visible); open = floor
        if closed {
            vw.set_voxel(pos, Voxel::wall(glass));
        } else {
            vw.set_voxel(pos, Voxel::floor(glass));
        }
    }
    let template = if closed { "window" } else { "open window" };
    let _ = spawn_furniture_template(
        world,
        template,
        FurnitureSpawnOpts {
            at: pos,
            kind_label: Some(label.into()),
        },
    );
}

fn place(world: &mut World, template: &str, pos: Pos) {
    let _ = spawn_furniture_template(
        world,
        template,
        FurnitureSpawnOpts { at: pos, kind_label: None },
    );
}

fn place_powered(world: &mut World, template: &str, pos: Pos, label: &str) {
    let _ = spawn_furniture_template(
        world,
        template,
        FurnitureSpawnOpts { at: pos, kind_label: Some(label.into()) },
    );
}

fn place_painting(world: &mut World, template: &str, pos: Pos) {
    let _ = spawn_furniture_template(
        world,
        template,
        FurnitureSpawnOpts { at: pos, kind_label: None },
    );
}

fn place_container(world: &mut World, template: &str, pos: Pos, contents: &[&str]) {
    let container = match spawn_furniture_template(
        world,
        template,
        FurnitureSpawnOpts { at: pos, kind_label: None },
    ) {
        Ok(e) => e,
        Err(_) => return,
    };
    // Spawn each item near the container as flavor — mark as Valuable
    // so the invader planner picks them up. Real `Container` plumbing
    // (open it, transfer items) is left for a future iteration; the
    // proxy is "items sit on the same tile as the container".
    for &item_name in contents {
        // Use the first available library item for the named valuable;
        // if none matches, fall back to "kitchen knife" with override.
        let label = item_name.to_string();
        let template_name = if world.resource::<fortress_engine::Library>().items.contains_key(item_name) {
            item_name.to_string()
        } else {
            "kitchen knife".to_string()
        };
        let opts = ItemSpawnOpts {
            at: Some(pos),
            override_label: Some(label),
            ..Default::default()
        };
        if let Ok(item) = spawn_item_template(world, &template_name, opts) {
            world.entity_mut(item).insert(Valuable);
        }
    }
    let _ = container;
}

fn humanoid(
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

fn equip(world: &mut World, who: Entity, items: &[&str]) {
    for item in items {
        let _ = spawn_item_template(
            world,
            item,
            ItemSpawnOpts { equip_on: Some(who), ..Default::default() },
        );
    }
}

fn mark_valuables(world: &mut World) {
    use fortress_engine::Painting;
    // All paintings worth >= 1000 are "valuables" — burglars target them.
    let paintings: Vec<Entity> = {
        let mut q = world.query::<(Entity, &Painting)>();
        q.iter(world)
            .filter(|(_, p)| p.value_currency >= 1000)
            .map(|(e, _)| e)
            .collect()
    };
    for e in paintings {
        world.entity_mut(e).insert(Valuable);
    }
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
    // Wake state: when a family member sees an invader or hears a
    // violent noise, they panic — the kid hides under the bed (well,
    // stays put, sneaking), the parents look for the threat.
    let alerts: Vec<(Entity, Option<Pos>, Option<Entity>)> = {
        let mut q = world.query_filtered::<(Entity, &Perceived, &TaskQueue), With<Family>>();
        q.iter(world)
            .filter(|(_, _, q)| q.is_empty())
            .map(|(e, p, _)| {
                let visible_invader = p.seen.iter().min_by_key(|s| s.distance).map(|s| s.entity);
                let noise = p.loudest_violent().map(|hs| hs.origin);
                (e, noise, visible_invader)
            })
            .collect()
    };
    for (member, noise, sight) in alerts {
        if sight.is_none() && noise.is_none() {
            continue;
        }
        if let Some(target) = sight {
            // Only attack if the seen entity is an invader.
            let is_invader = world.get::<Invader>(target).is_some();
            if is_invader {
                if let Some(mut q) = world.get_mut::<TaskQueue>(member) {
                    q.push(Task::Attack(target));
                }
                if let Some(mut g) = world.get_mut::<Goal>(member) {
                    *g = Goal::Kill(target);
                }
                world.entity_mut(member).insert(Locomotion::Running);
                continue;
            }
        }
        if let Some(origin) = noise {
            if let Some(mut q) = world.get_mut::<TaskQueue>(member) {
                q.push(Task::MoveTo(origin));
            }
            if let Some(mut g) = world.get_mut::<Goal>(member) {
                *g = Goal::GoTo(origin);
            }
        }
    }
}

fn invader_planner(world: &mut World) {
    let invader_ids: HashSet<Entity> = {
        let mut q = world.query_filtered::<Entity, (With<Invader>, Without<Departed>)>();
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

    let closed_doors: Vec<(Entity, Pos)> = {
        let mut q = world.query::<(Entity, &Position, &Door)>();
        q.iter(world)
            .filter(|(_, _, d)| !d.state.is_passable())
            .map(|(e, p, _)| (e, p.0))
            .collect()
    };

    for (invader, pos, goal, queue_empty) in invaders {
        let alive = world.get::<Health>(invader).map(|h| h.is_alive()).unwrap_or(false);
        if !alive {
            continue;
        }

        // Step 1: visible target
        let visible_target = world.get::<Perceived>(invader).and_then(|p| {
            p.seen
                .iter()
                .filter(|s| !invader_ids.contains(&s.entity))
                .filter(|s| {
                    world.get::<Health>(s.entity).map(|h| h.is_alive()).unwrap_or(false)
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

        // Step 2: heard violent noise
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

        // Step 3: nearest reachable valuable. Sort by manhattan
        // distance and take the first one we can actually pathfind
        // to (avoids spinning forever on a valuable behind a locked
        // door).
        let mut sorted_loot: Vec<(Entity, Pos)> = valuables.clone();
        sorted_loot.sort_by_key(|(_, vp)| pos.manhattan(*vp));
        let target_loot: Option<(Entity, Pos)> = {
            let vw = world.resource::<VoxelWorld>();
            sorted_loot
                .into_iter()
                .find(|(_, vp)| find_path(vw, pos, *vp, 4096).is_some())
        };

        let primary_dest = target_loot
            .map(|(_, p)| p)
            .unwrap_or(BACK_YARD_END);

        let reachable = {
            let vw = world.resource::<VoxelWorld>();
            find_path(vw, pos, primary_dest, 4096).is_some()
        };

        if !reachable {
            let door_choice = closed_doors
                .iter()
                .copied()
                .filter_map(|(door, door_pos)| {
                    let vw = world.resource::<VoxelWorld>();
                    let approach = approach_tile(pos, door_pos);
                    find_path(vw, pos, approach, 4096).map(|_| (door, door_pos, pos.manhattan(door_pos)))
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

        let leaving = matches!(goal, Goal::GoTo(p) if p == BACK_YARD_END);
        if !leaving || queue_empty {
            if let Some(mut q) = world.get_mut::<TaskQueue>(invader) {
                q.clear();
                q.push(Task::MoveTo(BACK_YARD_END));
            }
            if let Some(mut g) = world.get_mut::<Goal>(invader) {
                *g = Goal::GoTo(BACK_YARD_END);
            }
            world.entity_mut(invader).insert(Locomotion::Walking);
        }
    }
}

fn approach_tile(from: Pos, door_pos: Pos) -> Pos {
    let dy = (from.y - door_pos.y).signum();
    let dx = (from.x - door_pos.x).signum();
    if dy != 0 {
        Pos::new(door_pos.x, door_pos.y + dy, door_pos.z)
    } else if dx != 0 {
        Pos::new(door_pos.x + dx, door_pos.y, door_pos.z)
    } else {
        Pos::new(door_pos.x, door_pos.y + 1, door_pos.z)
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
        let door_data = world
            .get::<Door>(target)
            .map(|d| (d.state, d.lock_dc, d.break_dc, d.label.clone()));
        let (state, lock_dc, break_dc, label) = match door_data {
            Some(d) => d,
            None => continue,
        };

        match state {
            DoorState::Open => {}
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
                    let _ = lock_dc;
                }
            }
            DoorState::Broken => {}
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
            .filter(|(_, p)| p.0.y >= BACK_YARD_END.y || p.0.y < HOUSE_Y_MIN - 1)
            .map(|(e, _)| e)
            .collect()
    };
    for invader in arrivals {
        world.entity_mut(invader).insert(Departed);
        let label = label_kind(world, invader);
        let tick = world.resource::<Clock>().tick;
        world.resource_mut::<EventLog>().push(
            tick,
            Event::Note(format!("{label} disappears into the night.")),
        );
    }
}

fn label_kind(world: &World, entity: Entity) -> String {
    match world.get::<Kind>(entity) {
        Some(k) => format!("{}#{}", k.0, entity.index()),
        None => format!("entity#{}", entity.index()),
    }
}

