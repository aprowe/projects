//! Rendering hooks. Renderers receive `&mut World` so they can run
//! ECS queries; they are expected to be effectively read-only by
//! convention. Two concrete renderers ship today: `AsciiRenderer` for
//! the voxel map, `LogRenderer` for prose narration. `CompositeRenderer`
//! runs two renderers per frame.

use std::collections::HashMap;

use bevy_ecs::prelude::World;

use crate::components::{Faction, Health, Kind, Position};
use crate::log::{narrate, EventLog};
use crate::time::Tick;
use crate::world::{MaterialId, Pos, TileKind, VoxelWorld};

pub trait Renderer {
    /// Called once at tick 0 (after scenario setup) and after every tick.
    fn frame(&mut self, world: &mut World, tick: Tick);
}

/// A no-op renderer for headless runs.
pub struct NullRenderer;

impl Renderer for NullRenderer {
    fn frame(&mut self, _world: &mut World, _tick: Tick) {}
}

/// Run two renderers in sequence on each frame.
pub struct CompositeRenderer<A: Renderer, B: Renderer>(pub A, pub B);

impl<A: Renderer, B: Renderer> Renderer for CompositeRenderer<A, B> {
    fn frame(&mut self, world: &mut World, tick: Tick) {
        self.0.frame(world, tick);
        self.1.frame(world, tick);
    }
}

/// Renders a single Z-slice of the world as ASCII to stdout. Living
/// entities on the slice are overlaid on top of voxel glyphs.
pub struct AsciiRenderer {
    pub min: Pos,
    pub max: Pos,
    pub z: i32,
    pub frame_every: u64,
    pub air_glyph: char,
    pub floor_glyph: char,
    pub ramp_glyph: char,
    pub default_solid_glyph: char,
    pub default_entity_glyph: char,
    pub material_glyphs: HashMap<MaterialId, char>,
    pub entity_kind_glyphs: HashMap<String, char>,
    pub faction_glyphs: HashMap<String, char>,
}

impl AsciiRenderer {
    pub fn new(min: Pos, max: Pos) -> Self {
        Self {
            min,
            max,
            z: min.z,
            frame_every: 1,
            air_glyph: ' ',
            floor_glyph: '.',
            ramp_glyph: '<',
            default_solid_glyph: '#',
            default_entity_glyph: '?',
            material_glyphs: HashMap::new(),
            entity_kind_glyphs: HashMap::new(),
            faction_glyphs: HashMap::new(),
        }
    }

    pub fn at_z(mut self, z: i32) -> Self {
        self.z = z;
        self
    }

    pub fn frame_every(mut self, every: u64) -> Self {
        self.frame_every = every.max(1);
        self
    }

    pub fn material(mut self, id: MaterialId, glyph: char) -> Self {
        self.material_glyphs.insert(id, glyph);
        self
    }

    pub fn entity_kind(mut self, kind: impl Into<String>, glyph: char) -> Self {
        self.entity_kind_glyphs.insert(kind.into(), glyph);
        self
    }

    pub fn faction(mut self, faction: impl Into<String>, glyph: char) -> Self {
        self.faction_glyphs.insert(faction.into(), glyph);
        self
    }

    pub fn floor_glyph(mut self, glyph: char) -> Self {
        self.floor_glyph = glyph;
        self
    }

    pub fn ramp_glyph(mut self, glyph: char) -> Self {
        self.ramp_glyph = glyph;
        self
    }

    fn glyph_for_voxel(&self, voxel_world: &VoxelWorld, pos: Pos) -> char {
        let voxel = voxel_world.voxel(pos);
        match voxel.kind {
            TileKind::Empty => self.air_glyph,
            TileKind::Floor => self.floor_glyph,
            TileKind::RampUp => self.ramp_glyph,
            TileKind::Wall => self
                .material_glyphs
                .get(&voxel.material)
                .copied()
                .unwrap_or(self.default_solid_glyph),
        }
    }
}

impl Renderer for AsciiRenderer {
    fn frame(&mut self, world: &mut World, tick: Tick) {
        if tick != 0 && !tick.is_multiple_of(self.frame_every) {
            return;
        }

        let mut overlay: HashMap<(i32, i32), char> = HashMap::new();
        let mut total = 0usize;
        let mut alive = 0usize;
        let mut q = world.query::<(&Kind, &Position, &Health, Option<&Faction>)>();
        for (kind, pos, health, faction) in q.iter(world) {
            total += 1;
            if !health.is_alive() {
                continue;
            }
            alive += 1;
            if pos.0.z != self.z {
                continue;
            }
            let glyph = self
                .entity_kind_glyphs
                .get(&kind.0)
                .copied()
                .or_else(|| {
                    faction
                        .and_then(|f| self.faction_glyphs.get(&f.0).copied())
                })
                .unwrap_or(self.default_entity_glyph);
            overlay.insert((pos.0.x, pos.0.y), glyph);
        }

        let voxel_world = world.resource::<VoxelWorld>();

        println!(
            "── tick {tick:>4} ── z={} ── entities {}/{} alive ──",
            self.z, alive, total
        );
        for y in self.min.y..=self.max.y {
            let mut row = String::with_capacity((self.max.x - self.min.x + 1) as usize);
            for x in self.min.x..=self.max.x {
                let glyph = overlay
                    .get(&(x, y))
                    .copied()
                    .unwrap_or_else(|| self.glyph_for_voxel(voxel_world, Pos::new(x, y, self.z)));
                row.push(glyph);
            }
            println!("{row}");
        }
        println!();
    }
}

/// Prints the event log as prose. Each tick produces roughly one
/// paragraph: setup events, then one sentence per logged event, then a
/// blank line. Quiet ticks emit a placeholder so the cadence is steady.
pub struct LogRenderer {
    pub announce_quiet: bool,
}

impl Default for LogRenderer {
    fn default() -> Self {
        Self {
            announce_quiet: true,
        }
    }
}

impl Renderer for LogRenderer {
    fn frame(&mut self, world: &mut World, tick: Tick) {
        let header = if tick == 0 {
            "Setup".to_string()
        } else {
            format!("Tick {tick}")
        };

        let events: Vec<crate::log::Event> = {
            let log = world.resource::<EventLog>();
            log.events_at(tick).cloned().collect()
        };
        let sentences: Vec<String> = events.iter().map(|e| narrate(e, world)).collect();

        if sentences.is_empty() {
            if self.announce_quiet {
                println!(
                    "[{header}] A quiet moment passes; nothing of consequence is recorded.\n"
                );
            }
            return;
        }

        println!("[{header}] {}\n", sentences.join(" "));
    }
}
