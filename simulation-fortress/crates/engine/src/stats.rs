//! D&D-style ability scores. The classic six: STR, DEX, CON, INT,
//! WIS, CHA. Each is a u8 in roughly `[3, 20]` (3 is feeble, 10
//! is average, 20 is peak human). The `modifier` for a stat is
//! `(score - 10) / 2`, so a STR of 16 gives +3 to attack and damage.

use bevy_ecs::prelude::Component;

#[derive(Component, Copy, Clone, Debug)]
pub struct Stats {
    pub str_: u8,
    pub dex: u8,
    pub con: u8,
    pub int: u8,
    pub wis: u8,
    pub cha: u8,
}

impl Stats {
    pub const fn average() -> Self {
        Self {
            str_: 10,
            dex: 10,
            con: 10,
            int: 10,
            wis: 10,
            cha: 10,
        }
    }

    /// Heavy-set thug, swings hard, slow on his feet.
    pub const fn brute() -> Self {
        Self {
            str_: 17,
            dex: 9,
            con: 14,
            int: 8,
            wis: 9,
            cha: 9,
        }
    }

    /// Quick on his feet, dodges well, light hitter.
    pub const fn rogue() -> Self {
        Self {
            str_: 11,
            dex: 17,
            con: 12,
            int: 13,
            wis: 11,
            cha: 13,
        }
    }

    /// Average adult homeowner.
    pub const fn citizen() -> Self {
        Self {
            str_: 11,
            dex: 11,
            con: 12,
            int: 12,
            wis: 11,
            cha: 11,
        }
    }

    /// Frail, smaller frame.
    pub const fn child() -> Self {
        Self {
            str_: 7,
            dex: 13,
            con: 9,
            int: 10,
            wis: 10,
            cha: 11,
        }
    }

    /// Old, wise, fragile.
    pub const fn elder() -> Self {
        Self {
            str_: 8,
            dex: 9,
            con: 9,
            int: 13,
            wis: 14,
            cha: 12,
        }
    }

    pub fn modifier(score: u8) -> i32 {
        ((score as i32) - 10).div_euclid(2)
    }

    pub fn str_mod(&self) -> i32 {
        Self::modifier(self.str_)
    }
    pub fn dex_mod(&self) -> i32 {
        Self::modifier(self.dex)
    }
    pub fn con_mod(&self) -> i32 {
        Self::modifier(self.con)
    }
    pub fn int_mod(&self) -> i32 {
        Self::modifier(self.int)
    }
    pub fn wis_mod(&self) -> i32 {
        Self::modifier(self.wis)
    }
    pub fn cha_mod(&self) -> i32 {
        Self::modifier(self.cha)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_match_dnd_rules() {
        assert_eq!(Stats::modifier(10), 0);
        assert_eq!(Stats::modifier(11), 0);
        assert_eq!(Stats::modifier(12), 1);
        assert_eq!(Stats::modifier(16), 3);
        assert_eq!(Stats::modifier(20), 5);
        assert_eq!(Stats::modifier(8), -1);
        assert_eq!(Stats::modifier(7), -2);
        assert_eq!(Stats::modifier(3), -4);
    }

    #[test]
    fn brute_has_high_strength() {
        let b = Stats::brute();
        assert_eq!(b.str_mod(), 3);
        assert_eq!(b.dex_mod(), -1);
    }

    #[test]
    fn rogue_has_high_dex() {
        let r = Stats::rogue();
        assert_eq!(r.dex_mod(), 3);
        assert_eq!(r.str_mod(), 0);
    }
}
