//! Helper actions that mutate the world AND push the corresponding
//! event into the log, so scenarios get narration for free.
//!
//! Use these from setup code or from systems that hold `&mut World`.
//! Pure ECS systems can also write events directly via `ResMut<EventLog>`.

use bevy_ecs::prelude::{Entity, World};

use crate::components::{Faction, Health, Kind, Position};
use crate::log::{Event, EventLog};
use crate::time::Clock;
use crate::world::{Pos, Voxel, VoxelWorld};

/// Spawn a creature with the standard component bundle and log the
/// `EntitySpawned` event at the current tick.
pub fn spawn_creature(
    world: &mut World,
    kind: impl Into<String>,
    position: Pos,
    health: i32,
    faction: Option<&str>,
) -> Entity {
    let kind = kind.into();
    let mut entity_mut = world.spawn((
        Kind(kind.clone()),
        Position(position),
        Health::new(health),
    ));
    if let Some(f) = faction {
        entity_mut.insert(Faction(f.to_string()));
    }
    let entity = entity_mut.id();

    let tick = world.resource::<Clock>().tick;
    world.resource_mut::<EventLog>().push(
        tick,
        Event::EntitySpawned {
            entity,
            kind,
            faction: faction.map(String::from),
            at: position,
        },
    );
    entity
}

/// Set a single voxel and log the change.
pub fn set_voxel_logged(world: &mut World, at: Pos, voxel: Voxel) {
    world.resource_mut::<VoxelWorld>().set_voxel(at, voxel);
    let tick = world.resource::<Clock>().tick;
    world.resource_mut::<EventLog>().push(
        tick,
        Event::VoxelChanged {
            at,
            material: voxel.material,
        },
    );
}

/// Fill an axis-aligned region with the same voxel and log a single
/// `VoxelRegionFilled` event for the whole region.
pub fn fill_region_logged(world: &mut World, min: Pos, max: Pos, voxel: Voxel) {
    world.resource_mut::<VoxelWorld>().fill(min, max, voxel);
    let tick = world.resource::<Clock>().tick;
    world.resource_mut::<EventLog>().push(
        tick,
        Event::VoxelRegionFilled {
            min,
            max,
            material: voxel.material,
        },
    );
}

/// Push a free-form note onto the log.
pub fn note(world: &mut World, message: impl Into<String>) {
    let tick = world.resource::<Clock>().tick;
    world
        .resource_mut::<EventLog>()
        .push(tick, Event::Note(message.into()));
}
