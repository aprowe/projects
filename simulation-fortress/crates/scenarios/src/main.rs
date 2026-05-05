use std::env;
use std::time::Duration;

use fortress_engine::{
    AsciiRenderer, CompositeRenderer, EventLog, Faction, Health, Item, ItemName, Kind,
    LogRenderer, Position, Pos, RunOptions, Scenario, Simulation, VoxelWorld, Wearing,
};

mod home_invasion;

fn main() {
    let mut args = env::args().skip(1);
    let mut scenario_name: Option<String> = None;
    let mut fast = false;
    let mut max_ticks: u64 = 200;
    let mut pace_ms: u64 = 400;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fast" => fast = true,
            "--ticks" => {
                max_ticks = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(max_ticks);
            }
            "--pace-ms" => {
                pace_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(pace_ms);
            }
            other if !other.starts_with("--") && scenario_name.is_none() => {
                scenario_name = Some(other.to_string());
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!("usage: fortress [scenario] [--fast] [--ticks N] [--pace-ms N]");
                std::process::exit(2);
            }
        }
    }

    let scenario_name = scenario_name.unwrap_or_else(|| "home_invasion".into());
    let pacing = if fast {
        None
    } else {
        Some(Duration::from_millis(pace_ms))
    };
    let options = RunOptions { max_ticks, pacing };

    let mut sim = Simulation::new();
    let final_tick = match scenario_name.as_str() {
        "home_invasion" => {
            let mut scenario = home_invasion::HomeInvasion;
            let log = LogRenderer::default();
            let ascii = AsciiRenderer::new(Pos::new(-1, -4, 0), Pos::new(8, 8, 0))
                .at_z(0)
                .frame_every(1)
                .entity_kind("intruder", 'I')
                .faction("family", 'f');
            let mut renderer = CompositeRenderer(log, ascii);
            let t = sim.run_with(&mut scenario, options, &mut renderer);
            print_summary(&scenario, &mut sim, t);
            t
        }
        other => {
            eprintln!("unknown scenario: {other}");
            eprintln!("available: home_invasion");
            std::process::exit(1);
        }
    };

    println!("done at tick {final_tick}");
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

    type CreatureRow = (bool, String, i32, String, Pos, Vec<(String, String)>);
    let creature_rows: Vec<CreatureRow> = {
        let mut q = sim
            .world
            .query::<(&Kind, &Position, &Health, Option<&Faction>, Option<&Wearing>)>();
        q.iter(&sim.world)
            .map(|(kind, pos, health, faction, wearing)| {
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
    let alive = creature_rows.iter().filter(|r| r.0).count();
    println!("entities:    {total}");
    println!("alive:       {alive}");
    for (is_alive, kind, hp, faction, pos, equipment) in creature_rows {
        let status = if is_alive { "alive" } else { "dead " };
        println!(
            "  [{}] {:10} hp={:>4} faction={:<8} pos=({:>3},{:>3},{:>3})",
            status, kind, hp, faction, pos.x, pos.y, pos.z,
        );
        for (slot, name) in equipment {
            println!("        {slot:>10}: {name}");
        }
    }
}
