//! A small example scenario: a wood-walled house with a family inside and
//! an intruder at the door. Demonstrates building voxel structures,
//! spawning faction-tagged entities, and driving simple per-tick behavior
//! with rich event-log narration.

use fortress_engine::{
    Entity, EntityId, EntityStore, Material, Pos, Scenario, SetupContext, TickContext, Voxel,
    World,
};

const FAMILY: &str = "family";
const INTRUDER: &str = "intruder";

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
        ctx.world.register_material(Material {
            name: "stone".into(),
            solid: true,
            density: 2.5,
            flammable: false,
        });
        ctx.note("A small wooden house stands alone at the edge of the woods.");

        let wall = Voxel::of(wood);
        for x in 0..8 {
            for y in 0..8 {
                let on_edge = x == 0 || x == 7 || y == 0 || y == 7;
                let is_door = x == 4 && y == 0;
                if on_edge && !is_door {
                    ctx.world.set_voxel(Pos::new(x, y, 0), wall);
                }
            }
        }
        ctx.note("The walls go up: wooden planks form an 8x8 single-room cabin with a door on the north side.");

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

        let next = intruder.position.step_toward(target.position);
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
            let inside_house = next.x >= 1 && next.x <= 6 && next.y >= 1 && next.y <= 6;
            if inside_house && !inside_position(intruder.position) {
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

fn inside_position(pos: Pos) -> bool {
    pos.x >= 1 && pos.x <= 6 && pos.y >= 1 && pos.y <= 6 && pos.z == 0
}
