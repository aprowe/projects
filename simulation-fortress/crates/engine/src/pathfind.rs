//! A* pathfinding over the voxel world.
//!
//! Operates on a single Z-level for now (vertical moves require ramps
//! which the search does not yet traverse). 8-connected neighborhoods
//! with diagonal corner-cutting forbidden: a diagonal step requires
//! both flanking cardinal neighbors to also be walkable.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::world::{Pos, VoxelWorld};

const NEIGHBORS_2D: &[(i32, i32)] = &[
    (-1, -1), (0, -1), (1, -1),
    (-1, 0),           (1, 0),
    (-1, 1),  (0, 1),  (1, 1),
];

const STEP_CARDINAL: u32 = 10;
const STEP_DIAGONAL: u32 = 14;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
struct Node {
    f: u32,
    g: u32,
    pos: Pos,
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.f.cmp(&other.f).then(self.g.cmp(&other.g))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Find the shortest walkable path from `start` to `goal`.
///
/// Returns `None` if no path exists, the goal is not walkable, or the
/// search exceeds `max_iter` expansions. The returned path includes
/// both `start` and `goal` (so `path[1]` is the first step).
pub fn find_path(world: &VoxelWorld, start: Pos, goal: Pos, max_iter: usize) -> Option<Vec<Pos>> {
    if start == goal {
        return Some(vec![start]);
    }
    if !world.is_walkable(goal) {
        return None;
    }

    let mut open: BinaryHeap<Reverse<Node>> = BinaryHeap::new();
    let mut g_score: HashMap<Pos, u32> = HashMap::new();
    let mut came_from: HashMap<Pos, Pos> = HashMap::new();

    open.push(Reverse(Node {
        f: heuristic(start, goal),
        g: 0,
        pos: start,
    }));
    g_score.insert(start, 0);

    let mut iters = 0usize;
    while let Some(Reverse(Node { f: _, g, pos: current })) = open.pop() {
        if iters >= max_iter {
            return None;
        }
        iters += 1;

        if current == goal {
            return Some(reconstruct(&came_from, current));
        }

        if g > *g_score.get(&current).unwrap_or(&u32::MAX) {
            continue;
        }

        for (dx, dy) in NEIGHBORS_2D {
            let next = Pos::new(current.x + dx, current.y + dy, current.z);
            if !world.is_walkable(next) {
                continue;
            }

            let diagonal = dx.abs() == 1 && dy.abs() == 1;
            if diagonal {
                let a = Pos::new(current.x + dx, current.y, current.z);
                let b = Pos::new(current.x, current.y + dy, current.z);
                if !world.is_walkable(a) || !world.is_walkable(b) {
                    continue;
                }
            }

            let step_cost = if diagonal { STEP_DIAGONAL } else { STEP_CARDINAL };
            let tentative_g = g + step_cost;
            if tentative_g < *g_score.get(&next).unwrap_or(&u32::MAX) {
                g_score.insert(next, tentative_g);
                came_from.insert(next, current);
                let f = tentative_g + heuristic(next, goal);
                open.push(Reverse(Node {
                    f,
                    g: tentative_g,
                    pos: next,
                }));
            }
        }
    }

    None
}

fn reconstruct(came_from: &HashMap<Pos, Pos>, mut current: Pos) -> Vec<Pos> {
    let mut path = vec![current];
    while let Some(&prev) = came_from.get(&current) {
        path.push(prev);
        current = prev;
    }
    path.reverse();
    path
}

fn heuristic(a: Pos, b: Pos) -> u32 {
    let dx = (a.x - b.x).unsigned_abs();
    let dy = (a.y - b.y).unsigned_abs();
    let min = dx.min(dy);
    let max = dx.max(dy);
    STEP_DIAGONAL * min + STEP_CARDINAL * (max - min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Material, MaterialId, Voxel};

    fn stone(world: &mut VoxelWorld) -> MaterialId {
        world.register_material(Material {
            name: "stone".into(),
            solid: true,
            density: 2.5,
            flammable: false,
            friction: 0.7,
        })
    }

    fn floor_grid(world: &mut VoxelWorld, mat: MaterialId, w: i32, h: i32) {
        for x in 0..w {
            for y in 0..h {
                world.set_voxel(Pos::new(x, y, 0), Voxel::floor(mat));
            }
        }
    }

    #[test]
    fn straight_path_on_open_floor() {
        let mut world = VoxelWorld::new();
        let stone = stone(&mut world);
        floor_grid(&mut world, stone, 5, 1);
        let path = find_path(&world, Pos::new(0, 0, 0), Pos::new(4, 0, 0), 1024).unwrap();
        assert_eq!(path.first(), Some(&Pos::new(0, 0, 0)));
        assert_eq!(path.last(), Some(&Pos::new(4, 0, 0)));
        assert_eq!(path.len(), 5);
    }

    #[test]
    fn detours_around_a_wall_through_a_doorway() {
        let mut world = VoxelWorld::new();
        let stone = stone(&mut world);
        floor_grid(&mut world, stone, 5, 5);
        // Wall down the middle column except at y=2 (the doorway).
        for y in 0..5 {
            if y != 2 {
                world.set_voxel(Pos::new(2, y, 0), Voxel::wall(stone));
            }
        }
        let path = find_path(&world, Pos::new(0, 0, 0), Pos::new(4, 0, 0), 1024).unwrap();
        assert_eq!(path.first(), Some(&Pos::new(0, 0, 0)));
        assert_eq!(path.last(), Some(&Pos::new(4, 0, 0)));
        assert!(path.contains(&Pos::new(2, 2, 0)), "path must use doorway: {:?}", path);
    }

    #[test]
    fn diagonal_corner_cutting_is_forbidden() {
        let mut world = VoxelWorld::new();
        let stone = stone(&mut world);
        floor_grid(&mut world, stone, 2, 2);
        // Walls in the two cardinal cells that flank the diagonal.
        world.set_voxel(Pos::new(1, 0, 0), Voxel::wall(stone));
        world.set_voxel(Pos::new(0, 1, 0), Voxel::wall(stone));
        // (1,1) is walkable but unreachable from (0,0) without corner-cut.
        let path = find_path(&world, Pos::new(0, 0, 0), Pos::new(1, 1, 0), 256);
        assert!(path.is_none(), "should not corner-cut, got {:?}", path);
    }

    #[test]
    fn unreachable_returns_none() {
        let mut world = VoxelWorld::new();
        let stone = stone(&mut world);
        world.set_voxel(Pos::new(0, 0, 0), Voxel::floor(stone));
        let path = find_path(&world, Pos::new(0, 0, 0), Pos::new(5, 5, 0), 1024);
        assert!(path.is_none());
    }

    #[test]
    fn unwalkable_goal_returns_none() {
        let mut world = VoxelWorld::new();
        let stone = stone(&mut world);
        floor_grid(&mut world, stone, 3, 3);
        world.set_voxel(Pos::new(2, 2, 0), Voxel::wall(stone));
        let path = find_path(&world, Pos::new(0, 0, 0), Pos::new(2, 2, 0), 256);
        assert!(path.is_none());
    }
}
