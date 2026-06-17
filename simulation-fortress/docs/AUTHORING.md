# Authoring Scenarios

A practical guide for building new scenarios on top of the engine —
written for a human author and equally aimed at an AI co-author. The
goal is to make the path from scenario idea to running simulation
explicit, repeatable, and short.

## The 7-step loop

Every scenario follows roughly the same authoring sequence:

1. **Survey** — read `docs/SYSTEMS.md` and `crates/engine/src/lib.rs`'s
   exports. Open the closest existing scenario in `crates/scenarios`
   to see how it's wired together. The point of this step is to know
   the toolbox before reaching for it.

2. **Map concepts to primitives.** Translate the scenario's domain
   nouns into engine concepts. A *kitchen* is a voxel layout with
   appliances as entities. A *farmer* is a humanoid creature with a
   role marker component. A *crop* is an entity with growth-stage
   components. An *order* is a queue resource. Write this mapping
   out before touching code.

3. **Identify gaps.** What does the scenario need that the engine
   doesn't have yet? For each gap, choose: build it as engine-level
   (`crates/engine`) if it would be useful in a second scenario, or
   keep it scenario-local for now. When in doubt, scenario-local;
   promote on second sighting.

4. **Write the scenario.** Implement the `Scenario` trait:
   - `setup(&mut World)` populates the world (voxel terrain, spawned
     entities, scenario-local resources).
   - `build_schedule()` returns a `Schedule` whose systems run once
     per tick.
   - `is_complete(&mut World)` reports completion.
   Keep custom components and systems in the scenario module.

5. **Smoke test.** Run with `--fast --ticks 200`. Read the narration.
   Verify the *kinds* of events you expected actually fire. If the
   intruder never reaches the door, your pathfinding setup is wrong.
   If no orders ever complete, your task queue isn't draining.

6. **Tune.** Fix the numbers — HP, damage, growth rates, arrival
   timers. Use a fixed `--seed` while tuning so behavior is
   reproducible.

7. **Promote.** When you find yourself reinventing something from
   another scenario (a generic queue, a "find nearest" helper, a
   common component), lift it into the engine and update SYSTEMS.md.

## Engine vs scenario: where things go

| Lives in `crates/engine` | Lives in scenario module |
|---|---|
| Voxel/world primitives | Map layout (specific cabin, beach, kitchen) |
| Generic components (Position, Health, Faction, Kind) | Marker components (Intruder, Family, Customer) |
| Reusable systems (combat, pathfind, mood) | Per-role behavior systems |
| Universal events (EntityMoved, BodyPartWounded) | Scenario-specific events (OrderFilled, CropHarvested) |
| Bundles (`spawn_humanoid_body`) | Scenario-specific spawn helpers |
| Tasks that compose 80% of needs (MoveTo, UseEntity, Attack) | Tasks specific to one domain (Plow, ServeOrder) |

The simplest test: if removing a piece of code wouldn't hurt any other
scenario, it belongs in the scenario.

## Layout of a scenario module

```
crates/scenarios/src/
├── main.rs               # CLI entry point, picks a scenario by name
├── home_invasion.rs      # one Scenario impl + its private systems
└── farming.rs            # another Scenario impl
```

Each scenario file typically contains, in order:

1. Use statements pulling from `fortress_engine` and the prelude.
2. `const` values that define the world (sizes, positions, names).
3. Scenario-local marker components (`Intruder`, `Family`, `Customer`).
4. The `Scenario` trait impl: `name`, `setup`, `build_schedule`,
   `is_complete`.
5. Per-tick systems referenced by `build_schedule`.
6. Spawn helpers for scenario-specific entities (e.g.,
   `spawn_crowbar`, `spawn_wool_shirt`).
7. Pure helper functions at the bottom.

## The discovery problem (especially for AI authors)

The biggest cost in step 1 is *knowing what's already there*. Two
artifacts are intended to make this cheap:

- **`docs/SYSTEMS.md`** — prose overview of every cross-cutting
  system, with status (done / partial / planned). Read this first.
- **`crates/engine/src/lib.rs`** — every public symbol re-exported
  in one place. The `prelude` module re-exports the most common bevy
  ECS items as well. Glance at this whenever SYSTEMS.md isn't
  specific enough.

A future addition will be a build-time JSON manifest enumerating
every `Component`, `Resource`, `Event` variant, and helper function
with its docstring — so an AI can query the toolbox without parsing
Rust. Until that exists, the prelude is the next best thing.

## Anti-patterns to avoid

- **Reinventing pathfinding, RNG, or events.** They exist; use them.
- **Stuffing all per-creature data into one `ExtraData` HashMap.**
  Each new fact wants its own component. `ExtraData` is for one-off
  string keys that don't repeat across creatures.
- **Single mega-system on `&mut World`.** Sometimes necessary
  (combat resolution touches many things), but most logic should
  use typed `Query` / `Res` / `ResMut` so bevy can validate and, in
  the future, parallelize.
- **Scenarios that mutate via raw `world.set_voxel` without
  logging.** Use `actions::set_voxel_logged` /
  `actions::fill_region_logged` so the narrator gets events.
- **Hardcoded numbers without an obvious knob.** Pull tuning
  constants to the top of the file as `const` so the next author
  (you, in two weeks) can find them.

## Worked example: how to think about a restaurant sim

To make the loop concrete, here's the way the steps play out for a
restaurant scenario, even though we haven't built it:

1. **Survey.** Items + inventory exist. Pathfinding exists. Combat
   exists but won't be central. SYSTEMS.md flags task system as
   planned — that's our biggest gap.

2. **Map.** Kitchen + dining area = voxel layout with `floor_glyph`
   tweaks. Staff = humanoids with `Host` / `Server` / `LineCook` /
   `Dishwasher` marker components. Customers = humanoids spawned by
   a procedural-arrival system. Tables, stoves, fridges = furniture
   entities (custom marker components, no Health). Ingredients =
   items with new `Ingredient` and `PerishesAt` components. Dishes
   = items built from a recipe.

3. **Gaps.** Recipes (engine-worthy: also for farming, zombie
   crafting). Money (engine-worthy if 2+ scenarios will use it).
   Order/queue (probably scenario-local — restaurants have a
   specific shape). Customer patience timer (scenario-local).

4. **Write it.** Implement `Scenario` for `Restaurant`. `setup`
   lays out kitchen + dining + spawns staff. `build_schedule`
   registers `customer_arrival_system`, `host_seat_system`,
   `server_take_order_system`, `cook_prepare_system`,
   `customer_eat_system`, `customer_pay_and_leave_system`.

5. **Test.** `--fast --ticks 500 --seed 1`. Verify customers arrive,
   get seated, are served, pay, leave. Look for stuck customers
   (patience expired, no server picked up the order).

6. **Tune.** Customer arrival rate. Cook speed. Patience threshold.

7. **Promote.** If recipes look general (yes), lift to engine. If
   tipping looks restaurant-specific, leave it.

## Cross-references

- `docs/SYSTEMS.md` — system-by-system status & open questions.
- `crates/engine/src/lib.rs` — toolbox listing.
- `crates/scenarios/src/home_invasion.rs` — current canonical
  example.
