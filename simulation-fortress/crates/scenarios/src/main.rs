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

mod family_home;
mod farming;
mod home_invasion;
mod mansion;
mod office;
mod replay;

fn main() {
    let mut args = env::args().skip(1);
    let mut scenario_name: Option<String> = None;
    let mut fast = false;
    let mut repl = false;
    let mut max_ticks: u64 = 200;
    let mut pace_ms: u64 = 400;
    let mut replay_html_path: Option<std::path::PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fast" => fast = true,
            "--repl" => repl = true,
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
                    "usage: fortress [scenario] [--fast] [--repl] [--ticks N] [--pace-ms N] [--replay-html FILE]"
                );
                std::process::exit(2);
            }
        }
    }

    let scenario_name = scenario_name.unwrap_or_else(|| "home_invasion".into());
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
            let ascii = AsciiRenderer::new(Pos::new(-1, -1, 0), Pos::new(16, 14, 0))
                .at_z(0)
                .frame_every(1)
                .faction("invader", 'I')
                .faction("family", 'f')
                .entity_kind("front door", '+')
                .entity_kind("master bedroom door", '+')
                .entity_kind("kid's bedroom door", '+')
                .entity_kind("tv set", 'T')
                .entity_kind("sofa", 's')
                .entity_kind("dining table", 't')
                .entity_kind("refrigerator", 'F')
                .entity_kind("fireplace", '*')
                .entity_kind("locked dresser", 'd');
            run_with_optional_replay(
                &mut sim, &mut scenario, options, ascii, repl,
                replay_html_path.clone(), "mansion home invasion",
            )
        }
        other => {
            eprintln!("unknown scenario: {other}");
            eprintln!("available: home_invasion, farming, office, family_home, mansion");
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
    let log = LogRenderer::default();
    let t = if let Some(path) = replay_path {
        let mut replay = ReplayRenderer::new(ascii.clone(), path.clone(), title);
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
