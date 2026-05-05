//! Rendering hooks. Renderers receive `&mut World` so they can run
//! ECS queries; they are expected to be effectively read-only by
//! convention. Two concrete renderers ship today: `AsciiRenderer` for
//! the voxel map, `LogRenderer` for prose narration. `CompositeRenderer`
//! runs two renderers per frame.

use std::collections::HashMap;

use bevy_ecs::prelude::World;

use crate::components::{Faction, Health, Kind, Position};
use crate::items::ItemMaterial;
use crate::log::{narrate, EventLog};
use crate::quality::Paint;
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

impl<R: Renderer + ?Sized> Renderer for &mut R {
    fn frame(&mut self, world: &mut World, tick: Tick) {
        (**self).frame(world, tick)
    }
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
#[derive(Clone)]
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

impl AsciiRenderer {
    /// Render the world's current Z-slice as a multi-line string,
    /// independent of stdout. Includes a header and trailing blank
    /// line. Returns `None` when the configured `frame_every` skips
    /// this tick. `frame()` calls this and prints; the replay
    /// renderer captures the same string for HTML embedding.
    /// Render the current Z-slice as colored HTML (one `<span>` per
    /// cell). Coloring rules:
    /// - Entities with a `Paint` component use that exact color.
    /// - Otherwise an entity is colored by its `ItemMaterial` if
    ///   present (so a steel knife is steel-colored).
    /// - Otherwise entities pick up the underlying voxel material
    ///   color so they read against their background.
    /// - Voxels (walls / floors / air) take their `Material.color`.
    /// Returns `None` when `frame_every` skips this tick.
    pub fn frame_to_html(&mut self, world: &mut World, tick: Tick) -> Option<String> {
        if tick != 0 && !tick.is_multiple_of(self.frame_every) {
            return None;
        }
        // overlay map: (x, y) -> (glyph, optional explicit color)
        let mut overlay: HashMap<(i32, i32), (char, Option<[u8; 3]>)> = HashMap::new();
        let mut total = 0usize;
        let mut alive = 0usize;
        let mut q = world.query::<(
            &Kind,
            &Position,
            Option<&Health>,
            Option<&Faction>,
            Option<&Paint>,
            Option<&ItemMaterial>,
        )>();
        let voxel_world = world.resource::<VoxelWorld>();
        for (kind, pos, health, faction, paint, item_material) in q.iter(world) {
            if let Some(h) = health {
                total += 1;
                if !h.is_alive() {
                    continue;
                }
                alive += 1;
            }
            if pos.0.z != self.z {
                continue;
            }
            let resolved = self
                .entity_kind_glyphs
                .get(&kind.0)
                .copied()
                .or_else(|| faction.and_then(|f| self.faction_glyphs.get(&f.0).copied()));
            let glyph = match (health.is_some(), resolved) {
                (true, Some(g)) => g,
                (true, None) => self.default_entity_glyph,
                (false, Some(g)) => g,
                (false, None) => continue,
            };
            let color = paint.map(|p| p.0).or_else(|| {
                item_material
                    .and_then(|m| voxel_world.material(m.0).map(|mat| mat.color))
            });
            overlay.entry((pos.0.x, pos.0.y)).or_insert((glyph, color));
        }

        // First pass: collect unique color pairs into a palette so
        // every cell can emit a short class name (`c0`, `c1`, …)
        // instead of a long inline style. Classes are scoped to this
        // frame via a uniquifying prefix.
        let mut palette: Vec<([u8; 3], [u8; 3])> = Vec::new();
        let mut palette_index: HashMap<([u8; 3], [u8; 3]), usize> = HashMap::new();
        let mut cells: Vec<(usize, char)> = Vec::with_capacity(
            ((self.max.y - self.min.y + 1) * (self.max.x - self.min.x + 1)) as usize,
        );
        for y in self.min.y..=self.max.y {
            for x in self.min.x..=self.max.x {
                let pos = Pos::new(x, y, self.z);
                let voxel = voxel_world.voxel(pos);
                let bg = voxel_world
                    .material(voxel.material)
                    .map(|m| m.color)
                    .unwrap_or([24, 24, 24]);
                let (glyph, fg) = match overlay.get(&(x, y)) {
                    Some((g, Some(c))) => (*g, *c),
                    Some((g, None)) => (*g, contrast_for(bg)),
                    None => (
                        self.glyph_for_voxel(voxel_world, pos),
                        match voxel.kind {
                            TileKind::Wall => bg,
                            TileKind::Floor => mix(bg, [200, 200, 200], 0.30),
                            _ => contrast_for(bg),
                        },
                    ),
                };
                let key = (fg, bg);
                let idx = match palette_index.get(&key) {
                    Some(&i) => i,
                    None => {
                        let i = palette.len();
                        palette.push(key);
                        palette_index.insert(key, i);
                        i
                    }
                };
                cells.push((idx, glyph));
            }
        }

        let prefix = format!("f{tick}");
        let mut out = String::new();
        out.push_str("<style>");
        for (i, (fg, bg)) in palette.iter().enumerate() {
            out.push_str(&format!(
                ".{prefix}-{i}{{color:{};background:{}}}",
                rgb_hex(*fg),
                rgb_hex(*bg),
            ));
        }
        out.push_str("</style>");
        out.push_str(&format!(
            "<div class=\"meta\">tick {tick} — z={} — entities {alive}/{total}</div>",
            self.z
        ));
        out.push_str("<div class=\"map\">");
        let width = (self.max.x - self.min.x + 1) as usize;
        for (i, (cls, glyph)) in cells.iter().enumerate() {
            if i % width == 0 {
                if i != 0 {
                    out.push_str("</div>");
                }
                out.push_str("<div class=\"row\">");
            }
            let escaped = match *glyph {
                '<' => "&lt;",
                '>' => "&gt;",
                '&' => "&amp;",
                '"' => "&quot;",
                ' ' => "&nbsp;",
                _ => "",
            };
            if escaped.is_empty() {
                out.push_str(&format!(
                    "<span class=\"{prefix}-{cls}\">{}</span>",
                    glyph
                ));
            } else {
                out.push_str(&format!(
                    "<span class=\"{prefix}-{cls}\">{escaped}</span>"
                ));
            }
        }
        out.push_str("</div></div>");
        Some(out)
    }

    pub fn frame_to_string(&mut self, world: &mut World, tick: Tick) -> Option<String> {
        if tick != 0 && !tick.is_multiple_of(self.frame_every) {
            return None;
        }

        let mut overlay: HashMap<(i32, i32), char> = HashMap::new();
        let mut total = 0usize;
        let mut alive = 0usize;
        let mut q = world.query::<(&Kind, &Position, Option<&Health>, Option<&Faction>)>();
        for (kind, pos, health, faction) in q.iter(world) {
            if let Some(h) = health {
                total += 1;
                if !h.is_alive() {
                    continue;
                }
                alive += 1;
            }
            if pos.0.z != self.z {
                continue;
            }
            let resolved = self
                .entity_kind_glyphs
                .get(&kind.0)
                .copied()
                .or_else(|| {
                    faction
                        .and_then(|f| self.faction_glyphs.get(&f.0).copied())
                });
            let glyph = match (health.is_some(), resolved) {
                (true, Some(g)) => g,
                (true, None) => self.default_entity_glyph,
                (false, Some(g)) => g,
                (false, None) => continue,
            };
            overlay.entry((pos.0.x, pos.0.y)).or_insert(glyph);
        }

        let voxel_world = world.resource::<VoxelWorld>();
        let mut out = String::new();
        out.push_str(&format!(
            "── tick {tick:>4} ── z={} ── entities {}/{} alive ──\n",
            self.z, alive, total
        ));
        for y in self.min.y..=self.max.y {
            for x in self.min.x..=self.max.x {
                let glyph = overlay
                    .get(&(x, y))
                    .copied()
                    .unwrap_or_else(|| self.glyph_for_voxel(voxel_world, Pos::new(x, y, self.z)));
                out.push(glyph);
            }
            out.push('\n');
        }
        out.push('\n');
        Some(out)
    }
}

impl Renderer for AsciiRenderer {
    fn frame(&mut self, world: &mut World, tick: Tick) {
        if let Some(s) = self.frame_to_string(world, tick) {
            print!("{s}");
        }
    }
}

fn rgb_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let lerp = |x: u8, y: u8| -> u8 {
        let v = (x as f32) * (1.0 - t) + (y as f32) * t;
        v.round().clamp(0.0, 255.0) as u8
    };
    [lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2])]
}

/// Pick a foreground color (white or near-black) that has decent
/// contrast against `bg`.
fn contrast_for(bg: [u8; 3]) -> [u8; 3] {
    // Simple luminance heuristic
    let luma = 0.2126 * bg[0] as f32 + 0.7152 * bg[1] as f32 + 0.0722 * bg[2] as f32;
    if luma > 140.0 {
        [16, 16, 24]
    } else {
        [232, 232, 240]
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
        let sentences: Vec<String> = events
            .iter()
            .map(|e| narrate(e, world))
            .filter(|s| !s.is_empty())
            .collect();

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
