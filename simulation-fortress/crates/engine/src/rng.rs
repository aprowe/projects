//! Seeded pseudo-random number generator resource.
//!
//! Uses splitmix64 — small, fast, and adequate for game-style randomness.
//! Determinism is on by default: the same seed produces the same run, so
//! scenarios are reproducible from the event log.

use bevy_ecs::resource::Resource;

#[derive(Resource, Clone, Copy, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn from_seed(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0xDEAD_BEEF_CAFE_F00D } else { seed },
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform integer in `0..upper`.
    pub fn range(&mut self, upper: u32) -> u32 {
        if upper == 0 {
            return 0;
        }
        (self.next_u64() % upper as u64) as u32
    }

    /// Returns true with probability `p` (clamped to `0.0..=1.0`).
    pub fn chance(&mut self, p: f32) -> bool {
        let p = p.clamp(0.0, 1.0);
        let v = (self.next_u64() >> 32) as f32 / (u32::MAX as f32);
        v < p
    }
}
