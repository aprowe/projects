use std::collections::HashMap;

use crate::world::Pos;

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct EntityId(pub u64);

#[derive(Clone, Debug)]
pub struct Entity {
    pub id: EntityId,
    pub kind: String,
    pub position: Pos,
    pub health: i32,
    pub faction: Option<String>,
    pub alive: bool,
    /// Open-ended per-scenario fields. Scenarios can stash anything here
    /// without needing to extend the engine.
    pub data: HashMap<String, String>,
}

impl Entity {
    pub fn is_alive(&self) -> bool {
        self.alive && self.health > 0
    }
}

#[derive(Default)]
pub struct EntityStore {
    next_id: u64,
    entities: HashMap<EntityId, Entity>,
}

impl EntityStore {
    pub fn spawn(&mut self, kind: impl Into<String>, position: Pos) -> EntityId {
        let id = EntityId(self.next_id);
        self.next_id += 1;
        let entity = Entity {
            id,
            kind: kind.into(),
            position,
            health: 100,
            faction: None,
            alive: true,
            data: HashMap::new(),
        };
        self.entities.insert(id, entity);
        id
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.get_mut(&id)
    }

    pub fn remove(&mut self, id: EntityId) -> Option<Entity> {
        self.entities.remove(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Entity> {
        self.entities.values_mut()
    }

    pub fn ids(&self) -> Vec<EntityId> {
        let mut ids: Vec<EntityId> = self.entities.keys().copied().collect();
        ids.sort();
        ids
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn living_in_faction(&self, faction: &str) -> usize {
        self.iter()
            .filter(|e| e.is_alive() && e.faction.as_deref() == Some(faction))
            .count()
    }
}
