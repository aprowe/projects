//! A small example scenario: a wood-walled house with a family inside and
//! an intruder at the door. Demonstrates building voxel structures with
//! floors and walls, spawning faction-tagged entities, and driving simple
//! per-tick behavior over A* pathfinding with rich event-log narration.

use fortress_engine::{
    find_path, Entity, EntityId, EntityStore, Material, Pos, Scenario, SetupContext, TickContext,
    Voxel, World,
};

const FAMILY: &str = "family";
const INTRUDER: &str = "intruder";

const HOUSE_X: std::ops::Range<i32> = 0..8;
const HOUSE_Y: std::ops::Range<i32> = 0..8;
const DOOR: Pos = Pos::new(4, 0, 0);
const GROUND_MIN: Pos = Pos::new(-2, -5, 0);
const GROUND_MAX: Pos = Pos::new(10, 10, 0);

#[derive(Default)]
pub struct HomeInvasion {
    intruder: Option<EntityId>,
}

impl Scenario for HomeInvasion {
    fn name(&self) -> &str {
        "home invasion"
    }

    fn setup(&mut self, ctx: &mut SetupContext<'_>) {
        let wood = ctx.world.register_material(Material {
            name: "wood".into(),
            solid: true,
            density: 0.7,
            flammable: true,
        });
        let grass = ctx.world.register_material(Material {
            name: "grass".into(),
            solid: false,
            density: 0.1,
            flammable: true,
        });
        ctx.note("A small wooden cabin stands alone at the edge of the woods.");

        ctx.fill(GROUND_MIN, GROUND_MAX, Voxel::floor(grass));
        ctx.note("Grass spreads in every direction, soft underfoot.");

        ctx.fill(
            Pos::new(HOUSE_X.start, HOUSE_Y.start, 0),
            Pos::new(HOUSE_X.end - 1, HOUSE_Y.end - 1, 0),
            Voxel::floor(wood),
        );
        ctx.note("Inside the cabin, planks of wood form the floor.");

        let wall = Voxel::wall(wood);
        for x in HOUSE_X {
            for y in HOUSE_Y {
                let on_edge = x == HOUSE_X.start
                    || x == HOUSE_X.end - 1
                    || y == HOUSE_Y.start
                    || y == HOUSE_Y.end - 1;
                let pos = Pos::new(x, y, 0);
                if on_edge && pos != DOOR {
                    ctx.world.set_voxel(pos, wall);
                }
            }
        }
        ctx.note(
            "The walls go up: wooden planks form an 8x8 single-room cabin with a door on the north side.",
        );

        for (i, (x, y)) in [(2, 4), (5, 4), (3, 6)].into_iter().enumerate() {
            let id = ctx.spawn(format!("resident_{i}"), Pos::new(x, y, 0));
            ctx.set_faction(id, FAMILY);
            if let Some(e) = ctx.entities.get_mut(id) {
                e.health = 60;
            }
        }
        ctx.note("Three residents settle into the cabin, going about their evening.");

        let intruder_id = ctx.spawn("intruder", Pos::new(4, -3, 0));
        ctx.set_faction(intruder_id, INTRUDER);
        if let Some(intruder) = ctx.entities.get_mut(intruder_id) {
            intruder.health = 120;
            intruder.data.insert("weapon".into(), "crowbar".into());
        }
        ctx.note("A masked intruder approaches from the north, crowbar in hand, eyes on the door.");
        self.intruder = Some(intruder_id);
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        let Some(intruder_id) = self.intruder else {
            return;
        };
        let Some(intruder) = ctx.entities.get(intruder_id).cloned() else {
            return;
        };
        if !intruder.is_alive() {
            return;
        }

        let Some(target) = nearest_living(ctx.entities, &intruder, FAMILY) else {
            ctx.note("The intruder pauses, breath ragged; no one alive remains to threaten.");
            return;
        };

        let path = find_path(ctx.world, intruder.position, target.position, 4096);
        let Some(path) = path else {
            ctx.note(format!(
                "The intruder peers about but can't find a path to {}#{}.",
                target.kind, target.id.0
            ));
            return;
        };
        if path.len() < 2 {
            return;
        }

        let next = path[1];
        if next == target.position {
            ctx.note(format!(
                "The intruder closes the gap on {}#{} and swings the crowbar.",
                target.kind, target.id.0
            ));
            ctx.damage(target.id, Some(intruder.id), 25);
            ctx.note(format!(
                "{}#{} fights back desperately, landing a few blows in return.",
                target.kind, target.id.0
            ));
            ctx.damage(intruder.id, Some(target.id), 5);
        } else {
            if !inside_house(intruder.position) && inside_house(next) {
                ctx.note("The intruder ducks through the doorway and into the cabin.");
            }
            ctx.move_entity(intruder.id, next);
        }
    }

    fn is_complete(
        &self,
        _world: &World,
        entities: &EntityStore,
        _tick: fortress_engine::Tick,
    ) -> bool {
        entities.living_in_faction(FAMILY) == 0 || entities.living_in_faction(INTRUDER) == 0
    }
}

fn nearest_living(entities: &EntityStore, from: &Entity, faction: &str) -> Option<Entity> {
    entities
        .iter()
        .filter(|e| e.is_alive() && e.faction.as_deref() == Some(faction))
        .min_by_key(|e| from.position.manhattan(e.position))
        .cloned()
}

fn inside_house(pos: Pos) -> bool {
    pos.z == 0
        && pos.x > HOUSE_X.start
        && pos.x < HOUSE_X.end - 1
        && pos.y > HOUSE_Y.start
        && pos.y < HOUSE_Y.end - 1
}
