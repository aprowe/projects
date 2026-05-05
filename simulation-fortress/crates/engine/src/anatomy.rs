//! Anatomy: body parts as ECS entities.
//!
//! Each creature is associated with a tree of body-part entities tagged
//! with `BodyPart`. A part stores its kind (`BodyPartKind`), back-link
//! to the creature (`PartOf`), local hit points (`PartHealth`), and a
//! relative size used for weighted random hit selection (`HitWeight`).
//! It can also carry an optional `PartLabel` (human-readable name,
//! e.g. "left wing", "venomous fang") and `ProvidesFunctions` listing
//! which abilities the part contributes to its creature.
//!
//! Bodies are described as data via `BodyPlan { parts: Vec<BodyPartSpec> }`
//! and applied with `apply_body_plan(world, creature, plan)`. The
//! engine ships preset plans for humanoids, quadrupeds, and dragons;
//! scenarios can build their own.

use bevy_ecs::prelude::{Component, Entity, World};

/// Marker: this entity is a body part, not a creature or item.
#[derive(Component, Copy, Clone, Debug)]
pub struct BodyPart;

/// What this part is. The variants here are deliberately broad —
/// scenarios pick the kind that best matches and can attach a
/// `PartLabel` for display ("venomous fang", "feathery wing").
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
    LeftWing,
    RightWing,
    Tail,
    Horn,
    Claw,
    /// Catch-all for parts that don't fit any specific variant.
    /// Always pair with a `PartLabel` so it has a name in narration.
    Other,
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
            BodyPartKind::LeftWing => "left wing",
            BodyPartKind::RightWing => "right wing",
            BodyPartKind::Tail => "tail",
            BodyPartKind::Horn => "horn",
            BodyPartKind::Claw => "claw",
            BodyPartKind::Other => "appendage",
        }
    }

    /// Whether destroying this part should instantly kill the creature.
    pub fn is_critical(self) -> bool {
        matches!(
            self,
            BodyPartKind::Heart | BodyPartKind::Head | BodyPartKind::Neck
        )
    }
}

/// Optional human-readable label. Overrides `BodyPartKind::label()`
/// for display. Useful with `BodyPartKind::Other` ("scaly hide",
/// "venomous fang") and to give species-specific flavor to standard
/// parts ("clawed hand").
#[derive(Component, Clone, Debug)]
pub struct PartLabel(pub String);

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
    Flight,
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
            Function::Flight => "flight",
        }
    }
}

/// Functions this part contributes when intact. Stored as a component
/// rather than a global table so dragons can have wings that grant
/// `Flight` and humanoid hands that grant `Grasp` without coupling
/// the engine to a fixed taxonomy.
#[derive(Component, Clone, Debug, Default)]
pub struct ProvidesFunctions(pub Vec<Function>);

/// Default function set for built-in `BodyPartKind` variants. Used by
/// preset body plans; you can always override per-part with a manual
/// `ProvidesFunctions` value in your own plan.
pub fn default_functions(kind: BodyPartKind) -> Vec<Function> {
    match kind {
        BodyPartKind::LeftEye | BodyPartKind::RightEye => vec![Function::Vision],
        BodyPartKind::LeftEar | BodyPartKind::RightEar => vec![Function::Hearing],
        BodyPartKind::Nose => vec![Function::Smell],
        BodyPartKind::Mouth => vec![Function::Speech],
        BodyPartKind::Tongue => vec![Function::Speech],
        BodyPartKind::LeftHand | BodyPartKind::RightHand => vec![Function::Grasp],
        BodyPartKind::LeftLeg | BodyPartKind::RightLeg => vec![Function::Mobility],
        BodyPartKind::LeftFoot | BodyPartKind::RightFoot => vec![Function::Mobility],
        BodyPartKind::Heart => vec![Function::Vitality],
        BodyPartKind::LeftLung | BodyPartKind::RightLung => vec![Function::Breathing],
        BodyPartKind::LeftWing | BodyPartKind::RightWing => vec![Function::Flight],
        BodyPartKind::Claw => vec![Function::Grasp],
        _ => vec![],
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

    pub fn capacity(self) -> f32 {
        match self {
            PartStatus::Intact => 1.0,
            PartStatus::Bruised => 0.9,
            PartStatus::Cut => 0.7,
            PartStatus::Broken => 0.4,
            PartStatus::Crushed | PartStatus::Severed => 0.0,
        }
    }

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

// ─── data-driven body plans ─────────────────────────────────────────────────

/// Description of a single body part within a plan.
#[derive(Clone, Debug)]
pub struct BodyPartSpec {
    pub kind: BodyPartKind,
    pub label: Option<String>,
    pub max_hp: i32,
    pub hit_weight: u32,
    /// If `None`, falls back to `default_functions(kind)`.
    pub functions: Option<Vec<Function>>,
}

impl BodyPartSpec {
    pub const fn new(kind: BodyPartKind, max_hp: i32, hit_weight: u32) -> Self {
        Self {
            kind,
            label: None,
            max_hp,
            hit_weight,
            functions: None,
        }
    }

    pub fn labeled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn providing(mut self, functions: Vec<Function>) -> Self {
        self.functions = Some(functions);
        self
    }
}

/// A creature's full body, as data. Apply with `apply_body_plan`.
#[derive(Clone, Debug, Default)]
pub struct BodyPlan {
    pub parts: Vec<BodyPartSpec>,
}

impl BodyPlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, spec: BodyPartSpec) -> Self {
        self.parts.push(spec);
        self
    }
}

/// Spawn the parts described by `plan`, attaching each to `creature`.
/// Returns the new body-part entities.
pub fn apply_body_plan(
    world: &mut World,
    creature: Entity,
    plan: &BodyPlan,
) -> Vec<Entity> {
    plan.parts
        .iter()
        .map(|spec| {
            let functions = spec
                .functions
                .clone()
                .unwrap_or_else(|| default_functions(spec.kind));
            let mut e = world.spawn((
                BodyPart,
                spec.kind,
                PartOf(creature),
                PartHealth::new(spec.max_hp),
                HitWeight(spec.hit_weight),
                ProvidesFunctions(functions),
            ));
            if let Some(label) = &spec.label {
                e.insert(PartLabel(label.clone()));
            }
            e.id()
        })
        .collect()
}

/// Convenience for the common case: spawn the standard humanoid plan.
pub fn spawn_humanoid_body(world: &mut World, creature: Entity) -> Vec<Entity> {
    apply_body_plan(world, creature, &humanoid_body_plan())
}

// ─── preset body plans ──────────────────────────────────────────────────────

/// Standard humanoid body: 22 parts.
pub fn humanoid_body_plan() -> BodyPlan {
    BodyPlan::new()
        .with(BodyPartSpec::new(BodyPartKind::Head, 30, 8))
        .with(BodyPartSpec::new(BodyPartKind::LeftEye, 5, 2))
        .with(BodyPartSpec::new(BodyPartKind::RightEye, 5, 2))
        .with(BodyPartSpec::new(BodyPartKind::LeftEar, 5, 3))
        .with(BodyPartSpec::new(BodyPartKind::RightEar, 5, 3))
        .with(BodyPartSpec::new(BodyPartKind::Nose, 5, 2))
        .with(BodyPartSpec::new(BodyPartKind::Mouth, 5, 1))
        .with(BodyPartSpec::new(BodyPartKind::Tongue, 5, 0))
        .with(BodyPartSpec::new(BodyPartKind::Neck, 20, 2))
        .with(BodyPartSpec::new(BodyPartKind::Torso, 50, 25))
        .with(BodyPartSpec::new(BodyPartKind::Heart, 15, 0))
        .with(BodyPartSpec::new(BodyPartKind::LeftLung, 15, 0))
        .with(BodyPartSpec::new(BodyPartKind::RightLung, 15, 0))
        .with(BodyPartSpec::new(BodyPartKind::Stomach, 15, 0))
        .with(BodyPartSpec::new(BodyPartKind::LeftArm, 25, 8))
        .with(BodyPartSpec::new(BodyPartKind::RightArm, 25, 8))
        .with(BodyPartSpec::new(BodyPartKind::LeftHand, 15, 4))
        .with(BodyPartSpec::new(BodyPartKind::RightHand, 15, 4))
        .with(BodyPartSpec::new(BodyPartKind::LeftLeg, 35, 12))
        .with(BodyPartSpec::new(BodyPartKind::RightLeg, 35, 12))
        .with(BodyPartSpec::new(BodyPartKind::LeftFoot, 15, 4))
        .with(BodyPartSpec::new(BodyPartKind::RightFoot, 15, 4))
}

/// Four-legged ground creature: dog, deer, lion. No hands; legs do
/// double duty for mobility.
pub fn quadruped_body_plan() -> BodyPlan {
    BodyPlan::new()
        .with(BodyPartSpec::new(BodyPartKind::Head, 25, 8))
        .with(BodyPartSpec::new(BodyPartKind::LeftEye, 4, 2))
        .with(BodyPartSpec::new(BodyPartKind::RightEye, 4, 2))
        .with(BodyPartSpec::new(BodyPartKind::LeftEar, 4, 3))
        .with(BodyPartSpec::new(BodyPartKind::RightEar, 4, 3))
        .with(BodyPartSpec::new(BodyPartKind::Nose, 4, 2))
        .with(BodyPartSpec::new(BodyPartKind::Mouth, 4, 2))
        .with(BodyPartSpec::new(BodyPartKind::Tongue, 4, 0))
        .with(BodyPartSpec::new(BodyPartKind::Neck, 20, 3))
        .with(BodyPartSpec::new(BodyPartKind::Torso, 60, 28))
        .with(BodyPartSpec::new(BodyPartKind::Heart, 15, 0))
        .with(BodyPartSpec::new(BodyPartKind::LeftLung, 15, 0))
        .with(BodyPartSpec::new(BodyPartKind::RightLung, 15, 0))
        .with(BodyPartSpec::new(BodyPartKind::Stomach, 15, 0))
        .with(BodyPartSpec::new(BodyPartKind::LeftLeg, 30, 8).labeled("front-left leg"))
        .with(BodyPartSpec::new(BodyPartKind::RightLeg, 30, 8).labeled("front-right leg"))
        .with(BodyPartSpec::new(BodyPartKind::LeftFoot, 12, 3).labeled("front-left paw"))
        .with(BodyPartSpec::new(BodyPartKind::RightFoot, 12, 3).labeled("front-right paw"))
        // Reuse the leg/foot enums for the rear pair; PartLabel keeps narration honest.
        .with(BodyPartSpec::new(BodyPartKind::LeftArm, 30, 8).labeled("rear-left leg").providing(vec![Function::Mobility]))
        .with(BodyPartSpec::new(BodyPartKind::RightArm, 30, 8).labeled("rear-right leg").providing(vec![Function::Mobility]))
        .with(BodyPartSpec::new(BodyPartKind::LeftHand, 12, 3).labeled("rear-left paw").providing(vec![Function::Mobility]))
        .with(BodyPartSpec::new(BodyPartKind::RightHand, 12, 3).labeled("rear-right paw").providing(vec![Function::Mobility]))
        .with(BodyPartSpec::new(BodyPartKind::Tail, 15, 4))
}

/// Big winged reptile: tougher than humanoid, with claws for grasping
/// and wings that confer Flight. Horns are decorative for now.
pub fn dragon_body_plan() -> BodyPlan {
    BodyPlan::new()
        .with(BodyPartSpec::new(BodyPartKind::Head, 80, 10))
        .with(BodyPartSpec::new(BodyPartKind::LeftEye, 8, 2))
        .with(BodyPartSpec::new(BodyPartKind::RightEye, 8, 2))
        .with(BodyPartSpec::new(BodyPartKind::LeftEar, 6, 2))
        .with(BodyPartSpec::new(BodyPartKind::RightEar, 6, 2))
        .with(BodyPartSpec::new(BodyPartKind::Nose, 6, 2))
        .with(BodyPartSpec::new(BodyPartKind::Mouth, 25, 4).labeled("fanged maw"))
        .with(BodyPartSpec::new(BodyPartKind::Tongue, 8, 0))
        .with(BodyPartSpec::new(BodyPartKind::Horn, 30, 3).labeled("left horn"))
        .with(BodyPartSpec::new(BodyPartKind::Horn, 30, 3).labeled("right horn"))
        .with(BodyPartSpec::new(BodyPartKind::Neck, 50, 4))
        .with(BodyPartSpec::new(BodyPartKind::Torso, 200, 30))
        .with(BodyPartSpec::new(BodyPartKind::Heart, 40, 0))
        .with(BodyPartSpec::new(BodyPartKind::LeftLung, 30, 0))
        .with(BodyPartSpec::new(BodyPartKind::RightLung, 30, 0))
        .with(BodyPartSpec::new(BodyPartKind::Stomach, 40, 0).labeled("furnace stomach"))
        .with(BodyPartSpec::new(BodyPartKind::LeftWing, 60, 8))
        .with(BodyPartSpec::new(BodyPartKind::RightWing, 60, 8))
        .with(BodyPartSpec::new(BodyPartKind::LeftLeg, 70, 10))
        .with(BodyPartSpec::new(BodyPartKind::RightLeg, 70, 10))
        .with(BodyPartSpec::new(BodyPartKind::Claw, 25, 4).labeled("left foreclaw"))
        .with(BodyPartSpec::new(BodyPartKind::Claw, 25, 4).labeled("right foreclaw"))
        .with(BodyPartSpec::new(BodyPartKind::Tail, 60, 6))
}

/// Smaller flying creature: head, wings, light body, no arms or
/// real legs. Beak provides bite; wings provide Flight.
pub fn bird_body_plan() -> BodyPlan {
    BodyPlan::new()
        .with(BodyPartSpec::new(BodyPartKind::Head, 8, 6))
        .with(BodyPartSpec::new(BodyPartKind::LeftEye, 2, 2))
        .with(BodyPartSpec::new(BodyPartKind::RightEye, 2, 2))
        .with(BodyPartSpec::new(BodyPartKind::Mouth, 3, 3).labeled("beak"))
        .with(BodyPartSpec::new(BodyPartKind::Neck, 5, 2))
        .with(BodyPartSpec::new(BodyPartKind::Torso, 18, 16))
        .with(BodyPartSpec::new(BodyPartKind::Heart, 4, 0))
        .with(BodyPartSpec::new(BodyPartKind::LeftLung, 4, 0))
        .with(BodyPartSpec::new(BodyPartKind::RightLung, 4, 0))
        .with(BodyPartSpec::new(BodyPartKind::Stomach, 4, 0))
        .with(BodyPartSpec::new(BodyPartKind::LeftWing, 12, 8))
        .with(BodyPartSpec::new(BodyPartKind::RightWing, 12, 8))
        .with(BodyPartSpec::new(BodyPartKind::LeftLeg, 6, 4).labeled("left talon leg"))
        .with(BodyPartSpec::new(BodyPartKind::RightLeg, 6, 4).labeled("right talon leg"))
        .with(BodyPartSpec::new(BodyPartKind::Claw, 4, 1).labeled("left talon"))
        .with(BodyPartSpec::new(BodyPartKind::Claw, 4, 1).labeled("right talon"))
        .with(BodyPartSpec::new(BodyPartKind::Tail, 6, 4).labeled("tail feathers"))
}

/// Long, legless reptile: head with fangs, segmented body, tail.
/// No legs, wings, or arms.
pub fn snake_body_plan() -> BodyPlan {
    BodyPlan::new()
        .with(BodyPartSpec::new(BodyPartKind::Head, 12, 8))
        .with(BodyPartSpec::new(BodyPartKind::LeftEye, 2, 2))
        .with(BodyPartSpec::new(BodyPartKind::RightEye, 2, 2))
        .with(BodyPartSpec::new(BodyPartKind::Mouth, 8, 5).labeled("fanged jaw"))
        .with(BodyPartSpec::new(BodyPartKind::Tongue, 4, 0).labeled("forked tongue"))
        .with(BodyPartSpec::new(BodyPartKind::Neck, 8, 4))
        .with(BodyPartSpec::new(BodyPartKind::Torso, 30, 60).labeled("coiled body"))
        .with(BodyPartSpec::new(BodyPartKind::Heart, 5, 0))
        .with(BodyPartSpec::new(BodyPartKind::Stomach, 5, 0))
        .with(BodyPartSpec::new(BodyPartKind::Tail, 12, 8))
}

// ─── queries ────────────────────────────────────────────────────────────────

/// Total functional capacity a creature has for `function`. Reads
/// each body part's `ProvidesFunctions` (preferred) or falls back to
/// `default_functions(kind)`.
pub fn function_capacity(world: &mut World, creature: Entity, function: Function) -> f32 {
    let mut total = 0.0;
    let mut q = world.query::<(
        &PartOf,
        &BodyPartKind,
        &PartHealth,
        Option<&ProvidesFunctions>,
    )>();
    for (parent, kind, health, provides) in q.iter(world) {
        if parent.0 != creature {
            continue;
        }
        let provided: &[Function] = match provides {
            Some(p) => &p.0,
            None => &[],
        };
        let has_function = if provided.is_empty() {
            default_functions(*kind).contains(&function)
        } else {
            provided.contains(&function)
        };
        if !has_function {
            continue;
        }
        total += health.status.capacity();
    }
    total
}

pub fn anatomy_alive(world: &mut World, creature: Entity) -> bool {
    function_capacity(world, creature, Function::Vitality) > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_creature(world: &mut World) -> Entity {
        world.spawn(()).id()
    }

    #[test]
    fn humanoid_plan_grants_two_units_of_each_paired_function() {
        let mut world = World::new();
        let creature = fresh_creature(&mut world);
        apply_body_plan(&mut world, creature, &humanoid_body_plan());
        assert_eq!(function_capacity(&mut world, creature, Function::Vision), 2.0);
        assert_eq!(function_capacity(&mut world, creature, Function::Hearing), 2.0);
        assert_eq!(function_capacity(&mut world, creature, Function::Mobility), 4.0);
        assert_eq!(function_capacity(&mut world, creature, Function::Flight), 0.0);
    }

    #[test]
    fn dragon_plan_grants_flight_via_wings() {
        let mut world = World::new();
        let creature = fresh_creature(&mut world);
        apply_body_plan(&mut world, creature, &dragon_body_plan());
        assert_eq!(function_capacity(&mut world, creature, Function::Flight), 2.0);
        assert!(function_capacity(&mut world, creature, Function::Grasp) > 0.0);
    }

    #[test]
    fn quadruped_plan_grants_four_legs_of_mobility() {
        let mut world = World::new();
        let creature = fresh_creature(&mut world);
        apply_body_plan(&mut world, creature, &quadruped_body_plan());
        // four legs + four feet/paws, all contributing Mobility
        assert_eq!(function_capacity(&mut world, creature, Function::Mobility), 8.0);
        assert_eq!(function_capacity(&mut world, creature, Function::Grasp), 0.0);
    }

    #[test]
    fn destroyed_part_drops_capacity() {
        let mut world = World::new();
        let creature = fresh_creature(&mut world);
        let parts = apply_body_plan(&mut world, creature, &humanoid_body_plan());
        // Find an ear and destroy it.
        let ear = parts
            .iter()
            .copied()
            .find(|e| matches!(world.get::<BodyPartKind>(*e), Some(BodyPartKind::LeftEar)))
            .unwrap();
        world.get_mut::<PartHealth>(ear).unwrap().status = PartStatus::Severed;
        assert_eq!(function_capacity(&mut world, creature, Function::Hearing), 1.0);
    }
}
