//! Home invasion scenario, expressed as ECS systems on the engine's
//! bevy world. A wood-walled cabin sits on a grass plot. Three family
//! members live inside; an intruder approaches from the north and tries
//! to kill them all.

use fortress_engine::actions::{fill_region_logged, note, spawn_creature};
use fortress_engine::prelude::*;
use fortress_engine::{
    equip_item, find_path, BodySlot, Clock, ElectricalConductivity, Event, EventLog, Health, Item,
    ItemName, Kind, Mass, Material, Position, Pos, Scenario, Temperature, Texture,
    ThermalConductivity, Voxel, VoxelWorld, Wearable,
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
            });
            let grass = vw.register_material(Material {
                name: "grass".into(),
                solid: false,
                density: 0.1,
                flammable: true,
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
            world.entity_mut(entity).insert(Family);

            let shirt = spawn_wool_shirt(world);
            equip_item(world, entity, shirt);
            let boots = spawn_leather_boots(world);
            equip_item(world, entity, boots);
        }
        note(world, "Three residents settle into the cabin, going about their evening.");

        let intruder = spawn_creature(world, "intruder", Pos::new(4, -3, 0), 120, Some(INTRUDER));
        world.entity_mut(intruder).insert(Intruder);

        let crowbar = spawn_crowbar(world);
        equip_item(world, intruder, crowbar);
        note(
            world,
            "A masked intruder approaches from the north, crowbar in hand, eyes on the door.",
        );
    }

    fn build_schedule(&mut self) -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(intruder_behavior);
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

/// Per-tick AI for the intruder: re-plan to the nearest living family
/// member, attack if adjacent, otherwise step along the path.
#[allow(clippy::type_complexity)]
fn intruder_behavior(
    voxel_world: Res<VoxelWorld>,
    clock: Res<Clock>,
    mut log: ResMut<EventLog>,
    mut intruders: Query<(Entity, &Kind, &mut Position, &mut Health), With<Intruder>>,
    mut family: Query<
        (Entity, &Kind, &Position, &mut Health),
        (With<Family>, Without<Intruder>),
    >,
) {
    let tick = clock.tick;

    let Ok((intruder_entity, _intruder_kind, mut intruder_pos, mut intruder_hp)) =
        intruders.single_mut()
    else {
        return;
    };
    if !intruder_hp.is_alive() {
        return;
    }

    let target = family
        .iter()
        .filter(|(_, _, _, h)| h.is_alive())
        .min_by_key(|(_, _, p, _)| intruder_pos.0.manhattan(p.0))
        .map(|(e, k, p, _)| (e, k.0.clone(), p.0));

    let Some((target_entity, target_kind, target_pos)) = target else {
        log.push(
            tick,
            Event::Note("The intruder pauses, breath ragged; no one alive remains to threaten.".into()),
        );
        return;
    };

    let path = find_path(&voxel_world, intruder_pos.0, target_pos, 4096);
    let Some(path) = path else {
        log.push(
            tick,
            Event::Note(format!(
                "The intruder peers about but can't find a path to {target_kind}#{}.",
                target_entity.index()
            )),
        );
        return;
    };
    if path.len() < 2 {
        return;
    }

    let next = path[1];
    if next == target_pos {
        log.push(
            tick,
            Event::Note(format!(
                "The intruder closes the gap on {target_kind}#{} and swings the crowbar.",
                target_entity.index()
            )),
        );
        // Hit the family member.
        if let Ok((_, _, _, mut target_hp)) = family.get_mut(target_entity) {
            target_hp.current -= 25;
            log.push(
                tick,
                Event::EntityAttacked {
                    attacker: Some(intruder_entity),
                    target: target_entity,
                    damage: 25,
                    remaining_health: target_hp.current,
                },
            );
            if !target_hp.is_alive() {
                log.push(
                    tick,
                    Event::EntityKilled {
                        entity: target_entity,
                        by: Some(intruder_entity),
                    },
                );
            }
        }
        // Family fights back a little.
        log.push(
            tick,
            Event::Note(format!(
                "{target_kind}#{} fights back desperately, landing a few blows in return.",
                target_entity.index()
            )),
        );
        intruder_hp.current -= 5;
        log.push(
            tick,
            Event::EntityAttacked {
                attacker: Some(target_entity),
                target: intruder_entity,
                damage: 5,
                remaining_health: intruder_hp.current,
            },
        );
        if !intruder_hp.is_alive() {
            log.push(
                tick,
                Event::EntityKilled {
                    entity: intruder_entity,
                    by: Some(target_entity),
                },
            );
        }
    } else {
        let was_outside = !inside_house(intruder_pos.0);
        let now_inside = inside_house(next);
        if was_outside && now_inside {
            log.push(
                tick,
                Event::Note("The intruder ducks through the doorway and into the cabin.".into()),
            );
        }
        let from = intruder_pos.0;
        intruder_pos.0 = next;
        log.push(
            tick,
            Event::EntityMoved {
                entity: intruder_entity,
                from,
                to: next,
            },
        );
    }
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
