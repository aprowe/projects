# Designing for emergence

For each model scenario, a short thought experiment: what behavior do
we want to *emerge* from the simulation rather than be scripted, and
which generic systems would produce it? The engine's job is to ship
the generic systems; the scenario's job is to set up the world and
let the systems do their work.

The discipline is to push every "I want X to happen" through a
generic system rather than a scenario-specific check. If three
scenarios end up wanting "the same thing" in different costumes,
that's the engine's signal.

---

## Home invasion

**Desired emergent behavior**

- A bigger multi-room home where pursuit and hiding matter.
- Family members react to combat noise from another room: scream,
  flee, hide, grab a weapon.
- Intruder forced to commit to a target, occasionally lose them in
  the layout, search rooms.
- Doors get smashed, furniture knocked over, broken glass becomes a
  hazard.
- A resident grabs a kitchen knife from the counter and stabs back.

**Generic systems that produce it**

- **Sound emission + propagation.** Combat blows, breaking voxels,
  shouts emit `Event::SoundEmitted { position, intensity, kind }`
  with a radius. Anything is allowed to emit; anything with
  `Hearing` is allowed to perceive.
- **Perception** (sight cones + hearing radii). A `Perception` system
  populates a per-creature `Perceived { seen, heard }` set each tick
  by walking voxel line-of-sight + sound events within range.
- **Goal::Flee(threat)** + **Goal::Hide(spot)**. The planner sees
  perceived danger; if its behavior is "civilian" it switches goal.
  Flee paths away from the threat; Hide picks a low-traffic tile,
  switches `Locomotion::Sneaking` (which the footing system already
  honors).
- **Voxel HP / breakable voxels.** Doors and walls take damage from
  attacks. When HP hits 0 the voxel becomes the next-state material
  (door splinters into wood floor + scattered planks).
- **Reachable items awareness.** A behavior system that, on threat
  detection, queries nearby items via `Position` for anything with a
  `Wearable(MainHand)` component, queues `Acquire(item)` →
  `MoveTo(item)` → `PickUp(item)` → `Equip(item)` → `Attack(threat)`.
- **Furniture as solid entities.** A `Solid` marker on a positioned
  entity makes pathfinding treat that tile as blocked. No new system —
  pathfind already reads voxel walls; extend it to also consult an
  index of solid entities.

**Interactions that fall out**

- Intruder smashes the door (voxel HP) → emits a sound at the
  doorway → resident perception system flags the noise → planner
  switches resident from Idle to Flee → resident sneaks to a closet →
  Hide goal stops them → they have low locomotion + line of sight
  blocked → intruder may lose them.
- During combat, resident screams (Sound emitted) → nearby resident
  in another room hears → switches to Flee/Hide too. The first
  scream sets the tone for the rest of the house.
- A resident's flee path runs through the kitchen → behavior sees
  knife on the counter → queues Acquire → resident now armed →
  better fight at next combat.
- Intruder takes damage that breaks a leg → Mobility drops →
  footing checks more often fail → trips chasing the resident.

**What's deliberately not engine-specific**

- The kitchen layout, the door positions, the family members'
  personalities — these are scenario data. The engine just gives
  perception + sound + Goal::Flee/Hide; the scenario decides who has
  Hearing(range) and what range, where the closet is, etc.

---

## Farming

**Desired emergent**

- Days have weather; rain accelerates growth, drought stalls it.
- Farmers need sleep at night; their daily rhythm of work / eat /
  sleep is a property of needs + schedule, not a script.
- Animals: a stray cow eats crops; a guard dog barks at strangers.
- A bad harvest leaves the larder thin; farmers go hungry sooner.
- Pests can ruin a row of crops if not noticed.

**Generic systems**

- **Day/night clock + schedule.** A `TimeOfDay { hour }` resource;
  needs decay rates pulse with it (energy drains slower when asleep).
  Goal selection consults time-of-day.
- **Weather** as a resource (`Weather { kind, intensity }`). Crop
  growth rate is a function of weather. Drought is just `Weather { kind: Drought, intensity }` plus the engine reading it.
- **Sleep need / Goal::Rest(spot).** Energy below threshold ⇒ planner
  swaps to Goal::Rest; tasks are MoveTo(bed), UseEntity(bed) which
  has a `Restorative { energy: 1.0 }` component (mirror of Edible).
- **Animal AI** = the same Goal/TaskQueue substrate, with a different
  planner. Cow planner: Idle wandering, but if Crop within 3 tiles
  → Goal::Eat(crop). Dog planner: bark (sound emit) when stranger
  within sight cone.
- **Pest entities** with their own tick. A `Pest` near a Crop reduces
  GrowthStage instead of advancing it. Farmer with `Perception`
  sees the pest, switches to Goal::Kill(pest).

**Interactions**

- Sun resource pulses → time = midnight → farmers' Rest goal kicks
  in → they go home, sleep, energy refills.
- Cow wanders to (3,4) → cow planner sees crop → eats one stage of
  it. Farmer perception spots the cow, switches goal to drive it
  off → cow flees. Crops saved at a partial loss.
- Drought rolls in → all crop GrowthStage advancement is multiplied
  by 0.3 → harvest day arrives later than expected → larder runs
  thin → next morning farmers' hunger spikes earlier than usual.

**Engine work needed**

- Day/night, weather as resources.
- Restorative component (twin of Edible, drops Energy instead of
  Hunger).
- Sleep goal + Goal::Rest.
- Animals don't need new components; they reuse the existing ECS
  primitives with different planners.

---

## Office drama

**Desired emergent**

- Cliques form: workers with similar personalities cluster.
- A bad mood spreads through gossip.
- A boss enters the room → workers stop slacking and look busy.
- Lunch hour empties the floor in 30 seconds.
- Promotion announcement → some moods spike, some plummet.

**Generic systems**

- **Personality** (Big-Five-ish trait vector). Drives every social
  decision: who you talk to, what you say, how you react.
- **Relationships** as a sparse map `(Entity, Entity) → Affinity`.
  Updated by every social interaction.
- **Conversation task / event.** `Task::Talk(target, topic)` produces
  `Event::Conversation { participants, topic, outcome }`. Outcome
  shifts both participants' relationships and moods.
- **Information / rumors** as data passed through conversations. A
  `KnownRumor` set per worker; Talk transmits it with some
  probability.
- **Status hierarchy.** `Status(u8)` component; a high-status
  entity in the room raises the local "professionalism" → workers'
  planners weight Work goal higher than Social.
- **Schedules.** A `Schedule` component or per-faction resource
  determines what goal range is allowed at what time-of-day.

**Interactions**

- Two workers with similar personality vectors cross paths → planner
  prefers Talk(neighbor) over Work → talk produces small relationship
  bump → next time at lunch they sit together → over many ticks a
  clique emerges. Nobody scripted "alice and bob are friends."
- Bad rumor (`Event::RumorStarted` from a planner) propagates by
  Talk events. Every recipient's mood drops slightly. After 50 ticks,
  the floor is grumpy.
- Boss enters → workers within sight of high-status entity → planner
  weight on Work goal jumps → desk usage spikes, social tasks pause.
- Lunch hour ticks past noon → schedule allows Goal::Eat for everyone
  → mass migration. Just the existing Hunger/Edible system, gated by
  a schedule resource.

**Engine work needed**

- Personality + Relationships components.
- Talk task + Conversation event.
- TimeOfDay resource + Schedule mechanism.
- Rumor / KnownInformation set.

---

## Restaurant sim

**Desired emergent**

- Customers wait, get impatient, complain or leave.
- Cooks juggle orders; sometimes burn one when overloaded.
- Hot food cools while waiting on the pass.
- Servers pick up the closest open task.
- A great review night vs a disaster night, depending on staffing.

**Generic systems**

- **Job queue** as a resource: orders come in, available staff claim
  them by proximity + skill. Same primitive as the farming
  desk/coffee claim system, generalized.
- **Patience** as a need (timer that drops mood when not satisfied).
- **Recipes** as data: `Ingredient` components combine into
  `Dish` entities. Cooking is `UseEntity(stove)` over multiple ticks.
- **Temperature decay** on items: every Item with `Temperature` drifts
  toward ambient at some rate. Hot dishes get less appealing.
- **Money** as a resource on each entity; transactions are
  `Event::PaymentMade { from, to, amount }`.
- **Customer arrival generator**: an engine system that spawns from
  a `crowd_template` (library role) on a schedule.

**Interactions**

- Customer arrives → host (highest status free server) seats them →
  patience timer starts → server takes order → cook claims order
  from job queue → uses stove → dish entity created with high
  Temperature → server delivers (MoveTo + Give to customer).
- Cook overloaded → claims drop, queue grows → patience timers tick
  down faster than dishes go out → mood drops → tip lower.
- Hot dish on the pass cools while waiting → customer eats but
  enjoys less → relationship with restaurant (entity!) goes down.

**Engine work needed**

- Job queue resource (generalize from farming).
- Patience component (twin of Hunger but tied to specific events).
- Recipe / Ingredient / Crafting system.
- Temperature decay.
- Money + transaction events.
- Procedural arrival system (could be general — a `Spawner` resource
  ticked each tick, which farming and zombie also use).

---

## Stealth thief in a crowded Indian market

**Desired emergent**

- A thief threads through a dense crowd, undetected when sneaking.
- Pickpockets succeed when target's awareness is low.
- A failed pickpocket triggers a shouted alarm; guards converge.
- Crowd density gives concealment: a guard can't see through people.
- Running in the lanes is fast but loud; sneaking is slow but quiet.

**Generic systems**

- **Stealth** as visibility: a `Perception` system that combines
  line-of-sight (voxels + crowd as soft cover) + actor's
  Locomotion-driven sound footprint.
- **Suspicion** per-creature: an alertness meter that rises when
  the creature perceives anomalous events (a shout, a thief running),
  decays slowly. At threshold, behavior switches from Patrol to
  Investigate; at higher threshold, to Pursue.
- **Pickpocket task** = Steal(target_inventory_index). Success
  rolls against `target.Perception.awareness` and `thief.skill`.
- **Crowd entities** at low fidelity — just `Position + Kind +
  Wandering` markers. Each crowd member blocks line of sight when
  on a tile between observer and observed.
- **Goal::Patrol(area)** with patrol points; guards cycle through
  them.

**Interactions**

- Thief sneaking + crowd between thief and guard → perception fails
  to add thief to guard's `Perceived.seen` → undetected.
- Failed pickpocket → target shouts (sound emit) → guards' Hearing
  picks it up → suspicion meters tick up → above threshold guards
  switch from Patrol to Investigate, pathfind to source.
- Thief now seen → suspicion → Pursue → multiple guards converge.
- Thief running → loud footstep sounds → easier to track even when
  visually hidden.

**Engine work needed**

- Sight + Hearing components and a Perception system.
- Suspicion / Alertness component.
- Steal task (extends task taxonomy).
- Crowd low-fidelity entities (mostly just markers).
- Patrol goal with waypoints.

---

## Cafeteria food fight (seniors vs freshmen)

**Desired emergent**

- A first thrown bowl ignites a chain reaction.
- Some students dodge, some get hit and retaliate.
- Splatter creates slippery floors that cause more chaos.
- A lunch monitor wades in; high status freezes nearby students.
- Faction morale flips when a "leader" goes down.

**Generic systems**

- **Throw task** = `Throw(target_pos, item)`. Item becomes a
  projectile (entity with Position + Velocity + Trajectory).
- **Projectile / ballistics** system that ticks position; on impact
  emits `Event::Hit { projectile, target_tile }`.
- **On-hit splatter**: when a projectile with a low-friction
  material hits a tile, it spawns a `Coating` of that material at
  the impact point. (Already-present footing system handles slips.)
- **Faction-driven targeting**: planner picks a target from the
  opposing faction with the lowest fear / most prominence.
- **Authority status**: any entity with high `Authority` in line of
  sight reduces nearby creatures' "throw weight" in their planner
  — they'd rather not be caught.
- **Morale waves**: on faction member down, all members of the
  faction get a fear bump (already exists via
  `fear_from_combat`); morale-based goal modifiers reduce throwing
  rate.

**Interactions**

- Senior throws plate → it flies → freshman dodges (mobility check)
  → plate hits wall, becomes Coating(mashed_potato) at the wall
  → next freshman who walks past slips. Everything reuses existing
  systems (projectile + coating + footing).
- Freshman gets hit → fear spike → freshman throws back ←
  fear-driven combat — same primitive as home_invasion.
- Lunch monitor enters with Authority(5) → all student planners
  weight Throw goal lower → fight winds down.
- Senior leader takes a hit → other seniors' fear spikes → faction
  morale drops → throwing rate drops → freshmen press the
  advantage.

**Engine work needed**

- Throw task + projectile/trajectory.
- On-hit Coating spawn (a single rule that could live in physics
  or interactions: "when projectile of material M lands, spawn
  Coating(M)").
- Authority component + planner gating.

---

## D-Day landing

**Desired emergent**

- Squads advance under enemy fire, not in formation but in
  cover-to-cover hops.
- Heavy machine gun fire pins down a section; the rest move around.
- A mortar drops, three soldiers go down at once, the rest's morale
  cracks.
- Tank rolls forward, soldiers follow in its shadow.
- Soldiers near a dying officer get a morale shock.

**Generic systems**

- **Ranged combat** + **line of sight** through voxels.
- **Suppression** as a status effect: heavy fire near you increases
  fear (existing!) past a threshold → soldier crouches, refuses to
  fire back, mobility halved.
- **Cover-aware pathfinding**: a path-cost function that prefers
  tiles with line-of-sight to fewer enemies.
- **Squad / order propagation**: an `Officer` entity with a
  `SquadOrder { goal, targets }` that lower-ranked soldiers' planners
  read and adopt.
- **Vehicles as containers** — entities that carry other entities
  (already a hierarchy; bevy_ecs supports parent/child).
- **Morale** as a derived value: from squad casualties, current
  fear, recent successful kills.

**Interactions**

- Soldier sees enemy → fires → ranged-attack rolls against target's
  cover bonus + range falloff.
- Repeated near-misses → fear spikes (sound emission scaled with
  near-miss radius?) → suppressed → can't aim → squad leader gives
  flank order → another soldier breaks cover.
- Tank's `Solid + Position` blocks line of sight from enemy to
  soldiers behind it → those soldiers' cover bonus is high → safe
  movement.
- Officer dies → squad members all get fear bump (mass
  fear_from_combat triggered by a Note event tied to leader death)
  → morale crashes → squad scatters.

**Engine work needed**

- Ranged combat (project + line of sight + accuracy falloff).
- Cover-cost in pathfinding.
- SquadOrder / hierarchy.
- Vehicle containers.
- Morale as a derived component (could be derive_morale, twin of
  derive_mood).

---

## Zombie invasion

**Desired emergent**

- A barricade choke point thins the horde.
- A gunshot inside a "safe" room draws every nearby zombie.
- Survivors ration food; one starves, the rest get desperate.
- A bitten survivor turns mid-combat, fights their friends.
- Zombies clump around sounds.

**Generic systems**

- **Sound + flocking**: zombies' planner is just "move toward the
  loudest recently-heard sound, else nearest visible human."
- **Status effect: Infected** with a timer. On `Event::EntityKilled`
  for an Infected creature, spawn a `zombie` from the library at
  the same position.
- **Resource depletion**: ammo as an `Inventory` count. Out of ammo
  = ranged attack fails.
- **Voxel building by actors**: a `Build(Pos, Voxel)` task — already
  scaffolded. A survivor with planks in inventory + the right tile
  empty queues build tasks to wall it off.
- **Crowd flocking** = same as market: low-fidelity entities,
  shared planner that reads sound events.

**Interactions**

- Survivor fires → loud sound at survivor's position → all zombies
  in radius update their target → flock toward them.
- Choke point → zombies pile up at a single tile → survivor melee
  attacks the front of the queue → zombie corpse blocks the tile
  → reinforcement.
- Bitten survivor (Infected status) keeps fighting alongside friends
  → status timer expires mid-combat → on death (any cause) the
  conversion fires → former ally now attacks his own faction.
- Food ration runs low → Hunger spikes across all survivors → some
  go raid (Goal::Acquire(food)) outside the safe room → exposure
  → losses.

**Engine work needed**

- Sound system (shared with home_invasion + thief market).
- Status effects with timers (the broader pattern; today only Fear /
  Hunger have it).
- On-death conversion hook (engine emits an event for every
  EntityKilled; an interaction in the interactions library says
  "Infected + EntityKilled → spawn zombie at position").
- Build task (scaffolded, needs implementation).

---

## Cross-cutting takeaways

A handful of generic systems would feed *most* of the scenarios above:

1. **Sound emission + propagation** — home invasion, market, zombie,
   D-Day, restaurant (clattering plates), cafeteria (the noise is the
   point).
2. **Perception (sight + hearing)** — every scenario with adversarial
   AI.
3. **Goal::Flee / Goal::Hide / Goal::Patrol** — combat-aware
   civilians and patrolling guards everywhere.
4. **Status effects with timers** — fear, infection, suppression,
   drunk, on-fire — different costumes, same primitive.
5. **Projectile / on-hit interaction** — cafeteria food, D-Day
   bullets, zombie thrown axes. The "splatter creates Coating" rule
   is one entry in the interactions library.
6. **TimeOfDay + schedule resources** — farming, office,
   restaurant.
7. **Job/order queue** — restaurant, farming, office task
   coordination, military orders.
8. **Authority / status** — office boss, lunch monitor, military
   officer.

Three of those (sound, perception, status effects with timers) plus
the **interactions library** would unlock four of seven scenarios on
their own. That's the next layer to build.
