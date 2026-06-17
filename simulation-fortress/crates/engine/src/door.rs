//! Doors — voxel-attached entities that change the world's
//! walkability based on their state.
//!
//! A `Door` entity has a `Position` (the tile it occupies) and a
//! `Door` component carrying the current `DoorState` and DCs for
//! lockpicking and forcing it open. The `door_voxel_sync` engine
//! system runs each tick: closed/locked doors set the underlying
//! voxel to a wood Wall (impassable, blocks pathfind + line of
//! sight); open or broken doors set it to a wood Floor (passable).
//!
//! Interactions are scenario-driven: a scenario watches for
//! `EntityUsed { user, target }` events where the target has a
//! `Door`, calls `manipulation_check` (DEX + Grasp) or
//! `strength_check` (STR + Mobility) as appropriate, and updates
//! the door's state.

use bevy_ecs::prelude::{Component, Entity, World};

use crate::components::Position;
use crate::world::{Pos, Voxel, VoxelWorld};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DoorState {
    Open,
    Closed,
    Locked,
    /// Hinges torn off; permanently passable but the doorway looks
    /// bad in narration.
    Broken,
}

impl DoorState {
    pub fn label(self) -> &'static str {
        match self {
            DoorState::Open => "open",
            DoorState::Closed => "closed",
            DoorState::Locked => "locked",
            DoorState::Broken => "broken",
        }
    }

    pub fn is_passable(self) -> bool {
        matches!(self, DoorState::Open | DoorState::Broken)
    }
}

#[derive(Component, Clone, Debug)]
pub struct Door {
    pub state: DoorState,
    /// Material used for the underlying voxel when the door is
    /// solid (closed/locked/broken-but-pretending). Most household
    /// doors are wood.
    pub material: u16,
    /// DC for a manipulation_check to pick the lock. 0 means
    /// "unlocked even when state is Locked" (won't happen — Locked
    /// implies dc > 0).
    pub lock_dc: i32,
    /// DC for a strength_check to break the door open by force.
    pub break_dc: i32,
    /// Friendly label for narration ("front door", "kitchen door").
    pub label: String,
}

impl Door {
    pub fn closed(material_id: u16, label: impl Into<String>) -> Self {
        Self {
            state: DoorState::Closed,
            material: material_id,
            lock_dc: 0,
            break_dc: 15,
            label: label.into(),
        }
    }

    pub fn locked(material_id: u16, label: impl Into<String>, lock_dc: i32) -> Self {
        Self {
            state: DoorState::Locked,
            material: material_id,
            lock_dc,
            break_dc: 18,
            label: label.into(),
        }
    }
}

/// Sync the voxel grid to match each door's current state. Closed
/// or locked doors become Wall voxels of the door's material; open
/// or broken doors become Floor voxels. Re-sync each tick — cheap
/// because there are only ever a handful of doors.
pub fn door_voxel_sync(world: &mut World) {
    let pairs: Vec<(Pos, DoorState, u16)> = {
        let mut q = world.query::<(&Position, &Door)>();
        q.iter(world)
            .map(|(p, d)| (p.0, d.state, d.material))
            .collect()
    };
    let mut vw = world.resource_mut::<VoxelWorld>();
    for (pos, state, material) in pairs {
        let voxel = if state.is_passable() {
            Voxel::floor(material)
        } else {
            Voxel::wall(material)
        };
        vw.set_voxel(pos, voxel);
    }
}

/// Helper: find the door entity at `pos`, if any.
pub fn door_at(world: &mut World, pos: Pos) -> Option<Entity> {
    let mut q = world.query::<(Entity, &Position, &Door)>();
    q.iter(world)
        .find(|(_, p, _)| p.0 == pos)
        .map(|(e, _, _)| e)
}
