use std::env;
use std::io::{self, BufRead, Write};
use std::time::Duration;

use fortress_engine::{
    apply_json, function_capacity, AsciiRenderer, BodyPartKind, CompositeRenderer, Energy,
    EventLog, Faction, Fear, Function, Health, Hunger, Item, ItemName, Kind, Library,
    LogRenderer, Mood, PartHealth, PartOf, PartStatus, Position, Pos, Renderer, RunOptions,
    Scenario, Simulation, VoxelWorld, Wearing,
};
use replay::ReplayRenderer;

mod replay;
mod scenarios;
mod tui;

// Bring the scenario types into scope under their old names so the
// dispatch and TUI-factory code below reads the same. Each module
// is now `scenarios::airport`, `scenarios::bank`, etc.
use scenarios::{
    airport, bank, cabin, cafeteria, family_home, farming, home_invasion, mansion, office,
};

fn main() {
    let mut args = env::args().skip(1);
    let mut scenario_name: Option<String> = None;
    let mut fast = false;
    let mut repl = false;
    let mut tui_mode = false;
    let mut max_ticks: u64 = 200;
    let mut pace_ms: u64 = 400;
    let mut replay_html_path: Option<std::path::PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fast" => fast = true,
            "--repl" => repl = true,
            "--tui" => tui_mode = true,
            "--ticks" => {
                max_ticks = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(max_ticks);
            }
            "--pace-ms" => {
                pace_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(pace_ms);
            }
            "--replay-html" => {
                replay_html_path = args.next().map(std::path::PathBuf::from);
            }
            other if !other.starts_with("--") && scenario_name.is_none() => {
                scenario_name = Some(other.to_string());
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!(
                    "usage: fortress [scenario] [--fast] [--repl] [--tui] [--ticks N] [--pace-ms N] [--replay-html FILE]"
                );
                std::process::exit(2);
            }
        }
    }

    let scenario_name_for_tui = scenario_name.clone();
    let scenario_name = scenario_name.unwrap_or_else(|| "home_invasion".into());
    set_tui_mode(tui_mode);

    // Interactive TUI dispatch — before any of the headless branches.
    if tui_mode {
        let entries = build_scenario_entries();
        let initial = scenario_name_for_tui
            .as_deref()
            .and_then(|n| entries.iter().position(|e| e.name == n));
        if let Err(e) = tui::run(entries, initial) {
            eprintln!("TUI error: {e}");
        }
        return;
    }
    let pacing = if fast || repl {
        None
    } else {
        Some(Duration::from_millis(pace_ms))
    };
    let options = RunOptions {
        max_ticks,
        pacing,
        ..RunOptions::default()
    };

    let mut sim = Simulation::new();
    let final_tick = match scenario_name.as_str() {
        "home_invasion" => {
            let mut scenario = home_invasion::HomeInvasion;
            let ascii = AsciiRenderer::new(Pos::new(-1, -4, 0), Pos::new(8, 8, 0))
                .at_z(0)
                .frame_every(1)
                .entity_kind("intruder", 'I')
                .faction("family", 'f');
            run_with_optional_replay(
                &mut sim, &mut scenario, options, ascii, repl,
                replay_html_path.clone(), "home invasion",
            )
        }
        "farming" => {
            let mut scenario = farming::Farming;
            let ascii = AsciiRenderer::new(Pos::new(-1, -1, 0), Pos::new(7, 7, 0))
                .at_z(0)
                .frame_every(2)
                .faction("farm", 'F');
            run_with_optional_replay(
                &mut sim, &mut scenario, options, ascii, repl,
                replay_html_path.clone(), "farming",
            )
        }
        "office" => {
            let mut scenario = office::OfficeDrama;
            let ascii = AsciiRenderer::new(Pos::new(0, 0, 0), Pos::new(11, 9, 0))
                .at_z(0)
                .frame_every(5)
                .faction("office", 'o');
            run_with_optional_replay(
                &mut sim, &mut scenario, options, ascii, repl,
                replay_html_path.clone(), "office drama",
            )
        }
        "family_home" => {
            let mut scenario = family_home::FamilyHome;
            let ascii = AsciiRenderer::new(Pos::new(-1, -1, 0), Pos::new(10, 10, 0))
                .at_z(0)
                .frame_every(1)
                .faction("invader", 'I')
                .faction("family", 'f')
                .faction("kid", 'k')
                .faction("feral", 'D');
            run_with_optional_replay(
                &mut sim, &mut scenario, options, ascii, repl,
                replay_html_path.clone(), "family home invasion",
            )
        }
        "mansion" => {
            let mut scenario = mansion::MansionInvasion;
            let mut ascii = AsciiRenderer::new(Pos::new(-2, -2, 0), Pos::new(60, 45, 0))
                .at_z(0)
                .frame_every(1)
                .faction("invader", 'I')
                .faction("family", 'f');
            // Furniture glyphs — tile each archetype with a recognizable letter.
            for (kind, glyph) in [
                ("sofa", 's'), ("armchair", 'a'), ("dining chair", 'h'), ("bench", 'b'),
                ("ottoman", 'o'), ("recliner", 'r'),
                ("king bed", 'B'), ("queen bed", 'B'), ("twin bed", 'b'), ("crib", 'c'),
                ("wardrobe", 'W'), ("dresser", 'D'), ("locked dresser", 'D'),
                ("nightstand", 'n'), ("bookshelf", 'L'), ("china cabinet", 'C'),
                ("safe", 'S'), ("filing cabinet", 'f'),
                ("dining table", 't'), ("coffee table", 'c'), ("desk", 'd'),
                ("kitchen island", 'i'), ("side table", 's'),
                ("tv set", 'T'), ("stereo", 'r'), ("refrigerator", 'F'),
                ("stove", 'O'), ("stove on", 'O'), ("microwave", 'm'), ("dishwasher", 'w'),
                ("washer", 'w'), ("dryer", 'y'),
                ("fireplace", '*'), ("ceiling fan", 'F'),
                ("toilet", 'u'), ("bathroom sink", 'k'), ("kitchen sink", 'K'),
                ("bathtub", 'U'), ("shower", 'H'),
                ("table lamp", 'l'), ("floor lamp", 'L'), ("chandelier", 'X'), ("sconce", 'i'),
                ("painting", 'P'), ("oil portrait", 'P'), ("abstract canvas", 'P'),
                ("photograph", 'p'), ("mirror", 'M'), ("wall clock", 'C'),
                ("persian rug", '_'), ("kitchen mat", '_'), ("bath mat", '_'),
                ("window", 'i'), ("open window", '/'),
                ("sliding glass door", '/'),
                ("potted plant", '%'), ("vase", 'v'), ("books", 'b'), ("candle", 'c'),
                ("staircase up", '>'), ("staircase down", '<'),
                ("column", '|'), ("railing", '-'),
                ("grill", 'g'), ("patio chair", 'h'), ("patio table", 't'),
                ("hammock", '~'), ("mailbox", 'M'), ("garden gnome", 'g'),
                ("pool", '~'), ("hot tub", '@'),
                ("front door", '+'), ("dining room door", '+'), ("kitchen door", '+'),
                ("pantry door", '+'), ("laundry door", '+'), ("guest room door", '+'),
                ("master bedroom door", '+'), ("master bath door", '+'),
                ("master closet door", '+'), ("kid's bedroom door", '+'),
                ("kid's bath door", '+'), ("study door", '+'), ("family room door", '+'),
            ] {
                ascii = ascii.entity_kind(kind, glyph);
            }
            // Powered devices (custom labels in scenarios)
            for kind in [
                "Marantz stereo (vinyl playing)",
                "gas stove (front-left burner on)",
                "playroom TV (cartoons)",
                "family room TV (sports)",
            ] {
                ascii = ascii.entity_kind(kind, 'T');
            }
            run_with_optional_replay_zs(
                &mut sim, &mut scenario, options, ascii, repl,
                replay_html_path.clone(), "mansion home invasion",
                &[0, 1],
            )
        }
        "airport" => {
            let mut scenario = airport::AirportInfiltration;
            let ascii = AsciiRenderer::new(Pos::new(-1, -1, 0), Pos::new(25, 13, 0))
                .at_z(0)
                .frame_every(1)
                .faction("spy", 'S')
                .faction("security", 'G')
                .faction("staff", 's')
                .faction("public", 'p')
                .entity_kind("staff door", '+')
                .entity_kind("X-ray belt", 'X')
                .entity_kind("gate B7", 'B');
            run_with_optional_replay(
                &mut sim, &mut scenario, options, ascii, repl,
                replay_html_path.clone(), "airport infiltration",
            )
        }
        "bank" => {
            let mut scenario = bank::BankHeist;
            let ascii = AsciiRenderer::new(Pos::new(0, 0, 0), Pos::new(40, 33, 0))
                .at_z(0)
                .frame_every(1)
                .faction("robber", 'R')
                .faction("staff", 's')
                .faction("public", 'p')
                .faction("police", 'C')
                .entity_kind("glass front door", '+')
                .entity_kind("back exit", '+')
                .entity_kind("vault door", '+')
                .entity_kind("teller window", 'I')
                .entity_kind("cash register", '$')
                .entity_kind("bank vault", 'V')
                .entity_kind("ATM", 'A')
                .entity_kind("safety deposit box", 'B')
                .entity_kind("getaway van", 'V')
                .entity_kind("police cruiser", 'V');
            run_with_optional_replay(
                &mut sim, &mut scenario, options, ascii, repl,
                replay_html_path.clone(), "bank heist",
            )
        }
        "cabin" => {
            let mut scenario = cabin::CabinAmbush;
            let mut ascii = AsciiRenderer::new(Pos::new(0, 0, 0), Pos::new(30, 30, 0))
                .at_z(0)
                .frame_every(1)
                .faction("retiree", 'O')
                .faction("assassin", 'A');
            for (k, g) in [
                ("cabin door", '+'), ("safe", 'S'), ("getaway van", 'V'),
                ("fireplace", '*'), ("twin bed", 'B'), ("armchair", 'h'),
                ("bookshelf", 'L'), ("books", 'b'), ("side table", 's'),
                ("table lamp", 'l'), ("potted plant", '%'),
                ("oak tree", 'T'), ("pine tree", 't'), ("birch tree", 'T'),
                ("redwood tree", 'T'), ("sapling", 't'), ("bush", '&'),
                ("rock", '*'), ("log", '='), ("stump", 'o'),
                ("camp fire", '*'), ("mailbox", 'M'),
            ] {
                ascii = ascii.entity_kind(k, g);
            }
            run_with_optional_replay(
                &mut sim, &mut scenario, options, ascii, repl,
                replay_html_path.clone(), "retired killer's cabin",
            )
        }
        "cafeteria" => {
            let mut scenario = cafeteria::Cafeteria;
            let ascii = AsciiRenderer::new(Pos::new(0, 0, 0), Pos::new(30, 17, 0))
                .at_z(0).frame_every(1)
                .faction("staff", 's').faction("freshman", 'f').faction("senior", 'S');
            run_with_optional_replay(
                &mut sim, &mut scenario, options, ascii, repl,
                replay_html_path.clone(), "cafeteria food fight",
            )
        }
        other => {
            eprintln!("unknown scenario: {other}");
            eprintln!("available: home_invasion, farming, office, family_home, mansion, airport, bank, cabin, cafeteria");
            std::process::exit(1);
        }
    };

    println!("done at tick {final_tick}");
}

/// Wrap the standard LogRenderer + AsciiRenderer chain, optionally
/// adding a ReplayRenderer that captures every frame and writes a
/// self-contained HTML file at the end of the run.
fn run_with_optional_replay<S: Scenario>(
    sim: &mut Simulation,
    scenario: &mut S,
    options: RunOptions,
    ascii: AsciiRenderer,
    repl: bool,
    replay_path: Option<std::path::PathBuf>,
    title: &str,
) -> u64 {
    if global_tui_mode() {
        // TUI mode is dispatched centrally before scenario branches
        // run; this guard short-circuits any leftover branch path.
        return 0;
    }
    run_with_optional_replay_zs(sim, scenario, options, ascii, repl, replay_path, title, &[])
}

// TUI flag stored in a process-wide static (set once from main, read
// by the scenario branches without changing every signature).
static TUI_MODE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
fn set_tui_mode(on: bool) { let _ = TUI_MODE.set(on); }
fn global_tui_mode() -> bool { TUI_MODE.get().copied().unwrap_or(false) }

/// Build the catalog of factory-backed scenario entries used by the
/// TUI scenario picker (and rewind / restart).
fn build_scenario_entries() -> Vec<tui::ScenarioEntry> {
    use tui::{ScenarioEntry, ScenarioFactory};
    fn entry(
        name: &str,
        factory: ScenarioFactory,
    ) -> ScenarioEntry {
        ScenarioEntry { name: name.to_string(), factory }
    }
    vec![
        entry("home_invasion", Box::new(|| {
            let s = home_invasion::HomeInvasion;
            let ascii = AsciiRenderer::new(Pos::new(-1, -4, 0), Pos::new(8, 8, 0))
                .at_z(0).frame_every(1)
                .entity_kind("intruder", 'I').faction("family", 'f');
            (Box::new(s), ascii)
        })),
        entry("farming", Box::new(|| {
            let s = farming::Farming;
            let ascii = AsciiRenderer::new(Pos::new(-1, -1, 0), Pos::new(7, 7, 0))
                .at_z(0).frame_every(1).faction("farm", 'F');
            (Box::new(s), ascii)
        })),
        entry("office", Box::new(|| {
            let s = office::OfficeDrama;
            let ascii = AsciiRenderer::new(Pos::new(0, 0, 0), Pos::new(11, 9, 0))
                .at_z(0).frame_every(1).faction("office", 'o');
            (Box::new(s), ascii)
        })),
        entry("family_home", Box::new(|| {
            let s = family_home::FamilyHome;
            let ascii = AsciiRenderer::new(Pos::new(-1, -1, 0), Pos::new(10, 10, 0))
                .at_z(0).frame_every(1)
                .faction("invader", 'I').faction("family", 'f')
                .faction("kid", 'k').faction("feral", 'D');
            (Box::new(s), ascii)
        })),
        entry("mansion", Box::new(|| {
            let s = mansion::MansionInvasion;
            let mut ascii = AsciiRenderer::new(Pos::new(-2, -2, 0), Pos::new(60, 45, 0))
                .at_z(0).frame_every(1)
                .faction("invader", 'I').faction("family", 'f');
            for (k, g) in mansion_glyphs() { ascii = ascii.entity_kind(k, g); }
            (Box::new(s), ascii)
        })),
        entry("airport", Box::new(|| {
            let s = airport::AirportInfiltration;
            let ascii = AsciiRenderer::new(Pos::new(-1, -1, 0), Pos::new(25, 13, 0))
                .at_z(0).frame_every(1)
                .faction("spy", 'S').faction("security", 'G')
                .faction("staff", 's').faction("public", 'p')
                .entity_kind("staff door", '+')
                .entity_kind("X-ray belt", 'X')
                .entity_kind("gate B7", 'B');
            (Box::new(s), ascii)
        })),
        entry("bank", Box::new(|| {
            let s = bank::BankHeist;
            let ascii = AsciiRenderer::new(Pos::new(0, 0, 0), Pos::new(40, 33, 0))
                .at_z(0).frame_every(1)
                .faction("robber", 'R').faction("staff", 's')
                .faction("public", 'p').faction("police", 'C')
                .entity_kind("bank vault", 'V').entity_kind("ATM", 'A')
                .entity_kind("cash register", '$').entity_kind("teller window", 'I');
            (Box::new(s), ascii)
        })),
        entry("cabin", Box::new(|| {
            let s = cabin::CabinAmbush;
            let mut ascii = AsciiRenderer::new(Pos::new(0, 0, 0), Pos::new(30, 30, 0))
                .at_z(0).frame_every(1)
                .faction("retiree", 'O').faction("assassin", 'A');
            for (k, g) in [
                ("cabin door", '+'), ("safe", 'S'), ("getaway van", 'V'),
                ("oak tree", 'T'), ("pine tree", 't'), ("birch tree", 'T'),
                ("redwood tree", 'T'), ("bush", '&'), ("rock", '*'),
                ("camp fire", '*'), ("fireplace", '*'),
            ] { ascii = ascii.entity_kind(k, g); }
            (Box::new(s), ascii)
        })),
        entry("cafeteria", Box::new(|| {
            let s = cafeteria::Cafeteria;
            let ascii = AsciiRenderer::new(Pos::new(0, 0, 0), Pos::new(30, 17, 0))
                .at_z(0).frame_every(1)
                .faction("staff", 's').faction("freshman", 'f').faction("senior", 'S');
            (Box::new(s), ascii)
        })),
    ]
}

fn mansion_glyphs() -> Vec<(&'static str, char)> {
    vec![
        ("sofa", 's'), ("armchair", 'a'), ("dining chair", 'h'), ("bench", 'b'),
        ("ottoman", 'o'), ("recliner", 'r'),
        ("king bed", 'B'), ("queen bed", 'B'), ("twin bed", 'b'), ("crib", 'c'),
        ("wardrobe", 'W'), ("dresser", 'D'), ("locked dresser", 'D'),
        ("nightstand", 'n'), ("bookshelf", 'L'), ("china cabinet", 'C'),
        ("safe", 'S'), ("filing cabinet", 'f'),
        ("dining table", 't'), ("coffee table", 'c'), ("desk", 'd'),
        ("kitchen island", 'i'), ("side table", 's'),
        ("tv set", 'T'), ("stereo", 'r'), ("refrigerator", 'F'),
        ("stove on", 'O'), ("stove", 'O'), ("microwave", 'm'), ("dishwasher", 'w'),
        ("washer", 'w'), ("dryer", 'y'), ("fireplace", '*'), ("ceiling fan", 'F'),
        ("toilet", 'u'), ("bathroom sink", 'k'), ("kitchen sink", 'K'),
        ("bathtub", 'U'), ("shower", 'H'),
        ("table lamp", 'l'), ("floor lamp", 'L'), ("chandelier", 'X'),
        ("painting", 'P'), ("oil portrait", 'P'), ("abstract canvas", 'P'),
        ("photograph", 'p'), ("mirror", 'M'), ("wall clock", 'C'),
        ("window", 'i'), ("sliding glass door", '/'),
        ("potted plant", '%'), ("vase", 'v'),
        ("staircase up", '>'),
        ("grill", 'g'), ("patio chair", 'h'), ("patio table", 't'),
        ("hammock", '~'), ("mailbox", 'M'), ("garden gnome", 'g'),
        ("pool", '~'), ("hot tub", '@'),
        ("front door", '+'), ("dining room door", '+'), ("kitchen door", '+'),
        ("master bedroom door", '+'), ("kid's bedroom door", '+'),
    ]
}

fn run_with_optional_replay_zs<S: Scenario>(
    sim: &mut Simulation,
    scenario: &mut S,
    options: RunOptions,
    ascii: AsciiRenderer,
    repl: bool,
    replay_path: Option<std::path::PathBuf>,
    title: &str,
    z_slices: &[i32],
) -> u64 {
    if global_tui_mode() {
        // TUI dispatch happens in main(); shouldn't reach here.
        let _ = (scenario, ascii);
        return 0;
    }
    let log = LogRenderer::default();
    let t = if let Some(path) = replay_path {
        let mut replay = ReplayRenderer::new(ascii.clone(), path.clone(), title);
        if !z_slices.is_empty() {
            replay = replay.with_z_slices(z_slices.iter().copied());
        }
        let mut renderer = CompositeRenderer(CompositeRenderer(log, ascii), &mut replay);
        let t = drive(sim, scenario, options, &mut renderer, repl);
        if let Err(e) = replay.flush_html() {
            eprintln!("failed to write replay html: {e}");
        } else {
            eprintln!("[replay] wrote {} ({} frames)", path.display(), replay.frame_count());
        }
        t
    } else {
        let mut renderer = CompositeRenderer(log, ascii);
        drive(sim, scenario, options, &mut renderer, repl)
    };
    print_summary(scenario, sim, t);
    t
}

/// Drive a scenario either headlessly via `Simulation::run_with` or
/// interactively via a stdin REPL that pauses between ticks.
fn drive<S: Scenario, R: Renderer>(
    sim: &mut Simulation,
    scenario: &mut S,
    options: RunOptions,
    renderer: &mut R,
    repl: bool,
) -> u64 {
    if !repl {
        return sim.run_with(scenario, options, renderer);
    }

    let mut schedule = sim.prepare(scenario, options);
    renderer.frame(&mut sim.world, 0);
    print_repl_help();

    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let mut input = String::new();
    let mut ticks_taken = 0u64;

    loop {
        if ticks_taken >= options.max_ticks {
            println!("[max ticks reached]");
            break;
        }
        if scenario.is_complete(&mut sim.world) {
            println!("[scenario complete]");
            break;
        }

        let tick = sim.current_tick();
        print!("> tick {tick}: ");
        io::stdout().flush().ok();
        input.clear();
        match stdin.read_line(&mut input) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("stdin error: {e}");
                break;
            }
        }
        let line = input.trim();
        match line {
            "" | "step" | "s" => {
                let next = sim.step(&mut schedule);
                renderer.frame(&mut sim.world, next);
                ticks_taken += 1;
            }
            "q" | "quit" | "exit" => break,
            "help" | "?" => print_repl_help(),
            "library" | "lib" => print_library_overview(sim.world.resource::<Library>()),
            cmd if cmd.starts_with("library ") || cmd.starts_with("lib ") => {
                let rest = cmd.split_once(' ').map(|p| p.1.trim()).unwrap_or("");
                let lib = sim.world.resource::<Library>();
                match rest {
                    "materials" => list_library_category("materials", lib.list_materials()),
                    "items" => list_library_category("items", lib.list_items()),
                    "body_plans" | "plans" => {
                        list_library_category("body plans", lib.list_body_plans())
                    }
                    "roles" => list_library_category("roles", lib.list_roles()),
                    query => {
                        let hits = lib.search(query);
                        if hits.is_empty() {
                            println!("no library entries match \"{query}\"");
                        } else {
                            println!("library hits for \"{query}\":");
                            for hit in hits {
                                println!("  [{}] {}", hit.category(), hit.name());
                            }
                        }
                    }
                }
            }
            cmd if cmd.starts_with("go ") => {
                if let Ok(n) = cmd[3..].trim().parse::<u64>() {
                    for _ in 0..n {
                        if scenario.is_complete(&mut sim.world) {
                            break;
                        }
                        if ticks_taken >= options.max_ticks {
                            break;
                        }
                        let next = sim.step(&mut schedule);
                        renderer.frame(&mut sim.world, next);
                        ticks_taken += 1;
                    }
                } else {
                    println!("usage: go N");
                }
            }
            json => match apply_json(&mut sim.world, json) {
                Ok(messages) => {
                    for m in messages {
                        println!("ok: {m}");
                    }
                }
                Err(e) => println!("error: {e}"),
            },
        }
    }

    sim.current_tick()
}

fn print_repl_help() {
    println!("commands:");
    println!("  <enter>, step, s              advance one tick");
    println!("  go N                          advance N ticks");
    println!("  <json>                        inject one Action (or array) and stay");
    println!("  library                       overview of all library categories");
    println!("  library materials|items|...   list one category");
    println!("  library <query>               search library by substring");
    println!("  q, quit, exit                 end the simulation");
    println!("  help, ?                       show this help");
    println!();
}

fn print_library_overview(lib: &Library) {
    println!("library:");
    for (name, count) in lib.category_names() {
        println!("  {name:>12}: {count} entries");
    }
    println!("  use `library <category>` or `library <search query>` to explore.");
}

fn list_library_category(label: &str, names: Vec<&str>) {
    println!("{label} ({}):", names.len());
    for name in names {
        println!("  {name}");
    }
}

fn print_summary<S: Scenario>(scenario: &S, sim: &mut Simulation, tick: u64) {
    println!("=== summary ===");
    println!("scenario:    {}", scenario.name());
    println!("ticks:       {tick}");

    let voxel_chunks = sim.world.resource::<VoxelWorld>().chunk_count();
    let log_entries = sim.world.resource::<EventLog>().len();
    println!("chunks:      {voxel_chunks}");
    println!("log entries: {log_entries}");

    let item_count = {
        let mut q = sim.world.query::<&Item>();
        q.iter(&sim.world).count()
    };
    println!("items:       {item_count}");

    type CreatureRow = (
        bevy_ecs::entity::Entity,
        bool,
        String,
        i32,
        String,
        Pos,
        Vec<(String, String)>,
    );
    let creature_rows: Vec<CreatureRow> = {
        let mut q = sim.world.query::<(
            bevy_ecs::entity::Entity,
            &Kind,
            &Position,
            &Health,
            Option<&Faction>,
            Option<&Wearing>,
        )>();
        q.iter(&sim.world)
            .map(|(entity, kind, pos, health, faction, wearing)| {
                let equipment = wearing
                    .map(|w| {
                        w.iter()
                            .map(|(slot, item)| {
                                let name = sim
                                    .world
                                    .get::<ItemName>(item)
                                    .map(|n| n.0.clone())
                                    .unwrap_or_else(|| format!("item#{}", item.index()));
                                (slot.label().to_string(), name)
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                (
                    entity,
                    health.is_alive(),
                    kind.0.clone(),
                    health.current,
                    faction.map(|f| f.0.clone()).unwrap_or_else(|| "-".into()),
                    pos.0,
                    equipment,
                )
            })
            .collect()
    };

    let total = creature_rows.len();
    let alive = creature_rows.iter().filter(|r| r.1).count();
    println!("entities:    {total}");
    println!("alive:       {alive}");
    for (entity, is_alive, kind, hp, faction, pos, equipment) in creature_rows {
        let status = if is_alive { "alive" } else { "dead " };
        println!(
            "  [{}] {:10} hp={:>4} faction={:<8} pos=({:>3},{:>3},{:>3})",
            status, kind, hp, faction, pos.x, pos.y, pos.z,
        );
        let damaged = collect_damaged_parts(sim, entity);
        if !damaged.is_empty() {
            println!("           wounds:");
            for (part, status, current, max) in damaged {
                println!("             {} [{}] ({}/{} hp)", part, status, current, max);
            }
        }
        let functions = creature_function_summary(sim, entity);
        if !functions.is_empty() {
            print!("           senses:");
            for (function, capacity) in functions {
                print!(" {}={:.1}", function, capacity);
            }
            println!();
        }
        let needs = creature_needs_summary(sim, entity);
        if !needs.is_empty() {
            println!("           needs:  {needs}");
        }
        for (slot, name) in equipment {
            println!("           {slot:>9}: {name}");
        }
    }
}

fn creature_needs_summary(sim: &Simulation, entity: bevy_ecs::entity::Entity) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(h) = sim.world.get::<Hunger>(entity) {
        parts.push(format!("hunger {:.2}", h.current));
    }
    if let Some(e) = sim.world.get::<Energy>(entity) {
        parts.push(format!("energy {:.2}", e.current));
    }
    if let Some(f) = sim.world.get::<Fear>(entity) {
        parts.push(format!("fear {:.2}", f.current));
    }
    if let Some(m) = sim.world.get::<Mood>(entity) {
        parts.push(format!("mood {:.2} ({})", m.current, m.label()));
    }
    parts.join("  ")
}

fn collect_damaged_parts(
    sim: &mut Simulation,
    creature: bevy_ecs::entity::Entity,
) -> Vec<(&'static str, &'static str, i32, i32)> {
    let mut q = sim
        .world
        .query::<(&PartOf, &BodyPartKind, &PartHealth)>();
    let mut rows: Vec<(&'static str, &'static str, i32, i32)> = q
        .iter(&sim.world)
        .filter(|(parent, _, ph)| parent.0 == creature && ph.status != PartStatus::Intact)
        .map(|(_, kind, ph)| (kind.label(), ph.status.label(), ph.current, ph.max))
        .collect();
    rows.sort_by_key(|r| r.0);
    rows
}

fn creature_function_summary(
    sim: &mut Simulation,
    creature: bevy_ecs::entity::Entity,
) -> Vec<(&'static str, f32)> {
    [
        Function::Vision,
        Function::Hearing,
        Function::Smell,
        Function::Speech,
        Function::Grasp,
        Function::Mobility,
        Function::Vitality,
        Function::Breathing,
    ]
    .into_iter()
    .map(|f| (f.label(), function_capacity(&mut sim.world, creature, f)))
    .collect()
}
