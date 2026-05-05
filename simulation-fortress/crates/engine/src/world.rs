use std::collections::HashMap;

use bevy_ecs::resource::Resource;

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Pos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub fn manhattan(self, other: Pos) -> i32 {
        (self.x - other.x).abs() + (self.y - other.y).abs() + (self.z - other.z).abs()
    }

    pub fn chebyshev(self, other: Pos) -> i32 {
        (self.x - other.x)
            .abs()
            .max((self.y - other.y).abs())
            .max((self.z - other.z).abs())
    }

    pub fn step_toward(self, target: Pos) -> Pos {
        Pos {
            x: self.x + (target.x - self.x).signum(),
            y: self.y + (target.y - self.y).signum(),
            z: self.z + (target.z - self.z).signum(),
        }
    }
}

pub type MaterialId = u16;

pub const AIR: MaterialId = 0;

#[derive(Clone, Debug)]
pub struct Material {
    pub name: String,
    pub solid: bool,
    pub density: f32,
    pub flammable: bool,
    /// Coefficient of friction in `[0.0, 1.0+]`. 0.0 = frictionless
    /// (oil, magma), ~0.15 = ice, 0.5 = polished stone, 0.6–0.7 =
    /// most floors, ~0.9 = rough rubber sole. Used by
    /// `physics::footing_check` to decide whether moving onto a tile
    /// makes a creature slip.
    pub friction: f32,
}

impl Material {
    pub fn air() -> Self {
        Self {
            name: "air".into(),
            solid: false,
            density: 0.0,
            flammable: false,
            friction: 1.0,
        }
    }
}

/// What the voxel cell physically is. Inspired by Dwarf Fortress: a tile
/// can be empty air, a zero-height floor (walkable surface, no solid in
/// the cell), a full solid wall, or a ramp connecting two Z levels.
#[repr(u8)]
#[derive(Copy, Clone, Default, Debug, Eq, PartialEq, Hash)]
pub enum TileKind {
    #[default]
    Empty = 0,
    Floor = 1,
    Wall = 2,
    RampUp = 3,
}

#[derive(Copy, Clone, Default, Debug, Eq, PartialEq)]
pub struct Voxel {
    pub kind: TileKind,
    pub damage: u8,
    pub material: MaterialId,
}

impl Voxel {
    pub const fn empty() -> Self {
        Self {
            kind: TileKind::Empty,
            damage: 0,
            material: AIR,
        }
    }

    pub const fn floor(material: MaterialId) -> Self {
        Self {
            kind: TileKind::Floor,
            damage: 0,
            material,
        }
    }

    pub const fn wall(material: MaterialId) -> Self {
        Self {
            kind: TileKind::Wall,
            damage: 0,
            material,
        }
    }

    pub const fn ramp(material: MaterialId) -> Self {
        Self {
            kind: TileKind::RampUp,
            damage: 0,
            material,
        }
    }
}

pub const CHUNK_SIZE: i32 = 16;
const CHUNK_VOLUME: usize = (CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE) as usize;

#[derive(Clone)]
pub struct Chunk {
    pub voxels: Box<[Voxel; CHUNK_VOLUME]>,
}

impl Default for Chunk {
    fn default() -> Self {
        Self {
            voxels: Box::new([Voxel::default(); CHUNK_VOLUME]),
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct ChunkCoord(pub i32, pub i32, pub i32);

/// The voxel grid resource. Owned by the bevy ECS world as a `Resource`.
#[derive(Resource)]
pub struct VoxelWorld {
    chunks: HashMap<ChunkCoord, Chunk>,
    materials: Vec<Material>,
    name_to_material: HashMap<String, MaterialId>,
}

impl VoxelWorld {
    pub fn new() -> Self {
        let mut world = Self {
            chunks: HashMap::new(),
            materials: Vec::new(),
            name_to_material: HashMap::new(),
        };
        world.register_material(Material::air());
        world
    }

    pub fn register_material(&mut self, material: Material) -> MaterialId {
        let id = self.materials.len() as MaterialId;
        self.name_to_material.insert(material.name.clone(), id);
        self.materials.push(material);
        id
    }

    pub fn material(&self, id: MaterialId) -> Option<&Material> {
        self.materials.get(id as usize)
    }

    pub fn material_id(&self, name: &str) -> Option<MaterialId> {
        self.name_to_material.get(name).copied()
    }

    pub fn voxel(&self, pos: Pos) -> Voxel {
        let (cc, idx) = chunk_index(pos);
        self.chunks
            .get(&cc)
            .map(|c| c.voxels[idx])
            .unwrap_or_default()
    }

    pub fn set_voxel(&mut self, pos: Pos, voxel: Voxel) {
        let (cc, idx) = chunk_index(pos);
        self.chunks.entry(cc).or_default().voxels[idx] = voxel;
    }

    pub fn fill(&mut self, min: Pos, max: Pos, voxel: Voxel) {
        for z in min.z..=max.z {
            for y in min.y..=max.y {
                for x in min.x..=max.x {
                    self.set_voxel(Pos::new(x, y, z), voxel);
                }
            }
        }
    }

    /// True if the cell is a full solid block that nothing can pass through.
    pub fn is_solid(&self, pos: Pos) -> bool {
        self.voxel(pos).kind == TileKind::Wall
    }

    /// True if a creature can stand in this tile.
    ///
    /// Floors and ramps are walkable on their own. Empty tiles are
    /// walkable only if supported from below by a Wall or RampUp (i.e.
    /// you stand on the top surface of the block beneath).
    pub fn is_walkable(&self, pos: Pos) -> bool {
        match self.voxel(pos).kind {
            TileKind::Wall => false,
            TileKind::Floor | TileKind::RampUp => true,
            TileKind::Empty => {
                let below = self.voxel(Pos::new(pos.x, pos.y, pos.z - 1));
                matches!(below.kind, TileKind::Wall | TileKind::RampUp)
            }
        }
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

impl Default for VoxelWorld {
    fn default() -> Self {
        Self::new()
    }
}

fn chunk_index(pos: Pos) -> (ChunkCoord, usize) {
    let cx = pos.x.div_euclid(CHUNK_SIZE);
    let cy = pos.y.div_euclid(CHUNK_SIZE);
    let cz = pos.z.div_euclid(CHUNK_SIZE);
    let lx = pos.x.rem_euclid(CHUNK_SIZE);
    let ly = pos.y.rem_euclid(CHUNK_SIZE);
    let lz = pos.z.rem_euclid(CHUNK_SIZE);
    let idx = (lz * CHUNK_SIZE * CHUNK_SIZE + ly * CHUNK_SIZE + lx) as usize;
    (ChunkCoord(cx, cy, cz), idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone(world: &mut VoxelWorld) -> MaterialId {
        world.register_material(Material {
            name: "stone".into(),
            solid: true,
            density: 2.5,
            flammable: false,
            friction: 0.7,
        })
    }

    #[test]
    fn voxels_round_trip_across_chunk_boundaries() {
        let mut world = VoxelWorld::new();
        let stone = stone(&mut world);
        let positions = [
            Pos::new(0, 0, 0),
            Pos::new(15, 15, 15),
            Pos::new(16, 0, 0),
            Pos::new(-1, -1, -1),
            Pos::new(-17, 5, 32),
        ];
        for p in positions {
            world.set_voxel(p, Voxel::wall(stone));
        }
        for p in positions {
            assert_eq!(world.voxel(p).material, stone, "round trip failed at {:?}", p);
            assert_eq!(world.voxel(p).kind, TileKind::Wall);
        }
    }

    #[test]
    fn floor_is_walkable_but_not_solid() {
        let mut world = VoxelWorld::new();
        let stone = stone(&mut world);
        let p = Pos::new(0, 0, 0);
        world.set_voxel(p, Voxel::floor(stone));
        assert!(world.is_walkable(p));
        assert!(!world.is_solid(p));
    }

    #[test]
    fn empty_tile_walkable_only_with_support() {
        let mut world = VoxelWorld::new();
        let stone = stone(&mut world);
        let p = Pos::new(0, 0, 0);
        // No support below — empty floats.
        assert!(!world.is_walkable(p));
        // Wall directly below provides support — top of block is walkable.
        world.set_voxel(Pos::new(0, 0, -1), Voxel::wall(stone));
        assert!(world.is_walkable(p));
    }

    #[test]
    fn wall_is_solid_and_unwalkable() {
        let mut world = VoxelWorld::new();
        let stone = stone(&mut world);
        let p = Pos::new(0, 0, 0);
        world.set_voxel(p, Voxel::wall(stone));
        assert!(world.is_solid(p));
        assert!(!world.is_walkable(p));
    }
}
