//! Interactive terminal UI for any scenario.
//!
//! Drops into a `ratatui` + `crossterm` event loop with:
//!
//! - Colored map view in the center.
//! - **Cursor** that you move with arrow keys; the inspector
//!   panel on the right shows what's at the cursor (entity, body,
//!   inventory, equipment, container contents, voxel material).
//! - **Mouse**: click any cell to jump the cursor there.
//! - **Playback**: space to single-step, `p` to play/pause,
//!   `+`/`-` to change auto-step speed, `,`/`.` to nudge the
//!   camera, `[`/`]` to scrub Z-levels.
//! - **God mode** (`g`): a menu lets you spawn a creature from a
//!   library role, an item, a piece of furniture; change the
//!   voxel under the cursor to a chosen material; or kill the
//!   entity at the cursor. Each spawn happens *between* ticks
//!   so the simulation cleanly absorbs the change.
//! - `q` to quit.
//!
//! The TUI never hard-codes scenario specifics — it only uses
//! engine APIs (`Library`, `spawn_*_template`, `World` queries).
//! Any scenario type works.

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use bevy_ecs::entity::Entity;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as CtEvent, KeyCode, KeyEventKind,
    MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use fortress_engine::{
    spawn_furniture_template, spawn_item_template, spawn_role_template, AsciiRenderer,
    BodyPartKind, FurnitureSpawnOpts, Health, ItemSpawnOpts, Kind, Library, PartHealth, PartOf,
    PartStatus, Position, Pos, Quality, RoleSpawnOpts, Scenario, Simulation, Style,
    TaskQueue, Value, Voxel, VoxelWorld, Wearing,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style as TuiStyle};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;

type Backend = CrosstermBackend<Stdout>;

/// Factory: produces a fresh boxed scenario + the AsciiRenderer
/// configured for it. Called whenever the player loads a new
/// scenario or rewinds (rewind = re-setup from tick 0 + step
/// forward to the desired tick).
pub type ScenarioFactory = Box<dyn Fn() -> (Box<dyn Scenario>, AsciiRenderer)>;

pub struct ScenarioEntry {
    pub name: String,
    pub factory: ScenarioFactory,
}

/// Public entry point. `entries` is the scenario menu; `initial`
/// is the index of the one to start with (None = show the menu
/// first, before any scenario runs).
pub fn run(entries: Vec<ScenarioEntry>, initial: Option<usize>) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_inner(&mut terminal, entries, initial);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    res
}

fn run_inner(
    terminal: &mut Terminal<Backend>,
    entries: Vec<ScenarioEntry>,
    initial: Option<usize>,
) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut current_idx = initial.unwrap_or(0);
    let mut show_menu = initial.is_none();
    // If non-zero, fast-forward this many ticks after re-setup —
    // used to implement rewind by replaying the deterministic sim.
    let mut target_tick: u64 = 0;
    let mut last_flash: Option<String> = None;

    loop {
        let (mut scenario, ascii) = (entries[current_idx].factory)();
        let mut sim = Simulation::new();
        let mut schedule =
            sim.prepare(scenario.as_mut(), fortress_engine::RunOptions::default());
        for _ in 0..target_tick {
            sim.step(&mut schedule);
        }
        let mut state = AppState::new(ascii);
        state.scenario_label = entries[current_idx].name.clone();
        state.ticks = sim.current_tick();
        if let Some(msg) = last_flash.take() {
            state.flash(msg);
        }
        if show_menu {
            state.mode = Mode::ScenarioMenu;
            state.picker = entries.iter().map(|e| e.name.clone()).collect();
            state.picker_idx = current_idx;
            show_menu = false;
        }

        match event_loop(
            terminal, &entries, &mut sim, &mut schedule, &mut scenario, &mut state,
        )? {
            LoopExit::Quit => return Ok(()),
            LoopExit::SwitchScenario(i) => {
                current_idx = i;
                target_tick = 0;
            }
            LoopExit::Rewind(t) => {
                target_tick = t;
                last_flash = Some(format!("rewound to tick {t}"));
            }
        }
    }
}

enum LoopExit {
    Quit,
    SwitchScenario(usize),
    /// Rewind: rebuild from tick 0 and fast-forward to this tick.
    Rewind(u64),
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum Mode {
    Normal,
    GodMenu,
    GodSpawnRole,
    GodSpawnItem,
    GodSpawnFurniture,
    GodSetVoxel,
    ScenarioMenu,
}

struct AppState {
    ascii: AsciiRenderer,
    cursor: Pos,
    z: i32,
    auto_play: bool,
    /// Milliseconds between auto-steps.
    pace_ms: u64,
    last_step: Instant,
    mode: Mode,
    /// Scrollable pick lists for god-mode submenus.
    picker: Vec<String>,
    picker_filter: String,
    picker_idx: usize,
    /// Last status message (shown in footer for ~2s).
    flash: Option<(String, Instant)>,
    /// Total ticks advanced.
    ticks: u64,
    /// Display label for the active scenario.
    scenario_label: String,
}

impl AppState {
    fn new(ascii: AsciiRenderer) -> Self {
        let center = Pos::new(
            (ascii.min.x + ascii.max.x) / 2,
            (ascii.min.y + ascii.max.y) / 2,
            ascii.z,
        );
        Self {
            ascii,
            cursor: center,
            z: center.z,
            auto_play: false,
            pace_ms: 250,
            last_step: Instant::now(),
            mode: Mode::Normal,
            picker: Vec::new(),
            picker_filter: String::new(),
            picker_idx: 0,
            flash: None,
            ticks: 0,
            scenario_label: String::new(),
        }
    }

    fn flash(&mut self, msg: impl Into<String>) {
        self.flash = Some((msg.into(), Instant::now()));
    }
}

fn event_loop(
    terminal: &mut Terminal<Backend>,
    entries: &[ScenarioEntry],
    sim: &mut Simulation,
    schedule: &mut bevy_ecs::schedule::Schedule,
    _scenario: &mut Box<dyn Scenario>,
    state: &mut AppState,
) -> io::Result<LoopExit> {
    loop {
        // Auto-step if playing and pace elapsed.
        if state.auto_play
            && state.mode == Mode::Normal
            && state.last_step.elapsed() >= Duration::from_millis(state.pace_ms)
        {
            sim.step(schedule);
            state.ticks = sim.current_tick();
            state.last_step = Instant::now();
        }

        terminal.draw(|f| draw(f, sim, state))?;

        // Read input with timeout so the auto-step can fire.
        let timeout = Duration::from_millis(50);
        if event::poll(timeout)? {
            match event::read()? {
                CtEvent::Key(k) if k.kind == KeyEventKind::Press => {
                    match handle_key(k.code, entries, sim, schedule, state) {
                        KeyOutcome::Continue => {}
                        KeyOutcome::Quit => return Ok(LoopExit::Quit),
                        KeyOutcome::Switch(i) => return Ok(LoopExit::SwitchScenario(i)),
                        KeyOutcome::Rewind(t) => return Ok(LoopExit::Rewind(t)),
                    }
                }
                CtEvent::Mouse(m) => handle_mouse(m, terminal, state),
                _ => {}
            }
        }
    }
}

enum KeyOutcome {
    Continue,
    Quit,
    Switch(usize),
    Rewind(u64),
}

fn handle_key(
    code: KeyCode,
    entries: &[ScenarioEntry],
    sim: &mut Simulation,
    schedule: &mut bevy_ecs::schedule::Schedule,
    state: &mut AppState,
) -> KeyOutcome {
    match state.mode {
        Mode::Normal => match code {
            KeyCode::Char('q') | KeyCode::Esc => return KeyOutcome::Quit,
            KeyCode::Up => state.cursor.y -= 1,
            KeyCode::Down => state.cursor.y += 1,
            KeyCode::Left => state.cursor.x -= 1,
            KeyCode::Right => state.cursor.x += 1,
            KeyCode::Char(' ') => {
                sim.step(schedule);
                state.ticks = sim.current_tick();
            }
            KeyCode::Char('p') => {
                state.auto_play = !state.auto_play;
                state.last_step = Instant::now();
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                state.pace_ms = (state.pace_ms.saturating_sub(50)).max(20);
            }
            KeyCode::Char('-') => {
                state.pace_ms = (state.pace_ms + 100).min(2000);
            }
            KeyCode::Char('[') => {
                state.z -= 1;
                state.cursor.z = state.z;
                state.ascii.z = state.z;
            }
            KeyCode::Char(']') => {
                state.z += 1;
                state.cursor.z = state.z;
                state.ascii.z = state.z;
            }
            // Step forward 1 tick (alias for space).
            KeyCode::Char('f') => {
                sim.step(schedule);
                state.ticks = sim.current_tick();
            }
            // Step backward 1 tick (rewinds via factory replay).
            KeyCode::Char('b') => {
                let target = state.ticks.saturating_sub(1);
                return KeyOutcome::Rewind(target);
            }
            // Fast-forward 10 ticks
            KeyCode::Char('>') | KeyCode::Char('.') => {
                for _ in 0..10 {
                    sim.step(schedule);
                }
                state.ticks = sim.current_tick();
                state.flash("ff +10");
            }
            // Rewind 10 ticks
            KeyCode::Char('<') | KeyCode::Char(',') => {
                let target = state.ticks.saturating_sub(10);
                return KeyOutcome::Rewind(target);
            }
            // Rewind to start
            KeyCode::Char('r') => {
                return KeyOutcome::Rewind(0);
            }
            // Scenario picker
            KeyCode::Char('m') => {
                state.mode = Mode::ScenarioMenu;
                state.picker = entries.iter().map(|e| e.name.clone()).collect();
                state.picker_filter.clear();
                state.picker_idx = 0;
            }
            KeyCode::Char('g') => state.mode = Mode::GodMenu,
            _ => {}
        },
        Mode::ScenarioMenu => match code {
            KeyCode::Esc => state.mode = Mode::Normal,
            KeyCode::Up => {
                if state.picker_idx > 0 {
                    state.picker_idx -= 1;
                }
            }
            KeyCode::Down => {
                if state.picker_idx + 1 < filtered(state).len() {
                    state.picker_idx += 1;
                }
            }
            KeyCode::Backspace => {
                state.picker_filter.pop();
                state.picker_idx = 0;
            }
            KeyCode::Char(c) if !c.is_control() => {
                state.picker_filter.push(c);
                state.picker_idx = 0;
            }
            KeyCode::Enter => {
                let filtered_items = filtered(state);
                if let Some(name) = filtered_items.get(state.picker_idx).cloned() {
                    if let Some(idx) = entries.iter().position(|e| e.name == name) {
                        return KeyOutcome::Switch(idx);
                    }
                }
            }
            _ => {}
        },
        Mode::GodMenu => match code {
            KeyCode::Esc => state.mode = Mode::Normal,
            KeyCode::Char('1') => enter_picker(state, Mode::GodSpawnRole, role_choices(sim)),
            KeyCode::Char('2') => enter_picker(state, Mode::GodSpawnItem, item_choices(sim)),
            KeyCode::Char('3') => {
                enter_picker(state, Mode::GodSpawnFurniture, furniture_choices(sim))
            }
            KeyCode::Char('4') => enter_picker(state, Mode::GodSetVoxel, material_choices(sim)),
            KeyCode::Char('5') => {
                kill_entity_at_cursor(sim, state);
                state.mode = Mode::Normal;
            }
            KeyCode::Char('6') => {
                heal_entity_at_cursor(sim, state);
                state.mode = Mode::Normal;
            }
            _ => {}
        },
        Mode::GodSpawnRole | Mode::GodSpawnItem | Mode::GodSpawnFurniture | Mode::GodSetVoxel => {
            match code {
                KeyCode::Esc => {
                    state.mode = Mode::Normal;
                    state.picker.clear();
                    state.picker_filter.clear();
                    state.picker_idx = 0;
                }
                KeyCode::Up => {
                    if state.picker_idx > 0 {
                        state.picker_idx -= 1;
                    }
                }
                KeyCode::Down => {
                    if state.picker_idx + 1 < filtered(state).len() {
                        state.picker_idx += 1;
                    }
                }
                KeyCode::Backspace => {
                    state.picker_filter.pop();
                    state.picker_idx = 0;
                }
                KeyCode::Char(c) if !c.is_control() => {
                    state.picker_filter.push(c);
                    state.picker_idx = 0;
                }
                KeyCode::Enter => {
                    let filtered = filtered(state);
                    if let Some(name) = filtered.get(state.picker_idx).cloned() {
                        match state.mode {
                            Mode::GodSpawnRole => spawn_role_at(sim, state, &name),
                            Mode::GodSpawnItem => spawn_item_at(sim, state, &name),
                            Mode::GodSpawnFurniture => spawn_furn_at(sim, state, &name),
                            Mode::GodSetVoxel => set_voxel_at(sim, state, &name),
                            _ => {}
                        }
                    }
                    state.mode = Mode::Normal;
                    state.picker.clear();
                    state.picker_filter.clear();
                    state.picker_idx = 0;
                }
                _ => {}
            }
        }
    }
    KeyOutcome::Continue
}

fn heal_entity_at_cursor(sim: &mut Simulation, state: &mut AppState) {
    let target: Option<Entity> = {
        let mut q = sim.world.query::<(Entity, &Position, &Health)>();
        q.iter(&sim.world)
            .find(|(_, p, _)| p.0 == state.cursor)
            .map(|(e, _, _)| e)
    };
    if let Some(e) = target {
        if let Some(mut h) = sim.world.get_mut::<Health>(e) {
            h.current = h.max;
        }
        state.flash(format!("healed entity #{}", e.index()));
    } else {
        state.flash("no entity here".to_string());
    }
}

fn handle_mouse(m: event::MouseEvent, terminal: &mut Terminal<Backend>, state: &mut AppState) {
    if !matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
        return;
    }
    // Translate terminal coords back to a map cell. The map sits in
    // the left pane; we know its rect from the layout we use in
    // `draw`. Recompute it here based on the current size.
    let size = match terminal.size() {
        Ok(s) => s,
        Err(_) => return,
    };
    let frame_area = Rect::new(0, 0, size.width, size.height);
    let (map_area, _, _) = layout_areas(frame_area);
    if m.column < map_area.x || m.column >= map_area.x + map_area.width {
        return;
    }
    if m.row < map_area.y || m.row >= map_area.y + map_area.height {
        return;
    }
    let local_x = (m.column - map_area.x) as i32;
    let local_y = (m.row - map_area.y) as i32;
    state.cursor.x = state.ascii.min.x + local_x;
    state.cursor.y = state.ascii.min.y + local_y;
}

fn enter_picker(state: &mut AppState, mode: Mode, items: Vec<String>) {
    state.mode = mode;
    state.picker = items;
    state.picker_filter.clear();
    state.picker_idx = 0;
}

fn filtered(state: &AppState) -> Vec<String> {
    if state.picker_filter.is_empty() {
        state.picker.clone()
    } else {
        let f = state.picker_filter.to_lowercase();
        state
            .picker
            .iter()
            .filter(|n| n.to_lowercase().contains(&f))
            .cloned()
            .collect()
    }
}

fn role_choices(sim: &Simulation) -> Vec<String> {
    sim.world
        .resource::<Library>()
        .list_roles()
        .into_iter()
        .map(String::from)
        .collect()
}
fn item_choices(sim: &Simulation) -> Vec<String> {
    sim.world
        .resource::<Library>()
        .list_items()
        .into_iter()
        .map(String::from)
        .collect()
}
fn furniture_choices(sim: &Simulation) -> Vec<String> {
    sim.world
        .resource::<Library>()
        .list_furniture()
        .into_iter()
        .map(String::from)
        .collect()
}
fn material_choices(sim: &Simulation) -> Vec<String> {
    sim.world
        .resource::<Library>()
        .list_materials()
        .into_iter()
        .map(String::from)
        .collect()
}

fn spawn_role_at(sim: &mut Simulation, state: &mut AppState, name: &str) {
    let opts = RoleSpawnOpts {
        at: state.cursor,
        ..Default::default()
    };
    match spawn_role_template(&mut sim.world, name, opts) {
        Ok(_) => state.flash(format!("spawned role '{name}' at {:?}", state.cursor)),
        Err(e) => state.flash(format!("err: {e}")),
    }
}
fn spawn_item_at(sim: &mut Simulation, state: &mut AppState, name: &str) {
    let opts = ItemSpawnOpts {
        at: Some(state.cursor),
        ..Default::default()
    };
    match spawn_item_template(&mut sim.world, name, opts) {
        Ok(_) => state.flash(format!("spawned item '{name}'")),
        Err(e) => state.flash(format!("err: {e}")),
    }
}
fn spawn_furn_at(sim: &mut Simulation, state: &mut AppState, name: &str) {
    let opts = FurnitureSpawnOpts {
        at: state.cursor,
        kind_label: None,
    };
    match spawn_furniture_template(&mut sim.world, name, opts) {
        Ok(_) => state.flash(format!("spawned furniture '{name}'")),
        Err(e) => state.flash(format!("err: {e}")),
    }
}
fn set_voxel_at(sim: &mut Simulation, state: &mut AppState, name: &str) {
    use fortress_engine::ensure_material;
    let mat = match ensure_material(&mut sim.world, name) {
        Ok(m) => m,
        Err(e) => {
            state.flash(format!("err: {e}"));
            return;
        }
    };
    sim.world
        .resource_mut::<VoxelWorld>()
        .set_voxel(state.cursor, Voxel::wall(mat));
    state.flash(format!("set voxel at {:?} to {name}", state.cursor));
}

fn kill_entity_at_cursor(sim: &mut Simulation, state: &mut AppState) {
    let mut q = sim.world.query::<(Entity, &Position, &Health)>();
    let target: Option<Entity> = q
        .iter(&sim.world)
        .find(|(_, p, h)| p.0 == state.cursor && h.is_alive())
        .map(|(e, _, _)| e);
    if let Some(e) = target {
        if let Some(mut h) = sim.world.get_mut::<Health>(e) {
            h.current = 0;
        }
        state.flash(format!("killed entity #{}", e.index()));
    } else {
        state.flash("nothing alive here".to_string());
    }
}

// ─── rendering ────────────────────────────────────────────────────

fn layout_areas(full: Rect) -> (Rect, Rect, Rect) {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(2)])
        .split(full);
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(v[0]);
    (h[0], h[1], v[1])
}

fn draw(f: &mut ratatui::Frame, sim: &mut Simulation, state: &mut AppState) {
    let (map_area, side_area, footer_area) = layout_areas(f.area());
    draw_map(f, map_area, sim, state);
    draw_inspector(f, side_area, sim, state);
    draw_footer(f, footer_area, state);
    if state.mode != Mode::Normal {
        draw_overlay(f, state);
    }
}

fn draw_map(f: &mut ratatui::Frame, area: Rect, sim: &mut Simulation, state: &AppState) {
    let block = Block::default().borders(Borders::ALL).title(format!(
        " map z={} cursor=({},{}) ",
        state.z, state.cursor.x, state.cursor.y
    ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Draw each cell as a single character with color from material.
    let mut lines: Vec<Line> = Vec::with_capacity(inner.height as usize);
    let z = state.z;
    // Build entity overlay first (queries take &mut), THEN drop into
    // the immutable voxel-world walk for rendering.
    type Overlay = std::collections::HashMap<(i32, i32), (char, [u8; 3])>;
    let mut overlay: Overlay = Default::default();
    type Row = (String, Pos, bool);
    let snapshot: Vec<Row> = {
        let mut q = sim.world.query::<(&Kind, &Position, Option<&Health>)>();
        q.iter(&sim.world)
            .filter_map(|(k, p, h)| {
                let alive = match h {
                    Some(h) if !h.is_alive() => return None,
                    Some(_) => true,
                    None => false,
                };
                Some((k.0.clone(), p.0, alive))
            })
            .collect()
    };
    for (kind_name, pos, is_creature) in snapshot {
        if pos.z != z {
            continue;
        }
        let glyph = state
            .ascii
            .entity_kind_glyphs
            .get(&kind_name)
            .copied()
            .unwrap_or_else(|| {
                if is_creature {
                    state.ascii.default_entity_glyph
                } else {
                    '?'
                }
            });
        let color = if is_creature { [240, 200, 80] } else { [180, 180, 200] };
        overlay.entry((pos.x, pos.y)).or_insert((glyph, color));
    }

    let voxel_world = sim.world.resource::<VoxelWorld>();
    for (row_idx, y) in (state.ascii.min.y..=state.ascii.max.y).enumerate() {
        if row_idx as u16 >= inner.height {
            break;
        }
        let mut spans: Vec<Span> = Vec::new();
        for x in state.ascii.min.x..=state.ascii.max.x {
            let pos = Pos::new(x, y, z);
            let voxel = voxel_world.voxel(pos);
            let mat = voxel_world.material(voxel.material);
            let bg = mat.map(|m| m.color).unwrap_or([20, 20, 24]);
            let (glyph, fg) = match overlay.get(&(x, y)) {
                Some((g, c)) => (*g, *c),
                None => (
                    glyph_for_voxel(&state.ascii, voxel_world, pos),
                    contrast_for(bg),
                ),
            };
            // Highlight the cursor cell with a bright background.
            let on_cursor = x == state.cursor.x && y == state.cursor.y;
            let style = if on_cursor {
                TuiStyle::default()
                    .bg(Color::Rgb(240, 240, 80))
                    .fg(Color::Rgb(20, 20, 24))
                    .add_modifier(Modifier::BOLD)
            } else {
                TuiStyle::default()
                    .bg(Color::Rgb(bg[0], bg[1], bg[2]))
                    .fg(Color::Rgb(fg[0], fg[1], fg[2]))
            };
            spans.push(Span::styled(glyph.to_string(), style));
        }
        lines.push(Line::from(spans));
    }
    let para = Paragraph::new(Text::from(lines));
    f.render_widget(para, inner);
}

fn glyph_for_voxel(ascii: &AsciiRenderer, vw: &VoxelWorld, pos: Pos) -> char {
    use fortress_engine::TileKind;
    let v = vw.voxel(pos);
    match v.kind {
        TileKind::Empty => ascii.air_glyph,
        TileKind::Floor => ascii.floor_glyph,
        TileKind::RampUp => ascii.ramp_glyph,
        TileKind::Wall => *ascii
            .material_glyphs
            .get(&v.material)
            .unwrap_or(&ascii.default_solid_glyph),
    }
}

fn contrast_for(bg: [u8; 3]) -> [u8; 3] {
    let luma = 0.2126 * bg[0] as f32 + 0.7152 * bg[1] as f32 + 0.0722 * bg[2] as f32;
    if luma > 140.0 {
        [16, 16, 24]
    } else {
        [232, 232, 240]
    }
}

fn draw_inspector(f: &mut ratatui::Frame, area: Rect, sim: &mut Simulation, state: &AppState) {
    let block = Block::default().borders(Borders::ALL).title(" inspect ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Voxel info first.
    let (kind_label, mat_name) = {
        let vw = sim.world.resource::<VoxelWorld>();
        let voxel = vw.voxel(state.cursor);
        let n = vw
            .material(voxel.material)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "?".into());
        (format!("{:?}", voxel.kind), n)
    };
    lines.push(Line::from(format!(
        "({}, {}, {})",
        state.cursor.x, state.cursor.y, state.cursor.z
    )));
    lines.push(Line::from(format!("voxel: {kind_label} ({mat_name})")));
    lines.push(Line::from(""));

    // Entities at cursor.
    let entities: Vec<Entity> = {
        let mut q = sim.world.query::<(Entity, &Position)>();
        q.iter(&sim.world)
            .filter(|(_, p)| p.0 == state.cursor)
            .map(|(e, _)| e)
            .collect()
    };

    if entities.is_empty() {
        lines.push(Line::from("(no entities)"));
    } else {
        for e in &entities {
            describe_entity(&mut lines, sim, *e);
            lines.push(Line::from(""));
        }
    }

    let para = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn describe_entity(lines: &mut Vec<Line>, sim: &mut Simulation, e: Entity) {
    let world = &mut sim.world;
    let kind = world
        .get::<Kind>(e)
        .map(|k| k.0.clone())
        .unwrap_or_else(|| format!("entity#{}", e.index()));
    lines.push(Line::from(Span::styled(
        format!("● {kind}#{}", e.index()),
        TuiStyle::default().add_modifier(Modifier::BOLD),
    )));
    if let Some(h) = world.get::<Health>(e) {
        let color = if !h.is_alive() {
            Color::Rgb(160, 60, 60)
        } else if h.current * 3 < h.max {
            Color::Rgb(240, 80, 80)
        } else {
            Color::Rgb(80, 200, 80)
        };
        lines.push(Line::from(Span::styled(
            format!("hp {}/{}", h.current, h.max),
            TuiStyle::default().fg(color),
        )));
    }
    if let Some(q) = world.get::<Quality>(e) {
        let l = q.label();
        if !l.is_empty() {
            lines.push(Line::from(format!("quality: {l}")));
        }
    }
    if let Some(s) = world.get::<Style>(e) {
        let l = s.label();
        if !l.is_empty() {
            lines.push(Line::from(format!("style: {l}")));
        }
    }
    if let Some(v) = world.get::<Value>(e) {
        if v.0 > 0 {
            lines.push(Line::from(format!("value: ${}", v.0)));
        }
    }
    if let Some(w) = world.get::<Wearing>(e) {
        let mut equipped: Vec<String> = Vec::new();
        for (slot, item) in w.iter() {
            let name = world
                .get::<fortress_engine::ItemName>(item)
                .map(|n| n.0.clone())
                .unwrap_or_else(|| format!("#{}", item.index()));
            equipped.push(format!("{}: {}", slot.label(), name));
        }
        if !equipped.is_empty() {
            lines.push(Line::from("equipped:"));
            for s in equipped {
                lines.push(Line::from(format!("  • {s}")));
            }
        }
    }
    if let Some(inv) = world.get::<fortress_engine::Inventory>(e) {
        if !inv.0.is_empty() {
            lines.push(Line::from(format!("inventory ({}):", inv.0.len())));
            for item in inv.0.iter().take(8) {
                let name = world
                    .get::<fortress_engine::ItemName>(*item)
                    .map(|n| n.0.clone())
                    .unwrap_or_else(|| format!("#{}", item.index()));
                lines.push(Line::from(format!("  • {name}")));
            }
            if inv.0.len() > 8 {
                lines.push(Line::from(format!("  …and {} more", inv.0.len() - 8)));
            }
        }
    }
    if let Some(c) = world.get::<fortress_engine::Container>(e) {
        let label = if c.locked {
            format!("container: locked DC {}, {} items", c.lock_dc, c.items.len())
        } else if c.open {
            "container: open".into()
        } else {
            format!("container: closed, {} items", c.items.len())
        };
        lines.push(Line::from(label));
    }
    if let Some(q) = world.get::<TaskQueue>(e) {
        if let Some(t) = q.0.front() {
            lines.push(Line::from(format!("task: {}", t.label())));
        }
    }
    // Wounds: list any non-Intact body parts on this entity.
    let mut parts: Vec<(&'static str, &'static str, i32, i32)> = {
        let mut q = world.query::<(&PartOf, &BodyPartKind, &PartHealth)>();
        q.iter(world)
            .filter(|(p, _, h)| p.0 == e && h.status != PartStatus::Intact)
            .map(|(_, k, h)| (k.label(), h.status.label(), h.current, h.max))
            .collect()
    };
    parts.sort_by_key(|r| r.0);
    if !parts.is_empty() {
        lines.push(Line::from("wounds:"));
        for (part, status, c, m) in parts.into_iter().take(8) {
            lines.push(Line::from(format!("  • {part}: {status} ({c}/{m})")));
        }
    }
}

fn draw_footer(f: &mut ratatui::Frame, area: Rect, state: &mut AppState) {
    let now = Instant::now();
    let flash = match &state.flash {
        Some((m, t)) if now.duration_since(*t) < Duration::from_millis(2500) => {
            Some(m.clone())
        }
        _ => {
            state.flash = None;
            None
        }
    };
    let play = if state.auto_play { "▶ playing" } else { "⏸ paused" };
    let line = if let Some(m) = flash {
        format!(
            "[{label}] {play}  {ms}ms  tick {tick}  z={z}  | {m}  | space/f step • b back1 • </> ±10 • r restart • p play • m menu • g god • q quit",
            label = state.scenario_label,
            ms = state.pace_ms,
            tick = state.ticks,
            z = state.z,
        )
    } else {
        format!(
            "[{label}] {play}  {ms}ms  tick {tick}  z={z}  | arrows cursor • click select • space/f step • b back1 • </> ±10 • r restart • [ ] z • +/- speed • p play • m menu • g god • q quit",
            label = state.scenario_label,
            ms = state.pace_ms,
            tick = state.ticks,
            z = state.z,
        )
    };
    let para = Paragraph::new(line).wrap(Wrap { trim: true });
    f.render_widget(para, area);
}

fn draw_overlay(f: &mut ratatui::Frame, state: &AppState) {
    let area = f.area();
    let popup = centered(area, 60, 70);
    f.render_widget(Clear, popup);
    match state.mode {
        Mode::GodMenu => {
            let block = Block::default().borders(Borders::ALL).title(" GOD MODE ");
            let inner = block.inner(popup);
            f.render_widget(block, popup);
            let lines = vec![
                Line::from(Span::styled(
                    "  spawn / mutate at cursor",
                    TuiStyle::default().add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from("  1) spawn role (creature from a library template)"),
                Line::from("  2) spawn item"),
                Line::from("  3) spawn furniture"),
                Line::from("  4) set voxel material (wall)"),
                Line::from("  5) kill entity at cursor"),
                Line::from("  6) heal entity at cursor (full HP)"),
                Line::from(""),
                Line::from("  esc: cancel"),
            ];
            f.render_widget(Paragraph::new(Text::from(lines)), inner);
        }
        Mode::ScenarioMenu => {
            let block = Block::default().borders(Borders::ALL).title(" SELECT SCENARIO ");
            let inner = block.inner(popup);
            f.render_widget(block, popup);
            let v = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(2), Constraint::Min(1)])
                .split(inner);
            let filter_line = Line::from(vec![
                Span::raw("filter: "),
                Span::styled(
                    state.picker_filter.clone(),
                    TuiStyle::default().add_modifier(Modifier::UNDERLINED),
                ),
                Span::raw("    (type to filter, ↑/↓ to choose, enter to load, esc to cancel)"),
            ]);
            f.render_widget(Paragraph::new(filter_line), v[0]);
            let filtered_items = filtered(state);
            let items: Vec<ListItem> = filtered_items
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    if i == state.picker_idx {
                        ListItem::new(format!("▶ {name}"))
                            .style(TuiStyle::default().add_modifier(Modifier::BOLD))
                    } else {
                        ListItem::new(format!("  {name}"))
                    }
                })
                .collect();
            f.render_widget(List::new(items), v[1]);
        }
        Mode::GodSpawnRole | Mode::GodSpawnItem | Mode::GodSpawnFurniture | Mode::GodSetVoxel => {
            let title = match state.mode {
                Mode::GodSpawnRole => " spawn role ",
                Mode::GodSpawnItem => " spawn item ",
                Mode::GodSpawnFurniture => " spawn furniture ",
                Mode::GodSetVoxel => " set voxel material ",
                _ => "",
            };
            let block = Block::default().borders(Borders::ALL).title(title);
            let inner = block.inner(popup);
            f.render_widget(block, popup);
            let v = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(2), Constraint::Min(1)])
                .split(inner);
            let filter_line = Line::from(vec![
                Span::raw("filter: "),
                Span::styled(
                    state.picker_filter.clone(),
                    TuiStyle::default().add_modifier(Modifier::UNDERLINED),
                ),
                Span::raw("    (type to filter, ↑/↓ to choose, enter to spawn)"),
            ]);
            f.render_widget(Paragraph::new(filter_line), v[0]);
            let filtered_items = filtered(state);
            let items: Vec<ListItem> = filtered_items
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    if i == state.picker_idx {
                        ListItem::new(format!("▶ {name}"))
                            .style(TuiStyle::default().add_modifier(Modifier::BOLD))
                    } else {
                        ListItem::new(format!("  {name}"))
                    }
                })
                .collect();
            f.render_widget(List::new(items), v[1]);
        }
        _ => {}
    }
}

fn centered(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
