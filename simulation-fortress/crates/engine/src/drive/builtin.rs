//! Universal drives that ship with the engine.
//!
//! Designed to be reusable across scenarios. Each drive scores
//! against generic world state (perception, fear, position) — none
//! of them assume a specific scenario.

use bevy_ecs::prelude::{Entity, World};

use super::Drive;
use crate::components::{Faction, Health, Position};
use crate::items::{Inventory, ItemMaterial, ItemName};
use crate::needs::Fear;
use crate::physics::Locomotion;
use crate::sound::Perceived;
use crate::tasks::{Goal, Task, TaskQueue};
use crate::time::Clock;
use crate::world::{Pos, VoxelWorld};

// ─── helpers ──────────────────────────────────────────────────────

fn label(world: &World, e: Entity) -> String {
    world
        .get::<crate::components::Kind>(e)
        .map(|k| k.0.clone())
        .unwrap_or_else(|| format!("entity#{}", e.index()))
}

fn nearest_visible<F>(world: &World, actor: Entity, mut filter: F) -> Option<(Entity, Pos)>
where
    F: FnMut(Entity) -> bool,
{
    world.get::<Perceived>(actor).and_then(|p| {
        p.seen
            .iter()
            .filter(|s| s.entity != actor && filter(s.entity))
            .min_by_key(|s| s.distance)
            .map(|s| (s.entity, s.position))
    })
}

// ─── drives ───────────────────────────────────────────────────────

/// Run for cover when violence happens. Scores high if the actor
/// recently heard a violent sound or a neighbor was attacked, scaled
/// by their current `Fear`. Enacts a flight to `flee_to`.
pub struct FleeFromViolence {
    pub flee_to: Pos,
}
impl Drive for FleeFromViolence {
    fn name(&self) -> &str { "FleeFromViolence" }
    fn score(&self, world: &mut World, actor: Entity) -> f32 {
        // Fear-driven, not chaos-driven. Single loud sounds don't
        // make brave actors flee; sustained terror does. Fear is
        // built up over time by `fear_from_combat`. This means a
        // freshman in a food fight doesn't immediately bolt — they
        // stick around and chuck food until the chaos has ground
        // their nerves down.
        let fear = world.get::<Fear>(actor).map(|f| f.current).unwrap_or(0.0);
        if fear < 0.35 {
            return 0.0;
        }
        // Above the threshold, scale linearly. fear=0.5 → 0.55,
        // fear=0.7 → 0.85 (interrupt threshold), fear=1.0 → 1.0.
        ((fear - 0.20) * 1.25).clamp(0.0, 1.0)
    }
    fn enact(&self, world: &mut World, actor: Entity) {
        if let Some(mut q) = world.get_mut::<TaskQueue>(actor) {
            q.push(Task::MoveTo(self.flee_to));
        }
        if let Some(mut g) = world.get_mut::<Goal>(actor) {
            *g = Goal::Flee(self.flee_to);
        }
        world.entity_mut(actor).insert(Locomotion::Running);
    }
}

/// "I don't recognize this place — let me get out." Scores low but
/// nonzero when the actor has no other goal AND their current
/// position isn't a tile they'd identify as home/work. Enacts a
/// wander toward `exit_hint`.
pub struct ExitUnfamiliar {
    pub exit_hint: Pos,
}
impl Drive for ExitUnfamiliar {
    fn name(&self) -> &str { "ExitUnfamiliar" }
    fn score(&self, _world: &mut World, _actor: Entity) -> f32 {
        // Drives are ranked by score; this is a baseline "if nothing
        // else, head toward the door" so it only wins in a vacuum.
        0.15
    }
    fn enact(&self, world: &mut World, actor: Entity) {
        if let Some(mut q) = world.get_mut::<TaskQueue>(actor) {
            q.push(Task::MoveTo(self.exit_hint));
        }
        if let Some(mut g) = world.get_mut::<Goal>(actor) {
            *g = Goal::GoTo(self.exit_hint);
        }
        world.entity_mut(actor).insert(Locomotion::Walking);
    }
}

/// Walk back to a known home position when you're somewhere else
/// and have nothing pressing.
pub struct ReturnHome {
    pub home: Pos,
}
impl Drive for ReturnHome {
    fn name(&self) -> &str { "ReturnHome" }
    fn score(&self, world: &mut World, actor: Entity) -> f32 {
        let pos = match world.get::<Position>(actor) {
            Some(p) => p.0,
            None => return 0.0,
        };
        let d = pos.manhattan(self.home);
        if d <= 1 { 0.0 } else { (d as f32 / 30.0).min(0.30) }
    }
    fn enact(&self, world: &mut World, actor: Entity) {
        if let Some(mut q) = world.get_mut::<TaskQueue>(actor) {
            q.push(Task::MoveTo(self.home));
        }
        if let Some(mut g) = world.get_mut::<Goal>(actor) {
            *g = Goal::GoTo(self.home);
        }
    }
}

/// Idle baseline — wander or just wait. Returns a tiny non-zero
/// score so this is the fallback when nothing else is interesting.
pub struct Idle;
impl Drive for Idle {
    fn name(&self) -> &str { "Idle" }
    fn score(&self, _world: &mut World, _actor: Entity) -> f32 { 0.05 }
    fn enact(&self, world: &mut World, actor: Entity) {
        if let Some(mut q) = world.get_mut::<TaskQueue>(actor) {
            q.push(Task::Wait(2));
        }
    }
}

/// Win the food fight. Scores high if the actor:
///   - holds something throwable (food-flavored item in inventory),
///   - sees a rival (a creature with a faction in `rival_factions`),
///   - hears recent throwing/screams.
/// Enacts: throw the priciest food at the rival who'd most enjoy
/// being hit (highest existing `Messiness`).
///
/// Generic enough that any "kid in a chaotic room with a snack and
/// a rival" — wherever they are — will start chucking.
pub struct WinFoodFight {
    pub rival_factions: Vec<String>,
}
impl Drive for WinFoodFight {
    fn name(&self) -> &str { "WinFoodFight" }
    fn score(&self, world: &mut World, actor: Entity) -> f32 {
        let has_food = world
            .get::<Inventory>(actor)
            .map(|inv| inv.0.iter().any(|e| is_food_item(world, *e)))
            .unwrap_or(false);
        if !has_food { return 0.0; }

        let sees_rival = world.get::<Perceived>(actor).map(|p| {
            p.seen.iter().any(|s| {
                s.entity != actor
                    && world
                        .get::<Faction>(s.entity)
                        .map(|f| self.rival_factions.iter().any(|rf| rf == &f.0))
                        .unwrap_or(false)
            })
        }).unwrap_or(false);
        if !sees_rival { return 0.0; }

        let chaos = world
            .get::<Perceived>(actor)
            .and_then(|p| p.loudest_violent().map(|hs| hs.apparent_intensity))
            .unwrap_or(0.0);
        // KEY GATE: only fire when there's already chaos. Calm
        // students with food don't pre-emptively throw at rivals;
        // they need a triggering disturbance. Without this, any
        // crowd of students with snacks instantly riots.
        if chaos < 0.05 {
            return 0.0;
        }
        // Baseline above the interrupt threshold so this drive
        // overrides routine schedule waits (kid stops "sitting at
        // table" and starts chucking food).
        (0.75 + chaos * 0.20).min(0.95)
    }
    fn enact(&self, world: &mut World, actor: Entity) {
        // Pick a target — closest visible rival, with a bonus for
        // already-messy ones (pile-on).
        let actor_pos = world.get::<Position>(actor).map(|p| p.0).unwrap_or_default();
        let target_pos: Option<Pos> = world.get::<Perceived>(actor).and_then(|p| {
            let mut best: Option<(f32, Pos)> = None;
            for s in &p.seen {
                if s.entity == actor { continue; }
                let is_rival = world.get::<Faction>(s.entity)
                    .map(|f| self.rival_factions.iter().any(|rf| rf == &f.0))
                    .unwrap_or(false);
                if !is_rival { continue; }
                let mess = world.get::<Messiness>(s.entity).map(|m| m.0).unwrap_or(0.0);
                let score = mess * 4.0 - (s.distance as f32 * 0.2);
                if best.map(|(b, _)| score > b).unwrap_or(true) {
                    best = Some((score, s.position));
                }
            }
            best.map(|(_, p)| p)
        });

        let food: Option<Entity> = world.get::<Inventory>(actor).and_then(|inv| {
            inv.0.iter().copied().find(|e| is_food_item(world, *e))
        });

        if let (Some(food), Some(tpos)) = (food, target_pos) {
            if let Some(mut q) = world.get_mut::<TaskQueue>(actor) {
                q.push(Task::Throw(food, tpos));
            }
            if let Some(mut g) = world.get_mut::<Goal>(actor) {
                *g = Goal::GoTo(actor_pos);
            }
            world.entity_mut(actor).insert(Locomotion::Walking);
        }
    }
}

fn is_food_item(world: &World, item: Entity) -> bool {
    // Heuristic: name hints OR food-ish material.
    let name_says = world.get::<ItemName>(item).map(|n| {
        let s = n.0.to_lowercase();
        s.contains("potato") || s.contains("ketchup") || s.contains("oatmeal")
            || s.contains("plate") || s.contains("bottle") || s.contains("flour")
            || s.contains("food") || s.contains("apple") || s.contains("bread")
    }).unwrap_or(false);
    if name_says { return true; }
    if let Some(mat) = world.get::<ItemMaterial>(item) {
        if let Some(m) = world.resource::<VoxelWorld>().material(mat.0) {
            let n = &m.name;
            return matches!(n.as_str(),
                "mashed_potato" | "ketchup" | "oatmeal" | "flour" | "sugar" | "coffee");
        }
    }
    false
}

/// "An adult sees children fighting in their jurisdiction and tries
/// to break it up." Scores high if the actor sees creatures
/// throwing food / being messy in the configured radius. Enacts:
/// move toward the messiest visible kid.
pub struct BreakUpFight {
    pub range: i32,
    pub jurisdiction: Option<(Pos, Pos)>, // bounding box; None = anywhere
}
impl Drive for BreakUpFight {
    fn name(&self) -> &str { "BreakUpFight" }
    fn score(&self, world: &mut World, actor: Entity) -> f32 {
        let actor_pos = match world.get::<Position>(actor) {
            Some(p) => p.0,
            None => return 0.0,
        };
        if let Some((min, max)) = self.jurisdiction {
            if !(actor_pos.x >= min.x && actor_pos.x <= max.x
                && actor_pos.y >= min.y && actor_pos.y <= max.y) {
                return 0.0;
            }
        }
        // Look for nearby messy or throwing entities.
        let saw_chaos = world.get::<Perceived>(actor).map(|p| {
            p.seen.iter().any(|s| {
                if s.distance > self.range { return false; }
                world.get::<Messiness>(s.entity).map(|m| m.0 > 0.05).unwrap_or(false)
            })
        }).unwrap_or(false);
        if saw_chaos { 0.65 } else { 0.0 }
    }
    fn enact(&self, world: &mut World, actor: Entity) {
        let target = nearest_visible(world, actor, |e| {
            world.get::<Messiness>(e).map(|m| m.0 > 0.05).unwrap_or(false)
        });
        if let Some((_, tpos)) = target {
            if let Some(mut q) = world.get_mut::<TaskQueue>(actor) {
                q.push(Task::MoveTo(tpos));
            }
            if let Some(mut g) = world.get_mut::<Goal>(actor) {
                *g = Goal::GoTo(tpos);
            }
            world.entity_mut(actor).insert(Locomotion::Running);
            // Verbal authority: emit a Note so the narrator tags this.
            let now = world.resource::<Clock>().tick;
            let lbl = label(world, actor);
            world.resource_mut::<crate::log::EventLog>().push(now,
                crate::log::Event::Note(format!(
                    "{lbl} barks: \"BREAK IT UP! NOW!\""
                )));
        }
    }
}

/// "I see a mess in my jurisdiction — I'll mop it up." Scores high
/// if the actor sees `Coating` entities (puddles, food splats) in
/// the configured range. Enacts: walk to it. (Actual cleanup is a
/// scenario concern; for now standing on the tile is enough — the
/// existing `decay_coatings` evaporates the mess.)
pub struct CleanSpills {
    pub range: i32,
}
impl Drive for CleanSpills {
    fn name(&self) -> &str { "CleanSpills" }
    fn score(&self, world: &mut World, actor: Entity) -> f32 {
        let actor_pos = match world.get::<Position>(actor) {
            Some(p) => p.0,
            None => return 0.0,
        };
        let saw_spill = {
            let mut q = world.query::<(&Position, &crate::physics::Coating)>();
            q.iter(world)
                .any(|(p, _)| p.0.chebyshev(actor_pos) <= self.range)
        };
        if saw_spill { 0.40 } else { 0.0 }
    }
    fn enact(&self, world: &mut World, actor: Entity) {
        let actor_pos = match world.get::<Position>(actor) {
            Some(p) => p.0,
            None => return,
        };
        let target: Option<Pos> = {
            let mut q = world.query::<(&Position, &crate::physics::Coating)>();
            q.iter(world)
                .map(|(p, _)| p.0)
                .min_by_key(|p| p.manhattan(actor_pos))
        };
        if let Some(t) = target {
            if let Some(mut q) = world.get_mut::<TaskQueue>(actor) {
                q.push(Task::MoveTo(t));
            }
            if let Some(mut g) = world.get_mut::<Goal>(actor) {
                *g = Goal::Tend(actor); // placeholder; "cleaning" task not modeled
            }
            world.entity_mut(actor).insert(Locomotion::Walking);
        }
    }
}

// ─── shared "win condition" component ─────────────────────────────

/// Cumulative messiness — how splattered the entity has gotten in
/// food fights, blood spatter, mud, etc. Engine carries the
/// component; scenarios decide what counts as a "win" (e.g.
/// cafeteria: lowest at end).
#[derive(bevy_ecs::prelude::Component, Default, Debug, Clone)]
pub struct Messiness(pub f32);

impl Messiness {
    pub fn add(&mut self, amount: f32) {
        self.0 = (self.0 + amount).clamp(0.0, 5.0);
    }
}
