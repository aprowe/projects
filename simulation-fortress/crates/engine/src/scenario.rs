use crate::entity::{EntityId, EntityStore};
use crate::log::{Event, EventLog};
use crate::time::Tick;
use crate::world::{Pos, Voxel, World};

/// Mutable access handed to scenarios during `setup`. Helpers on this
/// type push the corresponding events into the log automatically, so
/// scenarios get a default narration without repeating themselves.
pub struct SetupContext<'a> {
    pub world: &'a mut World,
    pub entities: &'a mut EntityStore,
    pub log: &'a mut EventLog,
}

impl SetupContext<'_> {
    pub fn spawn(&mut self, kind: impl Into<String>, position: Pos) -> EntityId {
        let kind = kind.into();
        let id = self.entities.spawn(kind.clone(), position);
        self.log.push(
            0,
            Event::EntitySpawned {
                id,
                kind,
                faction: None,
                at: position,
            },
        );
        id
    }

    pub fn set_faction(&mut self, id: EntityId, faction: impl Into<String>) {
        if let Some(e) = self.entities.get_mut(id) {
            e.faction = Some(faction.into());
        }
    }

    pub fn set_voxel(&mut self, pos: Pos, voxel: Voxel) {
        self.world.set_voxel(pos, voxel);
        self.log.push(
            0,
            Event::VoxelChanged {
                at: pos,
                material: voxel.material,
            },
        );
    }

    pub fn fill(&mut self, min: Pos, max: Pos, voxel: Voxel) {
        self.world.fill(min, max, voxel);
        self.log.push(
            0,
            Event::VoxelRegionFilled {
                min,
                max,
                material: voxel.material,
            },
        );
    }

    pub fn note(&mut self, message: impl Into<String>) {
        self.log.push(0, Event::Note(message.into()));
    }
}

/// Mutable access handed to scenarios during `tick`.
pub struct TickContext<'a> {
    pub world: &'a mut World,
    pub entities: &'a mut EntityStore,
    pub log: &'a mut EventLog,
    pub tick: Tick,
}

impl TickContext<'_> {
    pub fn move_entity(&mut self, id: EntityId, to: Pos) {
        if let Some(e) = self.entities.get_mut(id) {
            let from = e.position;
            if from == to {
                return;
            }
            e.position = to;
            self.log
                .push(self.tick, Event::EntityMoved { id, from, to });
        }
    }

    /// Apply damage and emit attack/kill events. `attacker` is optional so
    /// environmental damage (fire, falling) can be modeled too.
    pub fn damage(&mut self, target: EntityId, attacker: Option<EntityId>, amount: i32) {
        let Some(e) = self.entities.get_mut(target) else {
            return;
        };
        e.health -= amount;
        let remaining_health = e.health;
        let died = e.alive && e.health <= 0;
        if died {
            e.alive = false;
        }
        self.log.push(
            self.tick,
            Event::EntityAttacked {
                attacker,
                target,
                damage: amount,
                remaining_health,
            },
        );
        if died {
            self.log
                .push(self.tick, Event::EntityKilled { id: target, by: attacker });
        }
    }

    pub fn note(&mut self, message: impl Into<String>) {
        self.log.push(self.tick, Event::Note(message.into()));
    }
}

/// A scenario describes a self-contained simulation: how to populate the
/// initial world and entities, what happens on each tick, and when the
/// simulation should be considered complete.
pub trait Scenario {
    fn name(&self) -> &str;

    fn setup(&mut self, ctx: &mut SetupContext<'_>);

    fn tick(&mut self, ctx: &mut TickContext<'_>);

    fn is_complete(&self, _world: &World, _entities: &EntityStore, _tick: Tick) -> bool {
        false
    }
}
