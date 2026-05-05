# Simulation Fortress: Common Systems

A scratchpad for the cross-cutting systems every scenario can rely on.
The aim is the smallest set of generic primitives that compose into very
different scenarios (D-Day landing, home invasion, dwarven fort, plague
outbreak, hostage situation) without bespoke per-scenario code.

This document is intentionally informal. Each section captures: what the
system is for, what exists today, and the open design questions we still
need to answer. Edit freely.

## Status legend

- **done** — usable, exercised by at least one scenario
- **partial** — scaffolded with a minimal API; deeper behavior pending
- **planned** — discussed but not implemented

## Architectural ground rules

- Engine code lives in `crates/engine` and has no third-party
  dependencies yet. Add deps deliberately.
- Scenarios live in `crates/scenarios` and depend on the engine. Each
  scenario is a `Scenario` impl: `setup` builds the world, `tick`
  advances behavior, `is_complete` ends the run.
- Rendering is decoupled via the `Renderer` trait. The CLI/ASCII
  renderer is the only one today; future renderers (TUI, graphical)
  live in their own crates and never get pulled into the engine.
- Time is integer ticks. A "tick" is the smallest indivisible step;
  scenarios decide what real-world duration that maps to.
- Coordinates are integer voxel cells (`Pos { x, y, z }`). Sub-voxel
  motion can be added later if a scenario needs it.

## 1. Tile / voxel system — partial

3D grid of voxel cells, chunked into 16×16×16 blocks for cache
locality. Each voxel currently stores a `MaterialId` and a `damage`
byte. `World::set_voxel` / `World::voxel` / `World::fill` are the only
mutation primitives.

Code: `crates/engine/src/world.rs`

Open questions:

- How big is a voxel? (1 m feels right for buildings, too coarse for
  bodies — do we need sub-voxel detail or a separate "fine" grid?)
- Sparse storage: should empty (all-air) chunks be elided entirely?
- Lighting / line-of-sight: per-voxel light value, or computed on
  demand?
- Multi-voxel structures (doors, windows, beds) — voxel attribute,
  separate "feature" layer, or entity?

## 2. Materials — partial

Materials are registered on the world and referenced by `MaterialId`.
Today a material has `name`, `solid`, `density`, `flammable`. Air is
material id 0.

Code: `crates/engine/src/world.rs::Material`

Likely additions:

- Thermal: melting point, ignition temperature, conductivity.
- Mechanical: hardness, yield strength (for damage/fracture).
- Electrical: conductivity (for future scenarios with electronics).
- Optical: opacity, color (for renderers).
- Composition: alloys, layered materials (skin over bone, paint over
  wood).

## 3. Fluids — planned

Liquids (water, blood, magma, fuel) and gases (smoke, steam, toxins)
share enough behavior to live under one system. Probably a separate
"flow" pass that runs after entity ticks.

Open questions:

- Cellular automata (DF style: each cell holds 0–7 units, flows by
  level differences) vs. a coarser bulk-fluid model?
- Pressure: does water seek its own level across the map, or only
  locally?
- Mixing: smoke + air, blood + water, fuel + fire — how do reactions
  trigger?
- Performance: fluids are the biggest perf hazard in DF-likes; do we
  cap simulated cells per tick?

## 4. Individuals (creatures, agents) — partial

Today: `Entity { id, kind, position, health, faction, alive, data }`.
The `data: HashMap<String, String>` field is an escape hatch for
scenario-specific fields until we promote them to first-class.

Code: `crates/engine/src/entity.rs`

Promotions to consider:

- `body: Body` (anatomy — see §5).
- `mind: Mind` (personality, mood, memories — see §6, §8).
- `inventory: Inventory` (see §9).
- `skills: Skills` (see §10).
- `species: SpeciesId` for shared traits across many entities.

## 5. Anatomy & wounds — planned

DF tracks body parts, layers (skin, fat, muscle, bone), and per-part
wounds. We probably want something similar but simpler.

Open questions:

- Tree of body parts vs. flat list with parent links?
- Wound types: bruise, cut, fracture, burn, infection — how do they
  interact?
- Bleeding as a tick-driven status effect that depletes health?
- Prosthetics, regrowth, scarring — scenario-toggleable?

## 6. Personality — planned

Each individual has a personality vector influencing decisions: e.g.
bravery, aggression, loyalty, curiosity, greed, empathy. Compatible
with Big Five but framed for game purposes.

Open questions:

- Continuous traits (`f32` per axis) or categorical (`Trait::Brave`)?
- How do traits feed into the AI loop — modifiers on action utility?
- Mood as short-term state on top of long-term traits.
- Do scenarios get to define new traits, or do we ship a fixed set?

## 7. Background & profession — planned

Where the individual came from and what they know. Drives starting
skills, items, relationships, and the histories system.

Likely fields:

- Origin (place / culture).
- Family (parents, siblings, children — links to other individuals).
- Profession history (soldier, farmer, surgeon, smith).
- Trauma & formative events.
- Reputation tags.

Backgrounds should be procedurally generatable so scenarios with many
NPCs don't need hand-written biographies.

## 8. Histories & memories — planned

Two related ideas:

- **World history**: a chronological log of significant events
  (battles, deaths, constructions, weather). Powers post-mortem
  reports and any "legends" view.
- **Personal memory**: each individual remembers events that involved
  them or their relationships. Influences later decisions ("I saw my
  brother killed by the intruder, I won't flee").

Open questions:

- Single global event log with subscribers, or per-entity logs?
- How much is persisted vs. forgotten over time?
- Indexing: how do we ask "did Alice witness Bob's death"?

## 9. Items & inventory — planned

Items are entities-lite: position (or carrier), material, quality,
durability, optional behavior. A crowbar, a rifle, a cookie — same
substrate.

Open questions:

- Items as full entities vs. a separate `Item` type?
- Stacks (50 arrows) vs. discrete items.
- Containers (pockets, boxes, barrels) — recursive inventory tree.
- Wear & repair — same `damage` byte as voxels, or its own model?
- Crafting / decomposition — recipes as data, or scenario code?

## 10. Skills & learning — planned

Per-individual proficiency in named skills (combat, medicine, masonry,
firearms, lockpicking). Skills gate available actions and modify
success rates. Using a skill increases it.

Open questions:

- Numeric level (`u16`) or bucketed (Novice → Legendary)?
- Decay over disuse?
- Skill trees / prerequisites, or flat list?
- Cross-scenario skill registry, or per-scenario.

## 11. Needs, drives & moods — planned

Hunger, thirst, sleep, social contact, safety, purpose. Unmet needs
push entities toward actions that satisfy them. Mood is the running
average of need satisfaction plus recent events.

Open questions:

- How many needs? Maslow-shaped, or DF-style "thoughts and
  preferences" (every entity has a list of things they like and
  dislike, modulating mood)?
- How do we keep this from dominating CPU when there are hundreds of
  agents?

## 12. Relationships & factions — partial

Today: every entity has an optional `faction: String`. That's enough
for the home invasion example (`family` vs `intruder`).

Code: `crates/engine/src/entity.rs::Entity::faction`

Wants to grow into:

- Per-entity directed relationships (Alice→Bob: trust=0.8, fear=0.1).
- Faction stances (allied / neutral / hostile / at-war).
- Diplomacy events that change stances.
- Coalition logic for multi-faction scenarios (D-Day: Allies vs Axis,
  but also French civilians as a third party).

## 13. Combat — planned

Resolves damage when one entity attacks another. Today the home
invasion scenario hard-codes `health -= 25`. We want a generic system
that uses anatomy + items + skills.

Open questions:

- Hit-roll vs. always-hit-but-damage-varies.
- Armor as layered materials over body parts.
- Ranged weapons: ballistics simulated (line of sight + travel time)
  or abstract.
- Morale and retreat.
- Non-lethal options (subdue, intimidate, surrender).

## 14. Pathfinding & navigation — planned

Move entities through the voxel world avoiding solids. A* over the
voxel grid is the obvious starting point, with a chunk-level coarse
graph for long-distance plans.

Open questions:

- How do we handle dynamic obstacles (other entities, doors)?
- 3D pathing across z-levels — stairs, ladders, jumps as graph edges?
- Re-plan cadence vs. cost.
- Group movement (a squad of soldiers).

## 15. Time, weather, seasons — planned

The clock today is a bare tick counter. Scenarios should be able to
attach calendars (mission elapsed time, time-of-day, season). Weather
modifies visibility, movement, fire spread.

Open questions:

- Tick-to-real-time mapping per scenario, or fixed.
- Day/night light propagation.
- Long simulations (a year) vs. short (a single skirmish) — one model
  or two?

## 16. World generation — planned

Procedural construction of the initial world: terrain, buildings,
populations, histories. Scenarios can stitch generators together
(D-Day = beach generator + bunker generator + invasion-force
generator + civilian-village generator).

Open questions:

- Deterministic from a seed (essential for reproducibility & testing).
- Generator API: `fn generate(world: &mut World, rng: &mut Rng,
  params: ...)`.
- Composability: can a generator nest others?

## 17. Events & logging — partial

A structured event stream is emitted by the engine and scenarios. The
`Event` enum currently covers spawns, movement, attacks, deaths,
voxel changes, and free-form notes. The `EventLog` lives on the
`Simulation` and accumulates everything that happens, tagged by tick.

Code: `crates/engine/src/log.rs`,
`crates/engine/src/scenario.rs::TickContext`

Today scenarios mutate via context helpers (`ctx.spawn`,
`ctx.move_entity`, `ctx.damage`, `ctx.note`) that automatically push
the matching event. The `LogRenderer` then turns each tick's events
into a paragraph of prose narration; the runner pairs it with the
`AsciiRenderer` so every tick produces both a map snapshot and a
written description.

Still open:

- Should the engine instrument `World`/`EntityStore` mutations
  directly so scenarios that bypass context helpers still log?
- Replay & snapshot: can we reconstruct a run from the event log
  alone, or do we need periodic snapshots?
- Subscriber model: external listeners (UIs, recorders) plugging in
  alongside renderers.
- Promote `Note` strings to structured event subtypes once we see
  patterns repeat.

## 18. Rendering — done (CLI only)

The `Renderer` trait takes a `&SimulationView` (read-only view of
world, entities, and event log) and produces output. Two concrete
renderers ship today:

- `AsciiRenderer` — prints a single Z-slice per frame with material
  glyphs and entity overlays.
- `LogRenderer` — prints a paragraph of prose narration per tick,
  derived from the event log.

`CompositeRenderer<A, B>` runs two renderers in sequence on each
frame, which the runner uses to combine the two.

Playback pacing is controlled by `RunOptions::pacing`: the runner
sleeps after each frame so a human can read the narration as it
streams. `--fast` on the CLI disables pacing.

Code: `crates/engine/src/render.rs`,
`crates/engine/src/simulation.rs::RunOptions`

Future renderers should live in their own crates so the engine stays
dependency-free. Likely candidates:

- `render-tui`: ratatui-based interactive view with scrolling and
  panels for entity inspection.
- `render-graphical`: bevy/wgpu voxel view for debugging in 3D.

## 19. Determinism & RNG — planned

Many systems above need randomness (combat rolls, world gen, AI
decisions). We want all of it routed through a seeded RNG carried by
the simulation so any run can be replayed.

## 20. Configuration & scripting — planned

Scenarios are Rust code today. That's fine for the engine team, but
we'll want a data-driven path for non-coders: TOML/JSON or a small
scripting language. Trade-off is iteration speed vs. expressiveness.
Defer until we know what real scenarios need.

## How systems compose — example: home invasion

The current home-invasion scenario only exercises §1, §2, §4, §12,
§18. To make it richer we'd pull in:

- §5 (anatomy): the intruder breaks a resident's arm before killing
  them; injured residents fight worse.
- §6 (personality): one resident is a coward and tries to hide; one
  is brave and charges.
- §9 (items): resident grabs a kitchen knife from the counter on the
  way past.
- §10 (skills): firearms skill on a homeowner with a pistol.
- §11 (needs): if the simulation ran long enough, residents would
  also need to eat and sleep — usually irrelevant to a 5-minute
  invasion.
- §13 (combat): replace the hard-coded damage with anatomy-driven
  resolution.
- §17 (events): every blow, scream, and death emits an event the
  renderer can flash on screen.

That layering is the point: each new system adds depth across every
scenario without rewriting them.
