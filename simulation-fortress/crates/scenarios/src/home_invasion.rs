//! Home invasion scenario, expressed as ECS systems on the engine's
//! bevy world. A wood-walled cabin sits on a grass plot. Three family
//! members live inside; an intruder approaches from the north and tries
//! to kill them all.

use fortress_engine::actions::{fill_region_logged, note, spawn_creature};
use fortress_engine::prelude::*;
use fortress_engine::{
    derive_mood, equip_item, execute_tasks, fear_from_combat, footing_check, retaliation_system,
    spawn_humanoid_body, tick_needs, BodySlot, Clock, ElectricalConductivity, Event, EventLog,
    Fear, Goal, Health, Item, ItemName, Kind, Mass, Material, Mood, Position, Pos,
    RetaliateOnAttack, Scenario, Task, TaskQueue, Temperature, Texture, ThermalConductivity,
    Voxel, VoxelWorld, Wearable,
};

const FAMILY: &str = "family";
const INTRUDER: &str = "intruder";

const HOUSE_X: std::ops::Range<i32> = 0..8;
const HOUSE_Y: std::ops::Range<i32> = 0..8;
const DOOR: Pos = Pos::new(4, 0, 0);
const GROUND_MIN: Pos = Pos::new(-2, -5, 0);
const GROUND_MAX: Pos = Pos::new(10, 10, 0);

/// Marker component for the lone intruder agent. Lets a system query
/// only that entity without filtering by string.
#[derive(Component)]
pub struct Intruder;

/// Marker component for family members.
#[derive(Component)]
pub struct Family;

#[derive(Default)]
pub struct HomeInvasion;

impl Scenario for HomeInvasion {
    fn name(&self) -> &str {
        "home invasion"
    }

    fn setup(&mut self, world: &mut World) {
        let (wood, grass) = {
            let mut vw = world.resource_mut::<VoxelWorld>();
            let wood = vw.register_material(Material {
                name: "wood".into(),
                solid: true,
                density: 0.7,
                flammable: true,
                friction: 0.55,
                smell_intensity: 0.05,
                volatility: 0.0,
                color: [120, 120, 120],
            });
            let grass = vw.register_material(Material {
                name: "grass".into(),
                solid: false,
                density: 0.1,
                flammable: true,
                friction: 0.8,
                smell_intensity: 0.05,
                volatility: 0.0,
                color: [120, 120, 120],
            });
            (wood, grass)
        };

        note(world, "A small wooden cabin stands alone at the edge of the woods.");
        fill_region_logged(world, GROUND_MIN, GROUND_MAX, Voxel::floor(grass));
        note(world, "Grass spreads in every direction, soft underfoot.");
        fill_region_logged(
            world,
            Pos::new(HOUSE_X.start, HOUSE_Y.start, 0),
            Pos::new(HOUSE_X.end - 1, HOUSE_Y.end - 1, 0),
            Voxel::floor(wood),
        );
        note(world, "Inside the cabin, planks of wood form the floor.");

        // Walls (overwriting the wood floor at the perimeter, with a door gap).
        let wall = Voxel::wall(wood);
        {
            let mut vw = world.resource_mut::<VoxelWorld>();
            for x in HOUSE_X {
                for y in HOUSE_Y {
                    let on_edge = x == HOUSE_X.start
                        || x == HOUSE_X.end - 1
                        || y == HOUSE_Y.start
                        || y == HOUSE_Y.end - 1;
                    let pos = Pos::new(x, y, 0);
                    if on_edge && pos != DOOR {
                        vw.set_voxel(pos, wall);
                    }
                }
            }
        }
        note(
            world,
            "The walls go up: wooden planks form an 8x8 single-room cabin with a door on the north side.",
        );

        for (i, (x, y)) in [(2, 4), (5, 4), (3, 6)].into_iter().enumerate() {
            let entity = spawn_creature(
                world,
                format!("resident_{i}"),
                Pos::new(x, y, 0),
                60,
                Some(FAMILY),
            );
            world
                .entity_mut(entity)
                .insert(Family)
                .insert(TaskQueue::default())
                .insert(Goal::default())
                .insert(RetaliateOnAttack)
                .insert(Fear::calm())
                .insert(Mood::default());
            spawn_humanoid_body(world, entity);

            let shirt = spawn_wool_shirt(world);
            equip_item(world, entity, shirt);
            let boots = spawn_leather_boots(world);
            equip_item(world, entity, boots);
        }
        note(world, "Three residents settle into the cabin, going about their evening.");

        let intruder = spawn_creature(world, "intruder", Pos::new(4, -3, 0), 120, Some(INTRUDER));
        world
            .entity_mut(intruder)
            .insert(Intruder)
            .insert(TaskQueue::default())
            .insert(Goal::default())
            .insert(Mood::default());
        spawn_humanoid_body(world, intruder);

        let crowbar = spawn_crowbar(world);
        equip_item(world, intruder, crowbar);
        note(
            world,
            "A masked intruder approaches from the north, crowbar in hand, eyes on the door.",
        );
    }

    fn build_schedule(&mut self) -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                tick_needs,
                derive_mood,
                intruder_planner,
                doorway_announcer,
                execute_tasks,
                footing_check,
                retaliation_system,
                fear_from_combat,
            )
                .chain(),
        );
        schedule
    }

    fn is_complete(&self, world: &mut World) -> bool {
        let mut family_alive = 0;
        let mut intruder_alive = 0;
        let mut q = world.query::<(&Health, Option<&Family>, Option<&Intruder>)>();
        for (health, family, intruder) in q.iter(world) {
            if !health.is_alive() {
                continue;
            }
            if family.is_some() {
                family_alive += 1;
            }
            if intruder.is_some() {
                intruder_alive += 1;
            }
        }
        family_alive == 0 || intruder_alive == 0
    }
}

/// Picks a target for the intruder and queues `Task::Attack(target)`
/// when the goal changes. Replans when the current target dies.
fn intruder_planner(world: &mut World) {
    let intruder = match find_intruder(world) {
        Some(e) => e,
        None => return,
    };
    let alive = world
        .get::<Health>(intruder)
        .map(|h| h.is_alive())
        .unwrap_or(false);
    if !alive {
        return;
    }

    let intruder_pos = world.get::<Position>(intruder).map(|p| p.0).unwrap();
    let target = nearest_living_family(world, intruder_pos).map(|(e, _, _)| e);
    let new_goal = match target {
        Some(t) => Goal::Kill(t),
        None => Goal::Idle,
    };

    let current_goal = world.get::<Goal>(intruder).cloned().unwrap_or_default();
    let queue_empty = world
        .get::<TaskQueue>(intruder)
        .map(|q| q.is_empty())
        .unwrap_or(true);

    let needs_replan = current_goal != new_goal || queue_empty;
    if !needs_replan {
        return;
    }

    if let Some(mut q) = world.get_mut::<TaskQueue>(intruder) {
        q.clear();
    }
    if let Some(mut g) = world.get_mut::<Goal>(intruder) {
        *g = new_goal.clone();
    }

    match new_goal {
        Goal::Kill(target_entity) => {
            if let Some(mut q) = world.get_mut::<TaskQueue>(intruder) {
                q.push(Task::Attack(target_entity));
            }
        }
        Goal::Idle => {
            push_note(
                world,
                "The intruder pauses, breath ragged; no one alive remains to threaten.",
            );
        }
        _ => {}
    }
}

/// Watches `EntityMoved` events for the intruder crossing into the
/// cabin and emits a one-line note. Pure narration; no game state
/// changes.
fn doorway_announcer(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let intruder = match find_intruder(world) {
        Some(e) => e,
        None => return,
    };
    let crossings: Vec<()> = world
        .resource::<EventLog>()
        .events_at(tick)
        .filter_map(|e| match e {
            Event::EntityMoved {
                entity,
                from,
                to,
            } if *entity == intruder && !inside_house(*from) && inside_house(*to) => Some(()),
            _ => None,
        })
        .collect();
    if !crossings.is_empty() {
        push_note(
            world,
            "The intruder ducks through the doorway and into the cabin.",
        );
    }
}

fn find_intruder(world: &mut World) -> Option<Entity> {
    let mut q = world.query_filtered::<Entity, With<Intruder>>();
    q.iter(world).next()
}

fn nearest_living_family(world: &mut World, from: Pos) -> Option<(Entity, String, Pos)> {
    let mut q =
        world.query_filtered::<(Entity, &Kind, &Position, &Health), (With<Family>, Without<Intruder>)>();
    q.iter(world)
        .filter(|(_, _, _, h)| h.is_alive())
        .min_by_key(|(_, _, p, _)| from.manhattan(p.0))
        .map(|(e, k, p, _)| (e, k.0.clone(), p.0))
}

fn push_note(world: &mut World, msg: impl Into<String>) {
    let tick = world.resource::<Clock>().tick;
    world
        .resource_mut::<EventLog>()
        .push(tick, Event::Note(msg.into()));
}


fn inside_house(pos: Pos) -> bool {
    pos.z == 0
        && pos.x > HOUSE_X.start
        && pos.x < HOUSE_X.end - 1
        && pos.y > HOUSE_Y.start
        && pos.y < HOUSE_Y.end - 1
}

fn spawn_crowbar(world: &mut World) -> Entity {
    world
        .spawn((
            Item,
            ItemName::new("steel crowbar"),
            Mass(2.5),
            Temperature(20.0),
            ThermalConductivity(50.0),
            ElectricalConductivity(1.0e7),
            Texture::Polished,
            Wearable(BodySlot::MainHand),
        ))
        .id()
}

fn spawn_wool_shirt(world: &mut World) -> Entity {
    world
        .spawn((
            Item,
            ItemName::new("wool shirt"),
            Mass(0.3),
            Temperature(30.0),
            ThermalConductivity(0.04),
            ElectricalConductivity(1.0e-13),
            Texture::Soft,
            Wearable(BodySlot::Torso),
        ))
        .id()
}

fn spawn_leather_boots(world: &mut World) -> Entity {
    world
        .spawn((
            Item,
            ItemName::new("leather boots"),
            Mass(0.9),
            Temperature(28.0),
            ThermalConductivity(0.14),
            ElectricalConductivity(1.0e-9),
            Texture::Rough,
            Wearable(BodySlot::Feet),
        ))
        .id()
}
