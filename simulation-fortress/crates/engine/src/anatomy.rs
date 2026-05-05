//! Anatomy: body parts as ECS entities.
//!
//! Each creature is associated with a tree of body-part entities tagged
//! with `BodyPart`. A part stores its kind (`BodyPartKind`), back-link
//! to the creature (`PartOf`), local hit points (`PartHealth`), and a
//! relative size used for weighted random hit selection (`HitWeight`).
//!
//! Body parts confer functions (vision, hearing, mobility, ...) on the
//! creature; those functions degrade as parts get bruised, broken,
//! crushed, or severed.

use bevy_ecs::prelude::{Component, Entity, World};

/// Marker: this entity is a body part, not a creature or item.
#[derive(Component, Copy, Clone, Debug)]
pub struct BodyPart;

/// What this part is. Anatomy here is humanoid-shaped; future scenarios
/// can introduce extra variants (Wing, Tail, Tentacle) by extending the
/// enum or, more flexibly, by switching to a string-tagged kind.
#[derive(Component, Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum BodyPartKind {
    Head,
    LeftEye,
    RightEye,
    LeftEar,
    RightEar,
    Nose,
    Mouth,
    Tongue,
    Neck,
    Torso,
    Heart,
    LeftLung,
    RightLung,
    Stomach,
    LeftArm,
    RightArm,
    LeftHand,
    RightHand,
    LeftLeg,
    RightLeg,
    LeftFoot,
    RightFoot,
}

impl BodyPartKind {
    pub fn label(self) -> &'static str {
        match self {
            BodyPartKind::Head => "head",
            BodyPartKind::LeftEye => "left eye",
            BodyPartKind::RightEye => "right eye",
            BodyPartKind::LeftEar => "left ear",
            BodyPartKind::RightEar => "right ear",
            BodyPartKind::Nose => "nose",
            BodyPartKind::Mouth => "mouth",
            BodyPartKind::Tongue => "tongue",
            BodyPartKind::Neck => "neck",
            BodyPartKind::Torso => "torso",
            BodyPartKind::Heart => "heart",
            BodyPartKind::LeftLung => "left lung",
            BodyPartKind::RightLung => "right lung",
            BodyPartKind::Stomach => "stomach",
            BodyPartKind::LeftArm => "left arm",
            BodyPartKind::RightArm => "right arm",
            BodyPartKind::LeftHand => "left hand",
            BodyPartKind::RightHand => "right hand",
            BodyPartKind::LeftLeg => "left leg",
            BodyPartKind::RightLeg => "right leg",
            BodyPartKind::LeftFoot => "left foot",
            BodyPartKind::RightFoot => "right foot",
        }
    }
}

/// Backlink from a body part to its creature.
#[derive(Component, Copy, Clone, Debug)]
pub struct PartOf(pub Entity);

/// Relative weight for random hit selection. 0 means the part isn't a
/// candidate for direct external blows (typical for internal organs).
#[derive(Component, Copy, Clone, Debug)]
pub struct HitWeight(pub u32);

/// Functional roles a body part can confer on its creature.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Function {
    Vision,
    Hearing,
    Smell,
    Speech,
    Grasp,
    Mobility,
    Vitality,
    Breathing,
}

impl Function {
    pub fn label(self) -> &'static str {
        match self {
            Function::Vision => "vision",
            Function::Hearing => "hearing",
            Function::Smell => "smell",
            Function::Speech => "speech",
            Function::Grasp => "grasp",
            Function::Mobility => "mobility",
            Function::Vitality => "vitality",
            Function::Breathing => "breathing",
        }
    }
}

/// Functions provided by a part when intact.
pub fn part_functions(kind: BodyPartKind) -> &'static [Function] {
    match kind {
        BodyPartKind::LeftEye | BodyPartKind::RightEye => &[Function::Vision],
        BodyPartKind::LeftEar | BodyPartKind::RightEar => &[Function::Hearing],
        BodyPartKind::Nose => &[Function::Smell],
        BodyPartKind::Mouth => &[Function::Speech],
        BodyPartKind::Tongue => &[Function::Speech],
        BodyPartKind::LeftHand | BodyPartKind::RightHand => &[Function::Grasp],
        BodyPartKind::LeftLeg | BodyPartKind::RightLeg => &[Function::Mobility],
        BodyPartKind::LeftFoot | BodyPartKind::RightFoot => &[Function::Mobility],
        BodyPartKind::Heart => &[Function::Vitality],
        BodyPartKind::LeftLung | BodyPartKind::RightLung => &[Function::Breathing],
        _ => &[],
    }
}

/// How damaged a body part is.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum PartStatus {
    #[default]
    Intact,
    Bruised,
    Cut,
    Broken,
    Crushed,
    Severed,
}

impl PartStatus {
    pub fn label(self) -> &'static str {
        match self {
            PartStatus::Intact => "intact",
            PartStatus::Bruised => "bruised",
            PartStatus::Cut => "cut",
            PartStatus::Broken => "broken",
            PartStatus::Crushed => "crushed",
            PartStatus::Severed => "severed",
        }
    }

    /// Fraction of full functional capacity remaining (0.0..=1.0).
    pub fn capacity(self) -> f32 {
        match self {
            PartStatus::Intact => 1.0,
            PartStatus::Bruised => 0.9,
            PartStatus::Cut => 0.7,
            PartStatus::Broken => 0.4,
            PartStatus::Crushed | PartStatus::Severed => 0.0,
        }
    }

    /// True once the part can no longer contribute to its functions.
    pub fn destroyed(self) -> bool {
        matches!(self, PartStatus::Crushed | PartStatus::Severed)
    }
}

#[derive(Component, Copy, Clone, Debug)]
pub struct PartHealth {
    pub current: i32,
    pub max: i32,
    pub status: PartStatus,
}

impl PartHealth {
    pub fn new(max: i32) -> Self {
        Self {
            current: max,
            max,
            status: PartStatus::Intact,
        }
    }
}

/// Layout of a default humanoid: kind, max HP, hit weight.
const HUMANOID_LAYOUT: &[(BodyPartKind, i32, u32)] = &[
    (BodyPartKind::Head, 30, 8),
    (BodyPartKind::LeftEye, 5, 2),
    (BodyPartKind::RightEye, 5, 2),
    (BodyPartKind::LeftEar, 5, 3),
    (BodyPartKind::RightEar, 5, 3),
    (BodyPartKind::Nose, 5, 2),
    (BodyPartKind::Mouth, 5, 1),
    (BodyPartKind::Tongue, 5, 0),
    (BodyPartKind::Neck, 20, 2),
    (BodyPartKind::Torso, 50, 25),
    (BodyPartKind::Heart, 15, 0),
    (BodyPartKind::LeftLung, 15, 0),
    (BodyPartKind::RightLung, 15, 0),
    (BodyPartKind::Stomach, 15, 0),
    (BodyPartKind::LeftArm, 25, 8),
    (BodyPartKind::RightArm, 25, 8),
    (BodyPartKind::LeftHand, 15, 4),
    (BodyPartKind::RightHand, 15, 4),
    (BodyPartKind::LeftLeg, 35, 12),
    (BodyPartKind::RightLeg, 35, 12),
    (BodyPartKind::LeftFoot, 15, 4),
    (BodyPartKind::RightFoot, 15, 4),
];

/// Spawn a default humanoid body for `creature`, returning the body-part
/// entities. Each part gets `BodyPart`, `BodyPartKind`, `PartOf`,
/// `PartHealth`, and `HitWeight` components.
pub fn spawn_humanoid_body(world: &mut World, creature: Entity) -> Vec<Entity> {
    HUMANOID_LAYOUT
        .iter()
        .map(|&(kind, hp, weight)| {
            world
                .spawn((
                    BodyPart,
                    kind,
                    PartOf(creature),
                    PartHealth::new(hp),
                    HitWeight(weight),
                ))
                .id()
        })
        .collect()
}

/// Total functional capacity a creature has for the given function.
/// 1.0 means "as good as one healthy organ", 2.0 means two healthy
/// organs (two ears -> 2.0 hearing), 0.0 means no function remains.
pub fn function_capacity(world: &mut World, creature: Entity, function: Function) -> f32 {
    let mut total = 0.0;
    let mut q = world.query::<(&PartOf, &BodyPartKind, &PartHealth)>();
    for (parent, kind, health) in q.iter(world) {
        if parent.0 != creature {
            continue;
        }
        if !part_functions(*kind).contains(&function) {
            continue;
        }
        total += health.status.capacity();
    }
    total
}

/// True if the creature has any vital function (Vitality > 0).
pub fn anatomy_alive(world: &mut World, creature: Entity) -> bool {
    function_capacity(world, creature, Function::Vitality) > 0.0
}
