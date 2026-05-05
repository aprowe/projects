use std::env;
use std::time::Duration;

use fortress_engine::{
    AsciiRenderer, CompositeRenderer, LogRenderer, Pos, RunOptions, Scenario, Simulation,
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
            let mut scenario = home_invasion::HomeInvasion::default();
            let log = LogRenderer::default();
            let ascii = AsciiRenderer::new(Pos::new(-1, -4, 0), Pos::new(8, 8, 0))
                .at_z(0)
                .frame_every(1)
                .entity_kind("intruder", 'I')
                .faction("family", 'f');
            let mut renderer = CompositeRenderer(log, ascii);
            let t = sim.run_with(&mut scenario, options, &mut renderer);
            print_summary(&scenario, &sim, t);
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

fn print_summary<S: Scenario>(scenario: &S, sim: &Simulation, tick: u64) {
    println!("=== summary ===");
    println!("scenario:    {}", scenario.name());
    println!("ticks:       {tick}");
    println!("chunks:      {}", sim.world.chunk_count());
    println!("entities:    {}", sim.entities.len());
    println!("log entries: {}", sim.log.len());
    let alive = sim.entities.iter().filter(|e| e.is_alive()).count();
    println!("alive:       {alive}");
    for entity in sim.entities.iter() {
        let faction = entity.faction.as_deref().unwrap_or("-");
        let status = if entity.is_alive() { "alive" } else { "dead " };
        println!(
            "  [{}] {:10} hp={:>4} faction={:<8} pos=({:>3},{:>3},{:>3})",
            status, entity.kind, entity.health, faction,
            entity.position.x, entity.position.y, entity.position.z,
        );
    }
}
