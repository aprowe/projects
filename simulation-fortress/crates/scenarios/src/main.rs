use std::env;
use std::time::Duration;

use fortress_engine::{
    function_capacity, AsciiRenderer, BodyPartKind, CompositeRenderer, Energy, EventLog, Faction,
    Fear, Function, Health, Hunger, Item, ItemName, Kind, LogRenderer, Mood, PartHealth, PartOf,
    PartStatus, Position, Pos, RunOptions, Scenario, Simulation, VoxelWorld, Wearing,
};

mod farming;
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
    let options = RunOptions {
        max_ticks,
        pacing,
        ..RunOptions::default()
    };

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
        "farming" => {
            let mut scenario = farming::Farming;
            let log = LogRenderer::default();
            let ascii = AsciiRenderer::new(Pos::new(-1, -1, 0), Pos::new(7, 7, 0))
                .at_z(0)
                .frame_every(2)
                .faction("farm", 'F');
            let mut renderer = CompositeRenderer(log, ascii);
            let t = sim.run_with(&mut scenario, options, &mut renderer);
            print_summary(&scenario, &mut sim, t);
            t
        }
        other => {
            eprintln!("unknown scenario: {other}");
            eprintln!("available: home_invasion, farming");
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
