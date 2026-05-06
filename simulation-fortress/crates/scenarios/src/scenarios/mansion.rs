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

use fortress_engine::furniture::Container;
use fortress_engine::{
    line_of_sight_blocked, Blemish, BlemishKind, Blemishes, Finish, Paint, Style,
};

use fortress_engine::actions::{fill_region_logged, note, spawn_creature};
use fortress_engine::library::{FurnitureSpawnOpts, ItemSpawnOpts};
use fortress_engine::prelude::*;
use fortress_engine::anatomy::{apply_body_plan, quadruped_body_plan};
use fortress_engine::{
    decay_coatings, derive_mood, door_voxel_sync, emit_combat_sounds, emit_movement_sounds,
    ensure_material, execute_tasks, fear_from_combat, find_path, footing_check,
    furniture_emit_system, manipulation_check, retaliation_system, spawn_furniture_template,
    tick_carrying, tick_fire, tick_schedules, tick_status_effects,
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

// ─── upstairs (z=1) ────────────────────────────────────────────────
// Smaller footprint: just the central rooms above the foyer.
const UP_X_MIN: i32 = 19;
const UP_X_MAX: i32 = 40;
const UP_Y_MIN: i32 = HOUSE_Y_MIN;
const UP_Y_MAX: i32 = 24;
const UP_DIVIDER_Y: i32 = 16;            // splits north / south upstairs rooms
const STAIR_FOOT: Pos = Pos::new(28, 13, 0); // bottom of grand staircase
const UP_STUDY_DOOR: Pos = Pos::new(29, UP_DIVIDER_Y, 1);

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

/// What an individual invader has *personally* observed: a set of
/// loot entities they've seen on the floor, and a set of unopened
/// containers they've spotted. The planner only targets things in
/// these sets — they don't have global oracle knowledge.
#[derive(Component, Default, Debug)]
pub struct LooterMemory {
    pub known_loot: HashSet<Entity>,
    pub known_containers: HashSet<Entity>,
    /// Rooms (by index into ROOMS) the invader has already swept.
    pub explored_rooms: HashSet<usize>,
    /// Which room the invader is currently searching, if any.
    pub current_search: Option<usize>,
}

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

            // ─── upstairs (z = 1) ───────────────────────────────────
            // Floor over the central rooms.
            let upstairs_carpet = vw.material_id("carpet").unwrap_or(0);
            let upstairs_wood = vw.material_id("hardwood_floor").unwrap_or(0);
            for y in UP_Y_MIN..=UP_Y_MAX {
                for x in UP_X_MIN..=UP_X_MAX {
                    // South of the divider = wood (upstairs hall + study);
                    // North = bedrooms in carpet.
                    let mat = if y >= UP_DIVIDER_Y { upstairs_wood } else { upstairs_carpet };
                    vw.set_voxel(Pos::new(x, y, 1), Voxel::floor(mat));
                }
            }
            // Outer walls (brick).
            let up_outer = Voxel::wall(brick);
            for x in (UP_X_MIN - 1)..=(UP_X_MAX + 1) {
                vw.set_voxel(Pos::new(x, UP_Y_MIN - 1, 1), up_outer);
                vw.set_voxel(Pos::new(x, UP_Y_MAX + 1, 1), up_outer);
            }
            for y in UP_Y_MIN..=UP_Y_MAX {
                vw.set_voxel(Pos::new(UP_X_MIN - 1, y, 1), up_outer);
                vw.set_voxel(Pos::new(UP_X_MAX + 1, y, 1), up_outer);
            }
            // Inner divider at y=UP_DIVIDER_Y, with a doorway gap.
            for x in UP_X_MIN..=UP_X_MAX {
                let p = Pos::new(x, UP_DIVIDER_Y, 1);
                if p != UP_STUDY_DOOR {
                    vw.set_voxel(p, Voxel::wall(drywall));
                }
            }
            // Vertical wall splitting the two upstairs bedrooms at x=29.
            for y in UP_Y_MIN..(UP_DIVIDER_Y) {
                let p = Pos::new(29, y, 1);
                vw.set_voxel(p, Voxel::wall(drywall));
            }
            // Doorways from the upstairs hallway into each bedroom.
            vw.set_voxel(Pos::new(24, UP_DIVIDER_Y - 1, 1), Voxel::floor(upstairs_carpet));
            vw.set_voxel(Pos::new(34, UP_DIVIDER_Y - 1, 1), Voxel::floor(upstairs_carpet));
        }

        note(
            world,
            "An estate at the end of a long driveway. A two-story brick mansion: lights on in the kitchen and family room, soft music drifting through the open back door, and a single light in an upstairs bedroom.",
        );

        // ─── doors ──────────────────────────────────────────────────
        let oak = ensure_material(world, "oak").expect("oak");
        let front_door = spawn_door_returning(world, FRONT_DOOR, oak, "front door", DoorKind::Closed);
        // The front door is painted glossy red.
        world.entity_mut(front_door)
            .insert(Paint::rgb(160, 30, 30))
            .insert(Finish::Glossy)
            .insert(Style::Victorian);
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
        // Grand staircase — anchor at z=0 stamps a RampUp; the
        // matching tile at z=1 becomes Empty so an actor can land
        // there and walk into the upstairs hall.
        place(world, "staircase up", STAIR_FOOT);
        place(world, "bench",      Pos::new(22, 9, 0));
        place(world, "side table", Pos::new(36, 9, 0));
        place(world, "vase",       Pos::new(36, 8, 0));
        place(world, "potted plant", Pos::new(22, 13, 0));
        place(world, "wall clock", Pos::new(29, 8, 0));
        place_painting(world, "oil portrait", Pos::new(33, 8, 0));

        // ─── dining room (table is 2x4: occupies x=9..10, y=9..12) ──
        place(world, "dining table", Pos::new(9, 9, 0));
        for (x, y) in [(7, 9), (12, 9), (7, 11), (12, 11)] {
            place(world, "dining chair", Pos::new(x, y, 0));
        }
        place(world, "china cabinet", Pos::new(4, 8, 0));
        place(world, "chandelier",    Pos::new(10, 13, 0));
        let dining_painting = spawn_furniture_template(
            world,
            "abstract canvas",
            FurnitureSpawnOpts { at: Pos::new(15, 8, 0), kind_label: None },
        ).expect("abstract canvas");
        // The dining painting has a small dent in the frame and a
        // sun-faded patch in the upper corner.
        world.entity_mut(dining_painting)
            .insert(Blemishes(vec![
                Blemish::new(BlemishKind::Dent, "in the gilded frame", 1),
                Blemish::new(BlemishKind::SunFaded, "upper corner", 2),
            ]));

        // ─── kitchen ────────────────────────────────────────────────
        place(world, "refrigerator",  Pos::new(4, 16, 0));
        place_powered(world, "stove on", Pos::new(4, 19, 0), "gas stove (front-left burner on)");
        place(world, "microwave",     Pos::new(4, 21, 0));
        place(world, "dishwasher",    Pos::new(4, 23, 0));
        place(world, "kitchen sink",  Pos::new(15, 16, 0));
        place(world, "kitchen island", Pos::new(8, 19, 0));   // 3x1, x=8..10
        place(world, "dining chair",  Pos::new(8, 22, 0));
        place(world, "dining chair",  Pos::new(10, 22, 0));
        place(world, "ceiling fan",   Pos::new(13, 22, 0));
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
        place_container(world, "nightstand", Pos::new(46, 9, 0),
            &["pearl earrings", "antique cufflinks"]);
        place_container(world, "nightstand", Pos::new(50, 9, 0),
            &["folded bills"]);
        place(world, "table lamp", Pos::new(46, 10, 0));
        place(world, "table lamp", Pos::new(50, 10, 0));
        place_container(world, "dresser", Pos::new(54, 12, 0),
            &["watch collection", "wedding ring"]);
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

        // ─── master closet (wardrobes are 2x1 each, space them out) ──
        place_container(world, "wardrobe", Pos::new(52, 20, 0),
            &["mink coat", "gold cufflinks"]);
        place_container(world, "wardrobe", Pos::new(55, 20, 0),
            &["evening dress", "silver chain"]);
        place_container(world, "wardrobe", Pos::new(52, 23, 0),
            &["dress shoes"]);
        place_container(world, "wardrobe", Pos::new(55, 23, 0),
            &["formal coat"]);

        // ─── kid bedroom ────────────────────────────────────────────
        place(world, "twin bed",     Pos::new(43, 28, 0));
        place(world, "twin bed",     Pos::new(48, 28, 0));
        place_container(world, "nightstand", Pos::new(45, 28, 0),
            &["piggy bank cash"]);
        place(world, "table lamp",   Pos::new(45, 29, 0));
        place_container(world, "dresser", Pos::new(50, 36, 0),
            &["birthday card with cash"]);
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

        // ─── upstairs (z = 1) — two bedrooms + landing ────────────
        // West bedroom (x=20..28, y=7..15)
        place(world, "queen bed",   Pos::new(22, 8, 1));
        place_container(world, "nightstand", Pos::new(25, 8, 1),
            &["leather wallet", "diamond stud earrings"]);
        place(world, "table lamp",  Pos::new(25, 9, 1));
        place_container(world, "wardrobe", Pos::new(20, 13, 1),
            &["silk dress", "vintage handbag"]);
        place(world, "ceiling fan", Pos::new(24, 11, 1));
        place_painting(world, "oil portrait", Pos::new(27, 8, 1));

        // East bedroom (x=30..40, y=7..15) — TEEN moves up here
        place(world, "twin bed",    Pos::new(32, 8, 1));
        place(world, "twin bed",    Pos::new(36, 8, 1));
        place_container(world, "nightstand", Pos::new(34, 9, 1),
            &["earphones", "phone"]);
        place(world, "desk",        Pos::new(38, 11, 1));
        place_container(world, "dresser", Pos::new(30, 14, 1),
            &["concert tickets", "pocket cash"]);
        place_painting(world, "abstract canvas", Pos::new(38, 8, 1));
        place_powered(world, "tv set", Pos::new(35, 14, 1), "upstairs TV (cartoons)");

        // Upstairs landing / hallway
        place(world, "table lamp",  Pos::new(29, 18, 1));
        place(world, "side table",  Pos::new(29, 17, 1));
        place(world, "potted plant", Pos::new(20, 23, 1));
        place(world, "potted plant", Pos::new(40, 23, 1));
        place_painting(world, "oil portrait", Pos::new(29, 24, 1));

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

        let teen = humanoid(world, "teen", Pos::new(34, 9, 1), FAMILY, 60,
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
        world.entity_mut(brute).insert(Invader).insert(LooterMemory::default());
        equip(world, brute, &["leather jacket", "leather boots", "steel crowbar"]);

        let burglar_a = humanoid(world, "burglar_a", Pos::new(30, DECK_Y_MIN + 2, 0), INVADER, 90, Stats::rogue());
        world.entity_mut(burglar_a).insert(Invader).insert(LooterMemory::default());
        equip(world, burglar_a, &["hoodie", "rubber boots", "hunting knife"]);

        let burglar_b = humanoid(world, "burglar_b", Pos::new(29, DECK_Y_MIN + 3, 0), INVADER, 85, Stats::rogue());
        world.entity_mut(burglar_b).insert(Invader).insert(LooterMemory::default());
        equip(world, burglar_b, &["hoodie", "rubber boots", "brass candlestick"]);

        let burglar_c = humanoid(world, "burglar_c", Pos::new(27, DECK_Y_MIN + 3, 0), INVADER, 85, Stats::rogue());
        world.entity_mut(burglar_c).insert(Invader).insert(LooterMemory::default());
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
                update_looter_memory,
                family_planner,
                invader_planner,
                execute_tasks,
                tick_carrying,
                tick_fire,
                tick_status_effects,
                tick_schedules,
                handle_door_use,
            )
                .chain(),
        );
        schedule.add_systems(
            (
                furniture_emit_system,
                emit_combat_sounds,
                emit_movement_sounds,
                update_sight,
                update_hearing,
                update_smell,
                footing_check,
                retaliation_system,
                fear_from_combat,
                decay_coatings,
                check_invader_departure,
            )
                .chain()
                .after(handle_door_use),
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
    let _ = spawn_door_returning(world, pos, mat, label, kind);
}

fn spawn_door_returning(world: &mut World, pos: Pos, mat: u16, label: &str, kind: DoorKind) -> Entity {
    let door = match kind {
        DoorKind::Open => {
            let mut d = Door::closed(mat, label);
            d.state = DoorState::Open;
            d
        }
        DoorKind::Closed => Door::closed(mat, label),
        DoorKind::Locked(dc) => Door::locked(mat, label, dc),
    };
    world.spawn((Position(pos), Kind(label.into()), door)).id()
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
    // Spawn each named valuable as an entity but DO NOT give it a
    // Position — the item lives inside the Container (no observer
    // can see it) until someone opens the container, at which point
    // `handle_container_open` drops the items at the container's tile.
    let mut item_ids: Vec<Entity> = Vec::new();
    for &item_name in contents {
        let label = item_name.to_string();
        let template_name = if world.resource::<fortress_engine::Library>().items.contains_key(item_name) {
            item_name.to_string()
        } else {
            "kitchen knife".to_string()
        };
        let opts = ItemSpawnOpts {
            at: None,
            override_label: Some(label),
            ..Default::default()
        };
        if let Ok(item) = spawn_item_template(world, &template_name, opts) {
            world.entity_mut(item).insert(Valuable);
            item_ids.push(item);
        }
    }
    if let Some(mut c) = world.get_mut::<Container>(container) {
        c.items.extend(item_ids);
    }
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

/// Refresh each invader's `LooterMemory` based on what they can
/// actually see this tick. Rather than re-using `Perceived.seen`
/// (which only tracks living creatures), we run our own
/// line-of-sight scan over Valuables and Containers — they're
/// stationary, so the cost is small.
fn update_looter_memory(world: &mut World) {
    let looters: Vec<(Entity, Pos, i32)> = {
        let mut q = world.query_filtered::<(Entity, &Position, &Sight), With<Invader>>();
        q.iter(world).map(|(e, p, s)| (e, p.0, s.range)).collect()
    };
    let visible_loot: Vec<(Entity, Pos)> = {
        let mut q = world.query_filtered::<(Entity, &Position), With<Valuable>>();
        q.iter(world).map(|(e, p)| (e, p.0)).collect()
    };
    let visible_containers: Vec<(Entity, Pos, bool)> = {
        let mut q = world.query::<(Entity, &Position, &Container)>();
        q.iter(world).map(|(e, p, c)| (e, p.0, c.open)).collect()
    };

    for (looter, lpos, range) in looters {
        let mut new_loot: Vec<Entity> = Vec::new();
        let mut new_containers: Vec<Entity> = Vec::new();
        for (item, ipos) in &visible_loot {
            if lpos.chebyshev(*ipos) > range {
                continue;
            }
            if line_of_sight_blocked(world, lpos, *ipos) {
                continue;
            }
            new_loot.push(*item);
        }
        for (cont, cpos, open) in &visible_containers {
            if *open {
                continue;
            }
            if lpos.chebyshev(*cpos) > range {
                continue;
            }
            if line_of_sight_blocked(world, lpos, *cpos) {
                continue;
            }
            new_containers.push(*cont);
        }
        if let Some(mut mem) = world.get_mut::<LooterMemory>(looter) {
            for e in new_loot {
                mem.known_loot.insert(e);
            }
            for e in new_containers {
                mem.known_containers.insert(e);
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

        // Step 1: visible hostile (a member of the family)
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

        // Step 2: heard violent noise — investigate
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

        // Snapshot memory once (we'll mutate selection below, but
        // selection is informational — not stored back).
        let (known_loot, known_containers) = {
            let mem = match world.get::<LooterMemory>(invader) {
                Some(m) => m,
                None => continue,
            };
            (
                mem.known_loot.iter().copied().collect::<Vec<_>>(),
                mem.known_containers.iter().copied().collect::<Vec<_>>(),
            )
        };

        // Step 3: pick from KNOWN loot we've seen with our own eyes.
        // Filter to ones still on the floor; sort by Value descending
        // (greedy burglar — grab the priciest first), then break ties
        // by manhattan distance.
        let mut loot_candidates: Vec<(Entity, Pos, u32, i32)> = known_loot
            .iter()
            .filter_map(|&e| {
                let p = world.get::<Position>(e)?;
                let v = world.get::<fortress_engine::Value>(e).map(|v| v.0).unwrap_or(0);
                Some((e, p.0, v, pos.manhattan(p.0)))
            })
            .collect();
        loot_candidates.sort_by(|a, b| b.2.cmp(&a.2).then(a.3.cmp(&b.3)));
        let target_loot: Option<(Entity, Pos)> = {
            let vw = world.resource::<VoxelWorld>();
            loot_candidates
                .into_iter()
                .find(|(_, vp, _, _)| find_path(vw, pos, *vp, 4096).is_some())
                .map(|(e, p, _, _)| (e, p))
        };

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

        // Step 4: any unopened container we know about?
        let mut container_candidates: Vec<(Entity, Pos)> = known_containers
            .iter()
            .filter(|&&e| {
                world.get::<Container>(e).map(|c| !c.open).unwrap_or(false)
            })
            .filter_map(|&e| world.get::<Position>(e).map(|p| (e, p.0)))
            .collect();
        container_candidates.sort_by_key(|(_, cp)| pos.manhattan(*cp));
        let target_container: Option<(Entity, Pos)> = {
            let vw = world.resource::<VoxelWorld>();
            container_candidates
                .into_iter()
                .find(|(_, cp)| {
                    let approach = approach_tile(pos, *cp);
                    find_path(vw, pos, approach, 4096).is_some()
                })
        };

        if let Some((cont, cont_pos)) = target_container {
            let approach = approach_tile(pos, cont_pos);
            if let Some(mut q) = world.get_mut::<TaskQueue>(invader) {
                q.clear();
                if pos != approach {
                    q.push(Task::MoveTo(approach));
                }
                q.push(Task::UseEntity(cont));
            }
            if let Some(mut g) = world.get_mut::<Goal>(invader) {
                *g = Goal::Tend(cont);
            }
            world.entity_mut(invader).insert(Locomotion::Walking);
            continue;
        }

        // Step 5: nothing in memory — search a new room. Pick the
        // closest unexplored room anchor and walk there. As we move,
        // `update_looter_memory` will log what we see along the way.
        let next_room = pick_next_search_room(world, invader, pos);
        if let Some((room_idx, anchor)) = next_room {
            if let Some(mut mem) = world.get_mut::<LooterMemory>(invader) {
                mem.current_search = Some(room_idx);
            }
            let reachable_anchor = {
                let vw = world.resource::<VoxelWorld>();
                find_path(vw, pos, anchor, 4096).is_some()
            };
            if !reachable_anchor {
                // The room is sealed by a closed door — try to break in.
                if let Some((door, door_pos)) = nearest_blocking_door(world, pos, anchor, &closed_doors) {
                    let approach = approach_tile(pos, door_pos);
                    if let Some(mut q) = world.get_mut::<TaskQueue>(invader) {
                        q.clear();
                        if pos != approach {
                            q.push(Task::MoveTo(approach));
                        }
                        q.push(Task::UseEntity(door));
                    }
                    if let Some(mut g) = world.get_mut::<Goal>(invader) {
                        *g = Goal::Tend(door);
                    }
                    world.entity_mut(invader).insert(Locomotion::Walking);
                    continue;
                }
                // No way in — give up on this room.
                if let Some(mut mem) = world.get_mut::<LooterMemory>(invader) {
                    mem.explored_rooms.insert(room_idx);
                    mem.current_search = None;
                }
                continue;
            }
            if !matches!(goal, Goal::GoTo(p) if p == anchor) || queue_empty {
                if let Some(mut q) = world.get_mut::<TaskQueue>(invader) {
                    q.clear();
                    q.push(Task::MoveTo(anchor));
                }
                if let Some(mut g) = world.get_mut::<Goal>(invader) {
                    *g = Goal::GoTo(anchor);
                }
                world.entity_mut(invader).insert(Locomotion::Walking);
            }
            continue;
        }

        // Step 6: nothing left to search — leave through the back.
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

/// Pick the next room to search: prefer rooms we can reach now,
/// closest first; if none are directly reachable, fall back to the
/// closest unexplored room (the planner will then route through a
/// closed door). Returns `None` when every room has been visited.
fn pick_next_search_room(world: &mut World, invader: Entity, pos: Pos) -> Option<(usize, Pos)> {
    // First, mark any room we're currently inside as explored.
    {
        let mut newly: Vec<usize> = Vec::new();
        for (i, (_, x_min, x_max, y_min, y_max, _)) in ROOMS.iter().enumerate() {
            if pos.x >= *x_min && pos.x <= *x_max && pos.y >= *y_min && pos.y <= *y_max {
                newly.push(i);
            }
        }
        if let Some(mut mem) = world.get_mut::<LooterMemory>(invader) {
            for i in newly {
                mem.explored_rooms.insert(i);
            }
        }
    }
    let explored = world
        .get::<LooterMemory>(invader)
        .map(|m| m.explored_rooms.clone())
        .unwrap_or_default();

    let mut reachable: Option<(usize, Pos, i32)> = None;
    let mut blocked: Option<(usize, Pos, i32)> = None;
    for (i, (_, x_min, x_max, y_min, y_max, _)) in ROOMS.iter().enumerate() {
        if explored.contains(&i) {
            continue;
        }
        let cx = (x_min + x_max) / 2;
        let cy = (y_min + y_max) / 2;
        let anchor = Pos::new(cx, cy, 0);
        let d = pos.manhattan(anchor);
        let path_exists = {
            let vw = world.resource::<VoxelWorld>();
            find_path(vw, pos, anchor, 4096).is_some()
        };
        if path_exists {
            if reachable.map(|(_, _, bd)| d < bd).unwrap_or(true) {
                reachable = Some((i, anchor, d));
            }
        } else if blocked.map(|(_, _, bd)| d < bd).unwrap_or(true) {
            blocked = Some((i, anchor, d));
        }
    }
    reachable.or(blocked).map(|(i, a, _)| (i, a))
}

/// Among `closed_doors` reachable from `from`, return the one whose
/// approach tile is closest to `to`. That's the door most likely to
/// be ON the path between us and our target room.
fn nearest_blocking_door(
    world: &mut World,
    from: Pos,
    to: Pos,
    closed_doors: &[(Entity, Pos)],
) -> Option<(Entity, Pos)> {
    let vw = world.resource::<VoxelWorld>();
    closed_doors
        .iter()
        .copied()
        .filter(|(_, dp)| {
            let approach = approach_tile(from, *dp);
            find_path(vw, from, approach, 4096).is_some()
        })
        .min_by_key(|(_, dp)| dp.manhattan(to))
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
        // Container? Open it (running a manipulation/strength check
        // if locked) and dump its contents at the container's tile.
        if world.get::<Container>(target).is_some() {
            handle_one_container_open(world, user, target, tick);
            continue;
        }

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

fn handle_one_container_open(world: &mut World, user: Entity, target: Entity, tick: u64) {
    let (already_open, locked, lock_dc, container_pos, container_kind) = {
        let c = match world.get::<Container>(target) {
            Some(c) => c.clone_metadata(),
            None => return,
        };
        let pos = world.get::<Position>(target).map(|p| p.0).unwrap_or_default();
        let kind = world.get::<Kind>(target).map(|k| k.0.clone()).unwrap_or_else(|| "container".into());
        (c.0, c.1, c.2, pos, kind)
    };
    if already_open {
        return;
    }
    if locked {
        // Strength check to force the lock open.
        let outcome = strength_check(world, user, lock_dc);
        let (success, roll, impossible) = unpack_check(&outcome);
        world.resource_mut::<EventLog>().push(
            tick,
            Event::AbilityCheck {
                actor: user,
                kind: "force open".into(),
                target: format!("the {container_kind}"),
                roll,
                dc: lock_dc,
                success,
                impossible,
            },
        );
        if !success {
            return;
        }
    }
    // Drop every item at the container's tile so observers
    // (including other invaders) can see and pick it up.
    let items: Vec<Entity> = {
        let mut c = match world.get_mut::<Container>(target) {
            Some(c) => c,
            None => return,
        };
        c.open = true;
        std::mem::take(&mut c.items)
    };
    for item in &items {
        world.entity_mut(*item).insert(Position(container_pos));
    }
    let actor_label = label_kind(world, user);
    world.resource_mut::<EventLog>().push(
        tick,
        Event::Note(format!(
            "{actor_label} pulls open the {container_kind} — {} items spill out.",
            items.len()
        )),
    );
}

trait ContainerMetadata {
    fn clone_metadata(&self) -> (bool, bool, i32);
}
impl ContainerMetadata for Container {
    fn clone_metadata(&self) -> (bool, bool, i32) {
        (self.open, self.locked, self.lock_dc)
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

