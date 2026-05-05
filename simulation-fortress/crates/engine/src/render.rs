//! Rendering hooks. The engine exposes a generic `Renderer` trait, an
//! ASCII voxel renderer for the command line, and a prose `LogRenderer`
//! that narrates the event log. Other renderers (TUI, graphical) can
//! implement the same trait without the engine ever depending on them.

use std::collections::HashMap;

use crate::entity::EntityStore;
use crate::log::{narrate, EventLog};
use crate::time::Tick;
use crate::world::{MaterialId, Pos, World, AIR};

/// Read-only window into the simulation, passed to renderers.
pub struct SimulationView<'a> {
    pub world: &'a World,
    pub entities: &'a EntityStore,
    pub log: &'a EventLog,
}

pub trait Renderer {
    /// Called once at tick 0 (after scenario setup) and after every tick.
    fn frame(&mut self, view: &SimulationView<'_>, tick: Tick);
}

/// A no-op renderer for headless runs.
pub struct NullRenderer;

impl Renderer for NullRenderer {
    fn frame(&mut self, _view: &SimulationView<'_>, _tick: Tick) {}
}

/// Run two renderers in sequence on each frame.
pub struct CompositeRenderer<A: Renderer, B: Renderer>(pub A, pub B);

impl<A: Renderer, B: Renderer> Renderer for CompositeRenderer<A, B> {
    fn frame(&mut self, view: &SimulationView<'_>, tick: Tick) {
        self.0.frame(view, tick);
        self.1.frame(view, tick);
    }
}

/// Renders a single Z-slice of the world as ASCII to stdout. Entities on
/// the slice are overlaid on top of voxel glyphs.
pub struct AsciiRenderer {
    pub min: Pos,
    pub max: Pos,
    pub z: i32,
    pub frame_every: u64,
    pub air_glyph: char,
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
            air_glyph: '.',
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

    fn glyph_for_voxel(&self, world: &World, pos: Pos) -> char {
        let voxel = world.voxel(pos);
        if voxel.material == AIR {
            return self.air_glyph;
        }
        if let Some(&g) = self.material_glyphs.get(&voxel.material) {
            return g;
        }
        if world.is_solid(pos) {
            self.default_solid_glyph
        } else {
            self.air_glyph
        }
    }

    fn glyph_for_entity_kind(&self, kind: &str) -> Option<char> {
        self.entity_kind_glyphs.get(kind).copied()
    }

    fn glyph_for_faction(&self, faction: &str) -> Option<char> {
        self.faction_glyphs.get(faction).copied()
    }
}

impl Renderer for AsciiRenderer {
    fn frame(&mut self, view: &SimulationView<'_>, tick: Tick) {
        if tick != 0 && !tick.is_multiple_of(self.frame_every) {
            return;
        }

        let mut overlay: HashMap<(i32, i32), char> = HashMap::new();
        for entity in view.entities.iter() {
            if !entity.is_alive() || entity.position.z != self.z {
                continue;
            }
            let glyph = self
                .glyph_for_entity_kind(&entity.kind)
                .or_else(|| {
                    entity
                        .faction
                        .as_deref()
                        .and_then(|f| self.glyph_for_faction(f))
                })
                .unwrap_or(self.default_entity_glyph);
            overlay.insert((entity.position.x, entity.position.y), glyph);
        }

        let alive = view.entities.iter().filter(|e| e.is_alive()).count();
        println!(
            "── tick {tick:>4} ── z={} ── entities {}/{} alive ──",
            self.z,
            alive,
            view.entities.len()
        );
        for y in self.min.y..=self.max.y {
            let mut row = String::with_capacity((self.max.x - self.min.x + 1) as usize);
            for x in self.min.x..=self.max.x {
                let glyph = overlay
                    .get(&(x, y))
                    .copied()
                    .unwrap_or_else(|| self.glyph_for_voxel(view.world, Pos::new(x, y, self.z)));
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
    fn frame(&mut self, view: &SimulationView<'_>, tick: Tick) {
        let events: Vec<_> = view.log.events_at(tick).collect();
        let header = if tick == 0 {
            "Setup".to_string()
        } else {
            format!("Tick {tick}")
        };

        if events.is_empty() {
            if self.announce_quiet {
                println!(
                    "[{header}] A quiet moment passes; nothing of consequence is recorded.\n"
                );
            }
            return;
        }

        let sentences: Vec<String> = events.iter().map(|e| narrate(e, view.entities)).collect();
        println!("[{header}] {}\n", sentences.join(" "));
    }
}
