# Simulation Fortress: Common Systems

A scratchpad for the cross-cutting systems every scenario can rely on.
The aim is the smallest set of generic primitives that compose into very
different scenarios (D-Day landing, home invasion, dwarven fort, plague
outbreak, hostage situation) without bespoke per-scenario code.

This document is intentionally informal. Each section captures: what the
system is for, what exists today, and the open design questions we still
need to answer. Edit freely.

For the *workflow* of building a new scenario on top of these systems,
see [`AUTHORING.md`](AUTHORING.md).

## Status legend

- **done** — usable, exercised by at least one scenario
- **partial** — scaffolded with a minimal API; deeper behavior pending
- **planned** — discussed but not implemented

## Architectural ground rules

- Engine code lives in `crates/engine` and depends on `bevy_ecs` for
  the entity-component-system core. Other deps stay deliberately
  minimal so we can keep iterating on simulation details rather than
  fighting integrations.
- The simulation owns one `bevy_ecs::World`. Voxel terrain, the clock,
  and the event log live as `Resource`s on it; creatures and items
  live as entities with components.
- Scenarios live in `crates/scenarios`. Each scenario is a `Scenario`
  impl: `setup` populates the world, `build_schedule` returns a bevy
  `Schedule` of systems that run once per tick, `is_complete` ends
  the run.
- Per-tick logic is written as ECS systems (`fn(Res<X>, Query<...>)`
  signatures), not free functions. Scenarios can register their own
  components, resources, and events on the world during setup.
- Rendering is decoupled via the `Renderer` trait. The CLI/ASCII
  renderer plus a prose `LogRenderer` are the only ones today; future
  renderers (TUI, graphical) live in their own crates and never get
  pulled into the engine.
- Time is integer ticks. A "tick" is the smallest indivisible step;
  scenarios decide what real-world duration that maps to.
- Coordinates are integer voxel cells (`Pos { x, y, z }`). Sub-voxel
  motion can be added later if a scenario needs it.

## 1. Tile / voxel system — partial

3D grid of voxel cells, chunked into 16×16×16 blocks for cache
locality. Each voxel stores a `TileKind` (Empty / Floor / Wall /
RampUp), a `MaterialId`, and a `damage` byte. The kind decides the
geometry and walkability of the cell:

- **Wall** — full solid block; impassable, opaque.
- **Floor** — zero-height walkable surface (DF-style); occupiable but
  not solid.
- **Empty** — open air; walkable only when the cell directly below is
  a Wall or RampUp (you stand on the top surface of the block beneath).
- **RampUp** — sloped, walkable, will eventually let pathfinding
  traverse Z levels.

`World::is_walkable` and `World::is_solid` express these rules.
`World::set_voxel`, `voxel`, and `fill` are the mutation primitives.

Code: `crates/engine/src/world.rs`

Open questions:

- How big is a voxel? (1 m feels right for buildings, too coarse for
  bodies — do we need sub-voxel detail or a separate "fine" grid?)
- Sparse storage: should empty (all-air) chunks be elided entirely?
- Lighting / line-of-sight: per-voxel light value, or computed on
  demand?
- Multi-voxel structures (doors, windows, beds) — extend `TileKind`,
  separate "feature" layer, or entity?
- Z-level traversal: ramp pathing, stairs, ladders, jumps.

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

Creatures are bevy ECS entities with a small set of standard
components: `Kind` (string label), `Position`, `Health { current, max }`,
optional `Faction`, optional `ExtraData` (string-keyed escape hatch).
Scenarios add their own marker components like `Intruder`, `Family`,
`Conscript` to drive system queries.

Code: `crates/engine/src/components.rs`

Adding new aspects of a creature is just a new `Component`:

- Anatomy → `Body` component plus a tree of body-part child entities (§5).
- Mind → `Personality`, `Mood`, `Memory` components (§6, §8).
- Skills → `Skills(HashMap<SkillId, u16>)` component (§10).
- Species → `Species(SpeciesId)` for shared traits across many
  individuals.

## 5. Anatomy & wounds — partial

Body parts are ECS entities, not nested data. Bodies are described
as data via `BodyPlan { parts: Vec<BodyPartSpec> }` and applied with
`apply_body_plan(world, creature, plan)`. Each spawned part carries:

- `BodyPart` marker.
- `BodyPartKind` — broad enum (Head, LeftEye, Torso, LeftWing,
  Tail, Horn, Claw, …, plus `Other` as a catch-all).
- `PartOf(Entity)` backlink.
- `PartHealth { current, max, status }`.
- `HitWeight(u32)` for weighted random combat targeting.
- `ProvidesFunctions(Vec<Function>)` — what abilities the part
  contributes when intact.
- Optional `PartLabel(String)` for display ("front-left paw",
  "venomous fang", "scaly hide").

`PartStatus` covers Intact, Bruised, Cut, Broken, Crushed, Severed,
each with a `capacity()` factor. `function_capacity(world,
creature, function)` aggregates intact contributions across the
creature's parts. Recognised functions today: Vision, Hearing,
Smell, Speech, Grasp, Mobility, Vitality, Breathing, Flight.

Preset body plans:

- `humanoid_body_plan()` — 22 parts, the canonical layout.
- `quadruped_body_plan()` — head + torso + 4 legs/paws + tail; no
  hands. Reuses `LeftArm`/`RightArm` enum variants for the rear
  pair, with `PartLabel` set to "rear-left leg" etc.
- `dragon_body_plan()` — heavy head with horns, wings (granting
  Flight), foreclaws, large tail. ~24 parts total.

Code: `crates/engine/src/anatomy.rs`

Open questions:

- Layered materials (skin / fat / muscle / bone) — extra components
  per part, or sub-entities?
- Bleeding as a tick-driven status effect that depletes aggregate
  Health.
- Cascading damage: a destroyed Torso should imperil Heart and
  Lungs.
- Prosthetics, regrowth, scarring — scenario-toggleable?
- Body-plan registry as a `Resource` so plans can be addressed by
  name from configs / scripts (Scribblenauts-style spawning).

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

## 9. Items, inventory & clothing — partial

Items are full ECS entities tagged with the `Item` marker plus an
`ItemName` and whichever physical-trait components apply. The trait
set is open and grows by adding new `Component`s rather than editing
the engine. Today's traits:

- `Mass(f32)` — kilograms.
- `Temperature(f32)` — degrees Celsius (passive; equilibration system
  pending).
- `ThermalConductivity(f32)` — W/(m·K).
- `ElectricalConductivity(f32)` — S/m (loose order-of-magnitude
  number, not strictly unit-checked).
- `Texture` — enum (Smooth, Rough, Coarse, Soft, Sharp, Slick,
  Sticky, Furry, Polished, Bumpy).
- `ItemMaterial(MaterialId)` — link back to the material registry.

Carrying & wearing:

- `Inventory(Vec<Entity>)` on a creature lists items they carry
  loosely.
- `Wearing(HashMap<BodySlot, Entity>)` lists currently equipped items
  by body slot. Slots: Head, Torso, Legs, Feet, Hands, MainHand,
  OffHand, Back.
- Items declare where they go via `Wearable(BodySlot)`.
- Helpers `give_item`, `equip_item`, `unequip_item`, `drop_item` keep
  the components consistent and emit `ItemTaken`, `ItemEquipped`,
  `ItemUnequipped`, `ItemDropped` events.

Code: `crates/engine/src/items.rs`

Demonstrated in the home invasion scenario: residents wear a wool
shirt (Torso) + leather boots (Feet); the intruder grips a steel
crowbar (MainHand). Each piece has full physical-trait components,
ready to feed into combat (§13), thermal (future), and electrical
systems.

Open questions:

- Stacks (50 arrows) vs. discrete items.
- Containers (pockets, boxes, barrels) — recursive entity hierarchy
  via bevy ECS relationships.
- Wear & repair — `Damage(u8)` component, or shared with voxel damage?
- Crafting / decomposition — recipes as data, or scenario code?
- Clothing layers (long underwear under a coat under a cloak) — a
  list of items per slot rather than a single entity.
- Hot/cold transfer between worn items, body, and environment.

## 10. Skills & learning — planned

Per-individual proficiency in named skills (combat, medicine, masonry,
firearms, lockpicking). Skills gate available actions and modify
success rates. Using a skill increases it.

Open questions:

- Numeric level (`u16`) or bucketed (Novice → Legendary)?
- Decay over disuse?
- Skill trees / prerequisites, or flat list?
- Cross-scenario skill registry, or per-scenario.

## 10b. Task system — partial

Three-layer model for what an actor wants and is doing:

- **Behavior** — the long-running role identity. Implemented as a
  scenario-defined marker component (`Intruder`, `Family`, `Farmer`,
  `Customer`). Drives planner systems.
- **Goal** — current high-level intent. The engine ships a small
  enum (`Goal::Idle / GoTo / Kill / Tend`) that scenarios can use
  directly or supplement with their own goal-like components.
- **TaskQueue** — `VecDeque<Task>` component on actors. The engine
  executor (`execute_tasks`) advances the head task each tick.

Built-in `Task` variants the executor knows how to run:

- `MoveTo(Pos)` — uses the pathfinder; completes on arrival,
  fails if no path.
- `Attack(Entity)` — auto-pursues by stepping toward the target
  when not adjacent; calls `resolve_attack` when adjacent;
  completes on target death.
- `Wait(u32)` — decrements each tick.
- `UseEntity(Entity)` — emits `Event::EntityUsed { user, target }`
  and completes; scenarios react to the event to apply effects
  (advance crop growth, trigger lever, etc).
- `PickUp(Entity)` / `Equip(Entity)` — call the corresponding item
  helpers and complete the same tick.

Failed tasks pop and emit `Event::TaskFailed { entity, reason }`,
narrated by the prose log renderer.

Two opt-in helpers ride along:

- `RetaliateOnAttack` — marker component. When the wearer is hit,
  `retaliation_system` queues `Task::Attack(attacker)` at the front
  of its queue (one-tick lag — retaliation lands the next tick).
- `retaliation_system` — engine system you register in the schedule
  alongside `execute_tasks`.

Code: `crates/engine/src/tasks.rs`

**Demonstrated in two scenarios:**

- *Home invasion* — `intruder_planner` sets `Goal::Kill(target)`
  and queues `Task::Attack(target)`. The executor handles
  pathfinding and combat. Family members carry `RetaliateOnAttack`,
  so the retaliation system queues counter-attacks driven by the
  same task machinery.
- *Farming* — `farmer_planner` sets `Goal::Tend(crop)` and queues
  `[MoveTo(crop_pos), UseEntity(crop)]`. A scenario-local
  `advance_used_crops` system listens for `EntityUsed` events on
  crop entities and ticks each crop through five growth stages
  (seed → sprout → growing → ripe → harvested). Two farmers
  cooperatively harvest a 4x4 field in ~100 ticks.

Still open:

- **Plan generation**: today scripted per-behavior; GOAP-style
  planning (preconditions + effects) waits until a scenario
  demands it.
- **Interruption priorities**: scenarios call `clear()` themselves
  when a higher-priority goal preempts. A formal priority system
  would help once we have multi-goal behaviors.
- **More built-in variants**: `Drop`, `Talk`, `Build(Pos, Voxel)`,
  `Eat`, `Sleep`, `Acquire(ItemKind)` (find-then-pickup). Add as
  scenarios need them.
- **Long-running task progress**: some tasks (cook a meal,
  construct a wall) want explicit progress tracking instead of
  ad-hoc `Wait`.

## 11. Needs, drives & moods — partial

Each need is its own component on a creature, so scenarios opt in to
whichever ones make sense (a soldier doesn't need sleep in a
30-minute skirmish; a zombie has no hunger of the human kind).

Implemented today:

- `Hunger { current, rate }` — drifts up toward 1.0; `feed(amount)`
  pushes it back. Helpers: `is_hungry()` (≥0.6), `is_starving()`
  (≥0.95).
- `Energy { current, drain_rate }` — drifts down toward 0.0;
  `rest(amount)` refills. Helpers: `is_tired()`, `is_exhausted()`.
- `Fear { current, decay_rate }` — decays toward 0.0;
  `frighten(amount)` spikes it. Helpers: `is_afraid()`,
  `is_terrified()`.
- `Mood` — derived value in `[-1, 1]`. The default `derive_mood`
  system mixes hunger, fear, and energy into a score; scenarios can
  write their own formula.

Engine systems (run them in your scenario's schedule):

- `tick_needs` — drifts every need each tick.
- `derive_mood` — recomputes Mood from the others.

Code: `crates/engine/src/needs.rs`

**Demonstrated in two scenarios:**

- *Farming* — farmers carry `Hunger`, `Energy`, `Mood`. Each tick
  hunger creeps up; when it crosses 0.6 the planner switches goal
  to `Goal::Eat(kitchen)` and queues `[MoveTo(larder), UseEntity
  (larder)]`. The scenario's `consume_at_kitchen` system listens for
  `EntityUsed` on the larder and resets hunger. Two lunch breaks
  fit naturally into the working day.
- *Home invasion* — residents carry `Fear` (and `Mood`). A
  scenario-local `frighten_attacked` system reads `BodyPartWounded`
  events and spikes the victim's fear, with a one-shot "wide-eyed
  with terror" note when the threshold is crossed. Final summary
  shows residents ending with high fear and "miserable" mood.

Still open:

- More needs: thirst, social contact, safety, purpose.
- Need-driven planning utility scoring (today scenarios test
  `is_hungry()` directly; a generic utility-AI selector would let
  multiple needs compete).
- Performance with hundreds of agents — currently every need ticks
  every entity every tick. Batched / cached approaches when scale
  matters.
- Mood "stickiness" — current value is recomputed fresh each tick;
  drifting toward the derived value would make individual events
  have lasting effect.

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

## 13. Combat — partial

`resolve_attack(world, attacker, target)` is the generic blow:

1. Inspect the attacker's `MainHand` slot. If a weapon is held, read
   its `Mass` and `Texture`; otherwise model a fist (mass 0.5,
   texture Soft).
2. Compute damage: `mass * 10` rounded, scaled by a texture
   modifier (Sharp 1.5, Polished/Smooth 1.0, Rough/Coarse/Bumpy 0.9,
   Sticky/Slick 0.7, Soft/Furry 0.5).
3. Pick a body part on the target by weighted random over `HitWeight`
   among non-destroyed parts (using the seeded `Rng` resource).
4. Apply damage to the chosen part's `PartHealth`. The new
   `PartStatus` follows from how much HP is left and whether the
   weapon is sharp: full → Intact → Bruised → Broken/Cut → Crushed
   /Severed.
5. Apply half the damage to the creature's aggregate `Health`.
6. If a critical part (Heart, Head, Neck) is destroyed, drop
   aggregate Health to 0.
7. Emit `BodyPartWounded`, optionally `BodyPartDestroyed`, then
   `EntityAttacked`, and `EntityKilled` if the creature is no longer
   alive.

Code: `crates/engine/src/combat.rs`,
`crates/engine/src/rng.rs`

Demonstrated in the home invasion: residents' fists (0.5 kg, Soft)
chip away at the intruder's small body parts — both his ears get
crushed in a typical run, dropping his hearing capacity to 0.0 while
he still wins the fight. The crowbar (2.5 kg, Polished) crushes
larger parts but never severs (not Sharp).

Still open:

- Hit-roll & dodge — currently every blow lands.
- Armor: layered materials between weapon and body part should
  absorb damage based on weapon texture (Sharp vs. Blunt) and the
  armor's thermal/elastic properties (already on items).
- Ranged weapons: ballistics simulated (line of sight + travel
  time) vs. abstract.
- Morale, retreat, surrender.
- Skill-based modifiers (§10).
- Cascading damage to internal organs from severe external hits.

## 14. Pathfinding & navigation — partial

`find_path(world, start, goal, max_iter) -> Option<Vec<Pos>>` runs A*
over the voxel grid, 8-connected on a single Z-level. Diagonal
corner-cutting is forbidden: a diagonal step requires both flanking
cardinal cells to also be walkable, matching DF's "you can't squeeze
between two walls" rule. Step costs are 10 (cardinal) and 14
(diagonal); the heuristic is the corresponding octile distance.

Code: `crates/engine/src/pathfind.rs`

Used by the home invasion scenario today: every tick the intruder
re-plans to the nearest living family member and takes path[1]. When
the path's first step lands on the target's tile, the move becomes an
attack instead.

Still open:

- Z-level traversal via ramps / stairs (the search is currently
  single-Z).
- Treating other entities as dynamic obstacles, with replan triggers.
- Coarse chunk-level graph for long-distance plans on large maps.
- Path caching / partial replans instead of full A* per tick.
- Group movement (a squad of soldiers moving together).

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

## 16b. Hazards — partial

Environmental hazards are entities with `Position` + `Hazard`. The
`check_hazards` system runs after `execute_tasks`, reads the tick's
`EntityMoved` events, and applies a hazard whenever a creature
steps onto its tile.

Today's only `Hazard` variant is `Slippery { slip_chance,
prone_ticks, label }`: rolls against the seeded RNG, and on a hit
shoves a `Task::Wait(prone_ticks)` to the front of the victim's
queue and emits an `Event::Slipped`. The renderer was extended so
non-creature entities (hazards, furniture) with `Position + Kind`
appear on the ASCII map *only* if the scenario registers a glyph
for that kind — voxels show through otherwise.

Code: `crates/engine/src/hazards.rs`

This is the engine's first piece of the broader **runtime event
injection** story: external code (CLI, REPL, future LLM front-end)
gets `&mut World` between ticks and can spawn hazards, push events
into the log, mutate components — and existing systems pick it up
on the next tick without any special "injection" plumbing.

Future hazard kinds: `Burning`, `Poisonous`, `Electric`,
`Suffocating`. Each one adds a variant to `Hazard` and a match arm
to `apply_hazard`.

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

## 19. Determinism & RNG — partial

A seeded `Rng` resource (splitmix64) lives on the simulation world.
The seed comes from `RunOptions::rng_seed` (default 0xCAFE_BABE_DEAD_BEEF);
same seed produces the same run, including combat hit-part picks.

Code: `crates/engine/src/rng.rs`

Still open:

- Route worldgen, AI decisions, and weather through this RNG once
  those systems exist.
- Per-entity RNG streams so adding a new system doesn't shift every
  earlier roll.
- Replay: rebuild final state from `rng_seed` + scenario inputs alone.

## 20. Configuration & scripting — planned

Scenarios are Rust code today. That's fine for the engine team, but
we'll want a data-driven path for non-coders: TOML/JSON or a small
scripting language. Trade-off is iteration speed vs. expressiveness.
Defer until we know what real scenarios need.

## How systems compose — example: home invasion

The current home-invasion scenario exercises §1, §2, §4, §5, §9,
§12, §13, §14, §17, §18, §19. To make it richer we'd pull in:

- §6 (personality): one resident is a coward and tries to hide; one
  is brave and charges.
- §10 (skills): firearms skill on a homeowner with a pistol; a
  brawler resident throws better punches.
- §11 (needs): if the simulation ran long enough, residents would
  also need to eat and sleep — usually irrelevant to a 5-minute
  invasion.
- §13 (combat): add armor absorption from worn clothing's thermal /
  texture properties (already on items), plus dodge rolls based on
  current Mobility capacity.
- §15 (weather): a stormy night reduces visibility, biasing the
  intruder toward riskier paths.

That layering is the point: each new system adds depth across every
scenario without rewriting them.
