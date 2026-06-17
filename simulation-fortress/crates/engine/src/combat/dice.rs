//! Dice rolling against the seeded `Rng` resource. All rolls are
//! deterministic given a fixed seed — runs are reproducible.
//!
//! Higher-level helpers also live here for ability + function
//! checks. A "manipulation check" — opening a doorknob, picking a
//! lock — uses base DEX modified by `Function::Grasp` capacity so a
//! creature with crushed hands struggles with fine motor work
//! regardless of their nominal DEX.

use bevy_ecs::prelude::{Entity, World};

use crate::anatomy::{function_capacity, Function};
use crate::rng::Rng;
use crate::stats::Stats;

#[derive(Copy, Clone, Debug)]
pub struct RollResult {
    /// The natural die roll, 1..=20 for a d20.
    pub natural: i32,
    /// Modifier added to the natural roll.
    pub modifier: i32,
    /// Total = natural + modifier.
    pub total: i32,
}

impl RollResult {
    /// Natural 20 — automatic critical hit in most D&D variants.
    pub fn is_critical_hit(&self) -> bool {
        self.natural == 20
    }
    /// Natural 1 — automatic miss.
    pub fn is_critical_miss(&self) -> bool {
        self.natural == 1
    }
}

/// Roll one d20 plus a modifier. Returns the natural roll, the
/// modifier, and the total separately so narration can show all of
/// them ("rolls 14 + 3 = 17").
pub fn roll_d20(rng: &mut Rng, modifier: i32) -> RollResult {
    let natural = (rng.range(20) + 1) as i32;
    RollResult {
        natural,
        modifier,
        total: natural + modifier,
    }
}

/// Roll `count` dice with `sides` faces each. Each die is uniform
/// over `1..=sides`.
pub fn roll_dice(rng: &mut Rng, count: u8, sides: u8) -> i32 {
    if count == 0 || sides == 0 {
        return 0;
    }
    (0..count).map(|_| (rng.range(sides as u32) + 1) as i32).sum()
}

/// Make an ability check: roll d20 + modifier vs DC. Returns `true`
/// if the roll meets or exceeds the DC.
pub fn check(rng: &mut Rng, modifier: i32, dc: i32) -> (RollResult, bool) {
    let r = roll_d20(rng, modifier);
    let success = r.total >= dc;
    (r, success)
}

/// Outcome of a function-gated ability check.
#[derive(Clone, Debug)]
pub enum CheckOutcome {
    /// Roll met or beat the DC.
    Success(RollResult),
    /// Roll failed.
    Failure(RollResult),
    /// The actor lacks any working capacity for the required
    /// function — e.g., trying to manipulate a door with no intact
    /// hands. Auto-fail without rolling.
    Impossible,
}

impl CheckOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, CheckOutcome::Success(_))
    }
    pub fn roll(&self) -> Option<&RollResult> {
        match self {
            CheckOutcome::Success(r) | CheckOutcome::Failure(r) => Some(r),
            CheckOutcome::Impossible => None,
        }
    }
}

/// Compute the per-function penalty for a fine-motor / sensory /
/// physical check. Each "missing" intact unit of the function
/// (relative to a baseline of 2.0 for paired organs) costs -2 on
/// the d20 roll. A function capacity of 0 returns `None`, meaning
/// the check is impossible.
fn function_penalty(world: &mut World, actor: Entity, function: Function) -> Option<i32> {
    let cap = function_capacity(world, actor, function);
    if cap <= 0.0 {
        return None;
    }
    let missing = (2.0 - cap.min(2.0)).max(0.0);
    let penalty = (missing * 2.0).round() as i32;
    Some(-penalty)
}

/// Manipulation check: turning a doorknob, picking a lock, threading
/// a needle. Uses base DEX modifier minus a penalty for missing or
/// crushed hands. With no intact hands at all (Grasp = 0), returns
/// `Impossible` without rolling.
pub fn manipulation_check(world: &mut World, actor: Entity, dc: i32) -> CheckOutcome {
    let dex = world.get::<Stats>(actor).map(|s| s.dex_mod()).unwrap_or(0);
    let penalty = match function_penalty(world, actor, Function::Grasp) {
        Some(p) => p,
        None => return CheckOutcome::Impossible,
    };
    let modifier = dex + penalty;
    let mut rng = world.resource_mut::<Rng>();
    let roll = roll_d20(&mut rng, modifier);
    if roll.total >= dc {
        CheckOutcome::Success(roll)
    } else {
        CheckOutcome::Failure(roll)
    }
}

/// Strength check: forcing a door, lifting a heavy crate. Uses STR
/// mod minus a penalty for damaged limbs (Mobility provides
/// leverage). Same impossible-on-zero rule.
pub fn strength_check(world: &mut World, actor: Entity, dc: i32) -> CheckOutcome {
    let str_mod = world.get::<Stats>(actor).map(|s| s.str_mod()).unwrap_or(0);
    // Heavy lifts use both arms and legs for bracing; reduce by
    // missing Mobility (legs) but not below the no-arms case.
    let penalty = match function_penalty(world, actor, Function::Mobility) {
        Some(p) => p / 2,
        None => return CheckOutcome::Impossible,
    };
    let modifier = str_mod + penalty;
    let mut rng = world.resource_mut::<Rng>();
    let roll = roll_d20(&mut rng, modifier);
    if roll.total >= dc {
        CheckOutcome::Success(roll)
    } else {
        CheckOutcome::Failure(roll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn d20_is_in_range() {
        let mut rng = Rng::from_seed(42);
        for _ in 0..100 {
            let r = roll_d20(&mut rng, 0);
            assert!(r.natural >= 1 && r.natural <= 20);
            assert_eq!(r.total, r.natural);
        }
    }

    #[test]
    fn d20_modifier_applied() {
        let mut rng = Rng::from_seed(42);
        let r = roll_d20(&mut rng, 5);
        assert_eq!(r.total, r.natural + 5);
    }

    #[test]
    fn dice_in_range() {
        let mut rng = Rng::from_seed(7);
        for _ in 0..200 {
            let v = roll_dice(&mut rng, 2, 6);
            assert!((2..=12).contains(&v));
        }
    }

    #[test]
    fn deterministic_seed() {
        let seed = 0xDEADBEEF;
        let r1: Vec<_> = (0..10)
            .map(|_| {
                let mut rng = Rng::from_seed(seed);
                roll_d20(&mut rng, 0).natural
            })
            .collect();
        // All start from the same seed, so all are equal.
        assert!(r1.iter().all(|n| *n == r1[0]));
    }
}
