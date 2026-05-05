//! Combat resolution.
//!
//! `resolve_attack(world, attacker, target)` picks a body part on the
//! target weighted by its `HitWeight`, computes damage from the
//! attacker's main-hand weapon (mass + texture), applies it to both the
//! local part HP and the creature's aggregate `Health`, and decides
//! whether the part is bruised, broken, crushed, or severed. Critical
//! organ destruction (heart, neck, decapitation) instantly drops the
//! creature's aggregate health to zero.

use bevy_ecs::prelude::{Entity, World};

use crate::anatomy::{
    anatomy_alive, BodyPartKind, HitWeight, PartHealth, PartOf, PartStatus,
};
use crate::components::Health;
use crate::items::{BodySlot, ItemName, Mass, Texture, Wearing};
use crate::log::{Event, EventLog};
use crate::rng::Rng;
use crate::time::Clock;

/// Summary of a single resolved blow.
#[derive(Copy, Clone, Debug)]
pub struct AttackResult {
    pub damage: i32,
    pub part: BodyPartKind,
    pub status: PartStatus,
    pub destroyed: bool,
    pub killed: bool,
}

#[derive(Clone, Debug)]
struct WeaponInfo {
    name: String,
    mass: f32,
    texture: Option<Texture>,
}

pub fn resolve_attack(
    world: &mut World,
    attacker: Entity,
    target: Entity,
) -> Option<AttackResult> {
    let weapon = describe_weapon(world, attacker);
    let damage = compute_damage(&weapon);

    let part_entity = pick_body_part(world, target)?;
    let part_kind = *world.get::<BodyPartKind>(part_entity)?;

    let (status, destroyed) = apply_part_damage(world, part_entity, &weapon, damage);

    // Aggregate health takes a portion of the local damage.
    let aggregate_damage = (damage / 2).max(1);
    if let Some(mut h) = world.get_mut::<Health>(target) {
        h.current -= aggregate_damage;
    }

    // Critical destructions: heart, head, or neck — instant kill.
    let critical = matches!(
        part_kind,
        BodyPartKind::Heart | BodyPartKind::Head | BodyPartKind::Neck
    );
    if destroyed && critical {
        if let Some(mut h) = world.get_mut::<Health>(target) {
            h.current = 0;
        }
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

    Some(AttackResult {
        damage,
        part: part_kind,
        status,
        destroyed,
        killed: dead,
    })
}

fn describe_weapon(world: &World, attacker: Entity) -> WeaponInfo {
    let weapon = world
        .get::<Wearing>(attacker)
        .and_then(|w| w.get(BodySlot::MainHand));
    if let Some(weapon) = weapon {
        WeaponInfo {
            name: world
                .get::<ItemName>(weapon)
                .map(|n| n.0.clone())
                .unwrap_or_else(|| "weapon".into()),
            mass: world.get::<Mass>(weapon).map(|m| m.0).unwrap_or(0.5),
            texture: world.get::<Texture>(weapon).copied(),
        }
    } else {
        WeaponInfo {
            name: "fist".into(),
            mass: 0.5,
            texture: Some(Texture::Soft),
        }
    }
}

fn compute_damage(weapon: &WeaponInfo) -> i32 {
    let base = (weapon.mass * 10.0).round() as i32;
    let modifier = match weapon.texture {
        Some(Texture::Sharp) => 1.5,
        Some(Texture::Polished) | Some(Texture::Smooth) => 1.0,
        Some(Texture::Rough) | Some(Texture::Coarse) | Some(Texture::Bumpy) => 0.9,
        Some(Texture::Sticky) | Some(Texture::Slick) => 0.7,
        Some(Texture::Soft) | Some(Texture::Furry) => 0.5,
        None => 1.0,
    };
    ((base as f32 * modifier).round() as i32).max(1)
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
    let sharp = matches!(weapon.texture, Some(Texture::Sharp));
    let mut ph = world
        .get_mut::<PartHealth>(part_entity)
        .expect("hit a part without PartHealth");
    ph.current -= damage;
    let status = if ph.current <= 0 {
        if sharp {
            PartStatus::Severed
        } else {
            PartStatus::Crushed
        }
    } else if ph.current < ph.max / 2 {
        if sharp {
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
