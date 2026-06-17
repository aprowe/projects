//! Combat resolution, D&D-style.
//!
//! `resolve_attack(attacker, target)` rolls 1d20 + STR mod against
//! the target's Armor Class. On a hit it rolls the weapon's damage
//! dice (or 1d4 fist), picks a body part by HitWeight, and applies
//! the damage to both the part's local HP and the creature's
//! aggregate Health. Natural 20 is a critical hit (max dice). Miss
//! emits `AttackMissed`; hit emits `BodyPartWounded` (and
//! `BodyPartDestroyed` if the part drops to 0); crit emits
//! `CriticalHit`. Destroying Heart, Head, or Neck instantly drops
//! the target's aggregate health to zero.
//!
//! All randomness routes through the seeded `Rng` resource — runs
//! are reproducible.

pub mod dice;

use bevy_ecs::prelude::{Entity, World};

use crate::anatomy::{
    anatomy_alive, BodyPartKind, HitWeight, PartHealth, PartOf, PartStatus,
};
use crate::components::Health;
use crate::dice::{roll_d20, roll_dice, RollResult};
use crate::items::{ArmorBonus, BodySlot, DamageDice, ItemName, Texture, Wearing};
use crate::log::{Event, EventLog};
use crate::rng::Rng;
use crate::stats::Stats;
use crate::time::Clock;

/// Summary of a single resolved blow.
#[derive(Clone, Debug)]
pub struct AttackResult {
    pub attack_roll: RollResult,
    pub target_ac: i32,
    pub hit: bool,
    pub critical: bool,
    pub damage: i32,
    pub part: Option<BodyPartKind>,
    pub status: Option<PartStatus>,
    pub destroyed: bool,
    pub killed: bool,
}

#[derive(Clone, Debug)]
struct WeaponInfo {
    name: String,
    dice: DamageDice,
    sharp: bool,
}

pub fn resolve_attack(
    world: &mut World,
    attacker: Entity,
    target: Entity,
) -> AttackResult {
    let weapon = describe_weapon(world, attacker);
    let attack_mod = world
        .get::<Stats>(attacker)
        .map(|s| s.str_mod())
        .unwrap_or(0);
    let target_ac = armor_class(world, target);

    // Attack roll
    let attack_roll = {
        let mut rng = world.resource_mut::<Rng>();
        roll_d20(&mut rng, attack_mod)
    };
    let critical = attack_roll.is_critical_hit();
    let critical_miss = attack_roll.is_critical_miss();
    let hit = !critical_miss && (critical || attack_roll.total >= target_ac);

    if !hit {
        push_event(
            world,
            Event::AttackMissed {
                attacker,
                target,
                attack_roll: attack_roll.total,
                target_ac,
                weapon: weapon.name.clone(),
            },
        );
        return AttackResult {
            attack_roll,
            target_ac,
            hit: false,
            critical: false,
            damage: 0,
            part: None,
            status: None,
            destroyed: false,
            killed: false,
        };
    }

    // Damage roll. On crit, max possible dice + bonus + str_mod.
    let damage = {
        let mut rng = world.resource_mut::<Rng>();
        let raw = if critical {
            (weapon.dice.count as i32) * (weapon.dice.sides as i32)
        } else {
            roll_dice(&mut rng, weapon.dice.count, weapon.dice.sides)
        };
        (raw + weapon.dice.bonus + attack_mod).max(1)
    };

    // Pick a body part to wound. If the target has no anatomy, the
    // hit lands on aggregate Health directly.
    let part_entity = match pick_body_part(world, target) {
        Some(e) => e,
        None => {
            return apply_bodyless_hit(
                world, attacker, target, weapon, attack_roll, target_ac, damage, critical,
            );
        }
    };
    let part_kind = match world.get::<BodyPartKind>(part_entity).copied() {
        Some(k) => k,
        None => {
            return AttackResult {
                attack_roll,
                target_ac,
                hit: false,
                critical: false,
                damage: 0,
                part: None,
                status: None,
                destroyed: false,
                killed: false,
            };
        }
    };

    let (status, destroyed) = apply_part_damage(world, part_entity, &weapon, damage);

    // Aggregate health takes a portion of the local damage.
    let aggregate_damage = (damage / 2).max(1);
    if let Some(mut h) = world.get_mut::<Health>(target) {
        h.current -= aggregate_damage;
    }

    // Critical destructions: heart, head, or neck — instant kill.
    if destroyed && part_kind.is_critical() {
        if let Some(mut h) = world.get_mut::<Health>(target) {
            h.current = 0;
        }
    }

    if critical {
        push_event(
            world,
            Event::CriticalHit {
                attacker,
                target,
                weapon: weapon.name.clone(),
            },
        );
    }
    push_event(
        world,
        Event::BodyPartWounded {
            entity: target,
            part: part_kind,
            damage,
            status,
            weapon: weapon.name.clone(),
        },
    );
    if destroyed {
        push_event(
            world,
            Event::BodyPartDestroyed {
                entity: target,
                part: part_kind,
                status,
                weapon: weapon.name.clone(),
            },
        );
    }

    let remaining_health = world.get::<Health>(target).map(|h| h.current).unwrap_or(0);
    push_event(
        world,
        Event::EntityAttacked {
            attacker: Some(attacker),
            target,
            damage,
            remaining_health,
        },
    );

    let dead = !creature_alive(world, target);
    if dead {
        push_event(
            world,
            Event::EntityKilled {
                entity: target,
                by: Some(attacker),
            },
        );
    }

    AttackResult {
        attack_roll,
        target_ac,
        hit: true,
        critical,
        damage,
        part: Some(part_kind),
        status: Some(status),
        destroyed,
        killed: dead,
    }
}

/// Damage path for entities without anatomy (zombies, training
/// dummies, debug spawns). Skips the body-part roll and applies
/// damage straight to aggregate Health, still emitting hit / kill
/// events. Returns an `AttackResult` with `part = None`.
#[allow(clippy::too_many_arguments)]
fn apply_bodyless_hit(
    world: &mut World,
    attacker: Entity,
    target: Entity,
    weapon: WeaponInfo,
    attack_roll: RollResult,
    target_ac: i32,
    damage: i32,
    critical: bool,
) -> AttackResult {
    let (remaining, killed) = {
        let mut h = match world.get_mut::<Health>(target) {
            Some(h) => h,
            None => {
                return AttackResult {
                    attack_roll,
                    target_ac,
                    hit: false,
                    critical: false,
                    damage: 0,
                    part: None,
                    status: None,
                    destroyed: false,
                    killed: false,
                };
            }
        };
        h.current = (h.current - damage).max(0);
        (h.current, h.current == 0)
    };
    if critical {
        push_event(
            world,
            Event::CriticalHit {
                attacker,
                target,
                weapon: weapon.name.clone(),
            },
        );
    }
    push_event(
        world,
        Event::EntityAttacked {
            attacker: Some(attacker),
            target,
            damage,
            remaining_health: remaining,
        },
    );
    if killed {
        push_event(
            world,
            Event::EntityKilled {
                entity: target,
                by: Some(attacker),
            },
        );
    }
    AttackResult {
        attack_roll,
        target_ac,
        hit: true,
        critical,
        damage,
        part: None,
        status: None,
        destroyed: false,
        killed,
    }
}

fn describe_weapon(world: &World, attacker: Entity) -> WeaponInfo {
    let weapon = world
        .get::<Wearing>(attacker)
        .and_then(|w| w.get(BodySlot::MainHand));
    if let Some(weapon) = weapon {
        let name = world
            .get::<ItemName>(weapon)
            .map(|n| n.0.clone())
            .unwrap_or_else(|| "weapon".into());
        let dice = world
            .get::<DamageDice>(weapon)
            .copied()
            .unwrap_or(DamageDice::new(1, 4));
        let sharp = matches!(world.get::<Texture>(weapon).copied(), Some(Texture::Sharp));
        WeaponInfo { name, dice, sharp }
    } else {
        WeaponInfo {
            name: "fist".into(),
            dice: DamageDice::new(1, 3),
            sharp: false,
        }
    }
}

/// 10 + DEX modifier + sum of all worn item ArmorBonus.
pub fn armor_class(world: &World, entity: Entity) -> i32 {
    let dex_mod = world
        .get::<Stats>(entity)
        .map(|s| s.dex_mod())
        .unwrap_or(0);
    let armor_bonus: i32 = world
        .get::<Wearing>(entity)
        .map(|w| {
            w.iter()
                .filter_map(|(_, item)| world.get::<ArmorBonus>(item).map(|b| b.0))
                .sum()
        })
        .unwrap_or(0);
    10 + dex_mod + armor_bonus
}

fn pick_body_part(world: &mut World, target: Entity) -> Option<Entity> {
    let candidates: Vec<(Entity, u32)> = {
        let mut q = world.query::<(Entity, &PartOf, &HitWeight, &PartHealth)>();
        q.iter(world)
            .filter(|(_, parent, _, ph)| parent.0 == target && !ph.status.destroyed())
            .map(|(e, _, w, _)| (e, w.0))
            .filter(|(_, w)| *w > 0)
            .collect()
    };
    if candidates.is_empty() {
        return None;
    }
    let total: u32 = candidates.iter().map(|(_, w)| w).sum();
    let pick = world.resource_mut::<Rng>().range(total);
    let mut acc = 0u32;
    for (entity, weight) in candidates {
        acc += weight;
        if pick < acc {
            return Some(entity);
        }
    }
    None
}

fn apply_part_damage(
    world: &mut World,
    part_entity: Entity,
    weapon: &WeaponInfo,
    damage: i32,
) -> (PartStatus, bool) {
    let mut ph = world
        .get_mut::<PartHealth>(part_entity)
        .expect("hit a part without PartHealth");
    ph.current -= damage;
    let status = if ph.current <= 0 {
        if weapon.sharp {
            PartStatus::Severed
        } else {
            PartStatus::Crushed
        }
    } else if ph.current < ph.max / 2 {
        if weapon.sharp {
            PartStatus::Cut
        } else {
            PartStatus::Broken
        }
    } else {
        PartStatus::Bruised
    };
    ph.status = status;
    let destroyed = status.destroyed();
    (status, destroyed)
}

fn creature_alive(world: &mut World, creature: Entity) -> bool {
    let aggregate_alive = world
        .get::<Health>(creature)
        .map(|h| h.current > 0)
        .unwrap_or(true);
    aggregate_alive && anatomy_alive(world, creature)
}

fn push_event(world: &mut World, event: Event) {
    let tick = world.resource::<Clock>().tick;
    world.resource_mut::<EventLog>().push(tick, event);
}
