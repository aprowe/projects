use std::collections::HashMap;

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
}

impl Material {
    pub fn air() -> Self {
        Self {
            name: "air".into(),
            solid: false,
            density: 0.0,
            flammable: false,
        }
    }
}

#[derive(Copy, Clone, Default, Debug, Eq, PartialEq)]
pub struct Voxel {
    pub material: MaterialId,
    /// Damage accumulated on this voxel: 0 is pristine, 255 is destroyed.
    pub damage: u8,
}

impl Voxel {
    pub const fn of(material: MaterialId) -> Self {
        Self { material, damage: 0 }
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

pub struct World {
    chunks: HashMap<ChunkCoord, Chunk>,
    materials: Vec<Material>,
    name_to_material: HashMap<String, MaterialId>,
}

impl World {
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

    pub fn is_solid(&self, pos: Pos) -> bool {
        self.material(self.voxel(pos).material)
            .map(|m| m.solid)
            .unwrap_or(false)
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

impl Default for World {
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

    #[test]
    fn voxels_round_trip_across_chunk_boundaries() {
        let mut world = World::new();
        let stone = world.register_material(Material {
            name: "stone".into(),
            solid: true,
            density: 2.5,
            flammable: false,
        });
        let positions = [
            Pos::new(0, 0, 0),
            Pos::new(15, 15, 15),
            Pos::new(16, 0, 0),
            Pos::new(-1, -1, -1),
            Pos::new(-17, 5, 32),
        ];
        for p in positions {
            world.set_voxel(p, Voxel::of(stone));
        }
        for p in positions {
            assert_eq!(world.voxel(p).material, stone, "round trip failed at {:?}", p);
        }
    }

    #[test]
    fn fill_marks_a_solid_region() {
        let mut world = World::new();
        let wood = world.register_material(Material {
            name: "wood".into(),
            solid: true,
            density: 0.7,
            flammable: true,
        });
        world.fill(Pos::new(0, 0, 0), Pos::new(2, 2, 0), Voxel::of(wood));
        for x in 0..=2 {
            for y in 0..=2 {
                assert!(world.is_solid(Pos::new(x, y, 0)));
            }
        }
        assert!(!world.is_solid(Pos::new(3, 0, 0)));
    }
}
