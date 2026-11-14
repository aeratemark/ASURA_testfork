//! Experience and levels: the curve a total experience falls on, and the points a level grants.
//!
//! The numbers are the pack's (`config/World/Vanilla/experience.json`); this module holds the rule and
//! no value of its own (`AGENTS.md` 4a). The curve is preforged's (`gw_experience_level.h`): from level
//! L, the next level costs `first + step * (L - 1)`, which puts level 20 at 140,600 total experience.
//! The same file records the character sheet's experience as the first of the fifteen dwords
//! `GAME_SMSG_CHARACTER_UPDATE_FACTIONS` carries, and the level as its tenth.

/// The experience curve and the skill points a level grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Curve {
    /// Total experience level 1 to 2 costs.
    pub first_level_xp: u32,
    /// How much more each later level costs than the one before it.
    pub xp_per_level_step: u32,
    /// The highest level the curve reaches.
    pub max_level: u32,
    /// Skill points a level grants, on top of the previous level's total.
    pub skill_points_per_level: u32,
}

impl Curve {
    /// The total experience a character at `level` has at least.
    ///
    /// Level 1 is zero. Level 2 is `first_level_xp`. Each step after that adds `xp_per_level_step` more
    /// than the previous step, so the cost is a triangle number, not a flat multiple.
    pub fn threshold(&self, level: u32) -> u32 {
        let level = level.clamp(1, self.max_level.max(1));
        let steps = level - 1;
        steps * self.first_level_xp + self.xp_per_level_step * steps * steps.saturating_sub(1) / 2
    }

    /// The level a total experience falls on.
    ///
    /// The client's own rule is "at least the threshold", not "just below the next one": a character
    /// with exactly 2,000 experience is level 2.
    pub fn level_for(&self, total: u32) -> u32 {
        let mut level = 1;
        while level < self.max_level && total >= self.threshold(level + 1) {
            level += 1;
        }
        level
    }

    /// The skill points a level has earned in total. A level 1 character has none.
    pub fn skill_points_for(&self, level: u32) -> u32 {
        let level = level.clamp(1, self.max_level.max(1));
        self.skill_points_per_level * (level - 1)
    }

    /// The most experience the curve gives a name to: the level cap's own threshold.
    pub fn max_total(&self) -> u32 {
        self.threshold(self.max_level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values the pack ships, so the test reads as the game's own curve.
    fn curve() -> Curve {
        Curve {
            first_level_xp: 2000,
            xp_per_level_step: 600,
            max_level: 20,
            skill_points_per_level: 1,
        }
    }

    #[test]
    fn the_curve_is_the_reference_curve() {
        let curve = curve();
        assert_eq!(curve.threshold(1), 0);
        assert_eq!(curve.threshold(2), 2000);
        assert_eq!(curve.threshold(3), 4600);
        assert_eq!(curve.threshold(20), 140_600, "the level 20 threshold");
        assert_eq!(curve.level_for(0), 1);
        assert_eq!(curve.level_for(1999), 1);
        assert_eq!(curve.level_for(2000), 2);
        assert_eq!(curve.level_for(4599), 2);
        assert_eq!(curve.level_for(4600), 3);
        assert_eq!(curve.level_for(140_599), 19);
        assert_eq!(curve.level_for(140_600), 20);
        assert_eq!(curve.level_for(u32::MAX), 20);
    }

    #[test]
    fn a_level_grants_one_skill_point_and_level_one_grants_none() {
        let curve = curve();
        assert_eq!(curve.skill_points_for(1), 0);
        assert_eq!(curve.skill_points_for(2), 1);
        assert_eq!(curve.skill_points_for(20), 19);
    }

    #[test]
    fn a_curve_with_a_cap_of_one_stays_at_level_one() {
        let curve = Curve {
            first_level_xp: 2000,
            xp_per_level_step: 600,
            max_level: 1,
            skill_points_per_level: 1,
        };
        assert_eq!(curve.threshold(1), 0);
        assert_eq!(curve.threshold(9), 0, "a level past the cap is the cap");
        assert_eq!(curve.level_for(u32::MAX), 1);
        assert_eq!(curve.skill_points_for(9), 0);
    }
}
