//! Quests: the stage a character's attempt is on, and the transitions that may move it.
//!
//! The states and the fences are preforged's `gw_quest_state.h`, whose native work established them:
//! an attempt is `active` -> `completed` -> `rewarded`, or `abandoned`; a reacceptance starts a new
//! cycle, and a delayed packet from an old cycle changes nothing. The set of objective credits is
//! server-owned: a credit is a stable identity the caller supplies, never a recycled client agent id.
//!
//! This module holds decisions and no values. The storage bounds - how many records a profile may hold
//! and how many credits one attempt may carry - are [`Limits`], handed in by the caller
//! (`docs/38-layer-decisions.md`, D34). The quest content, its text, markers and reward amounts, is the
//! pack's (`config/World/<pack>/quests.json`); this module never reads a file and never prints.

use std::collections::{BTreeMap, BTreeSet};

/// The stage of one attempt, at the values preforged stores (`Stage` in `gw_quest_state.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Accepted and not yet complete.
    Active = 1,
    /// Every objective is credited; the turn-in is still owed.
    Completed = 2,
    /// The reward was granted; the log entry is gone.
    Rewarded = 3,
    /// The player abandoned it; the cycle is kept so an old packet cannot revive it.
    Abandoned = 4,
}

impl Stage {
    /// The stored value, as the database and the codec carry it.
    pub fn value(self) -> u32 {
        self as u32
    }

    /// The stage a stored value names, or `None` for anything outside 1 to 4.
    pub fn from_value(value: u32) -> Option<Stage> {
        match value {
            1 => Some(Stage::Active),
            2 => Some(Stage::Completed),
            3 => Some(Stage::Rewarded),
            4 => Some(Stage::Abandoned),
            _ => None,
        }
    }
}

/// The caller's storage bounds: how many records one profile may hold, and how many objective credits
/// one attempt may carry. They are arguments, not constants, because a number that describes what this
/// machine stores belongs in configuration and not in a rule (`config/Server/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most quest records one character profile may hold.
    pub max_records: usize,
    /// The most objective credits one attempt may carry.
    pub max_credits: usize,
}

/// One attempt at one quest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// The attempt number, starting at 1. A reacceptance increments it; a stale request names an old one.
    pub cycle: u32,
    /// Where the attempt is.
    pub stage: Stage,
    /// The objective identities already credited, scoped to this attempt.
    pub credits: BTreeSet<u64>,
}

impl Progress {
    /// A fresh attempt.
    pub fn new() -> Progress {
        Progress {
            cycle: 1,
            stage: Stage::Active,
            credits: BTreeSet::new(),
        }
    }
}

/// Every quest attempt one character has.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    /// Keyed by the client's quest id.
    pub records: BTreeMap<u32, Progress>,
}

impl Profile {
    /// Check the bounds a stored profile must hold.
    pub fn validate(&self, limits: &Limits) -> Result<(), &'static str> {
        if self.records.len() > limits.max_records {
            return Err("the quest profile holds more records than the limit");
        }
        for (id, progress) in &self.records {
            if *id == 0 {
                return Err("a quest record has id 0");
            }
            if progress.cycle == 0 {
                return Err("a quest record has cycle 0");
            }
            if progress.credits.len() > limits.max_credits {
                return Err("a quest attempt holds more credits than the limit");
            }
            if progress.credits.contains(&0) {
                return Err("a quest attempt holds credit 0");
            }
        }
        Ok(())
    }

    /// Accept a quest, or accept an abandoned one again.
    ///
    /// A new record starts at cycle 1 and `active`. An existing record may be reaccepted only when it is
    /// `abandoned`, or when the caller marked the quest repeatable and it is `rewarded`; the new attempt
    /// is the previous cycle plus one. `None` refuses: an active or completed attempt, a rewarded one
    /// that is not repeatable, a full profile, or a cycle counter at its end.
    pub fn accept(&self, id: u32, repeatable: bool, limits: &Limits) -> Option<Profile> {
        if id == 0 {
            return None;
        }
        self.validate(limits).ok()?;
        let mut next = self.clone();
        match next.records.get_mut(&id) {
            None => {
                if next.records.len() >= limits.max_records {
                    return None;
                }
                next.records.insert(id, Progress::new());
                Some(next)
            }
            Some(progress) => {
                let allowed = progress.stage == Stage::Abandoned
                    || (repeatable && progress.stage == Stage::Rewarded);
                if !allowed || progress.cycle == u32::MAX {
                    return None;
                }
                let cycle = progress.cycle + 1;
                *progress = Progress {
                    cycle,
                    ..Progress::new()
                };
                Some(next)
            }
        }
    }

    /// Move one attempt between two stages: `active` to `completed`, or `completed` to `rewarded`.
    ///
    /// The request must name the attempt's own cycle and current stage. `None` refuses every other
    /// transition, a stale cycle, and a missing record.
    pub fn advance(&self, id: u32, cycle: u32, from: Stage, to: Stage) -> Option<Profile> {
        if !((from == Stage::Active && to == Stage::Completed)
            || (from == Stage::Completed && to == Stage::Rewarded))
        {
            return None;
        }
        let progress = self.records.get(&id)?;
        if progress.cycle != cycle || progress.stage != from {
            return None;
        }
        let mut next = self.clone();
        next.records.get_mut(&id)?.stage = to;
        Some(next)
    }

    /// Abandon an attempt that is `active` or `completed`.
    ///
    /// The cycle is kept, so a completion or a reward that arrives after the abandon names a stage that
    /// is no longer current and is refused. A rewarded attempt cannot be abandoned.
    pub fn abandon(&self, id: u32, cycle: u32) -> Option<Profile> {
        let progress = self.records.get(&id)?;
        if progress.cycle != cycle
            || (progress.stage != Stage::Active && progress.stage != Stage::Completed)
        {
            return None;
        }
        let mut next = self.clone();
        next.records.get_mut(&id)?.stage = Stage::Abandoned;
        Some(next)
    }

    /// Credit one objective of an `active` attempt.
    ///
    /// `required` is the number of distinct credits that completes the quest; the last one moves the
    /// attempt to `completed`. A duplicate credit, a credit of 0, an attempt that is not active, a stale
    /// cycle and a count over the limit are all refused.
    pub fn credit(
        &self,
        id: u32,
        cycle: u32,
        credit: u64,
        required: usize,
        limits: &Limits,
    ) -> Option<Profile> {
        if credit == 0 || required == 0 || required > limits.max_credits {
            return None;
        }
        let progress = self.records.get(&id)?;
        if progress.cycle != cycle
            || progress.stage != Stage::Active
            || progress.credits.contains(&credit)
            || progress.credits.len() >= required
        {
            return None;
        }
        let mut next = self.clone();
        let progress = next.records.get_mut(&id)?;
        progress.credits.insert(credit);
        if progress.credits.len() == required {
            progress.stage = Stage::Completed;
        }
        Some(next)
    }
}

/// The floating marker a quest puts above its giver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Marker {
    /// No marker.
    None,
    /// The quest can be accepted.
    Offer,
    /// The quest can be turned in.
    TurnIn,
}

/// The marker one quest carries for this profile. A turn-in wins over an offer, the order the live
/// capture shows when one NPC has both.
pub fn marker(profile: &Profile, id: u32) -> Marker {
    if reward_open(profile, id) {
        Marker::TurnIn
    } else if offer_open(profile, id) {
        Marker::Offer
    } else {
        Marker::None
    }
}

/// Whether an NPC's offer is open for this quest: no attempt yet, or an abandoned one.
///
/// A repeatable quest is not modelled; the caller that has one passes `repeatable` to
/// [`Profile::accept`] instead.
pub fn offer_open(profile: &Profile, id: u32) -> bool {
    match profile.records.get(&id) {
        None => true,
        Some(progress) => progress.stage == Stage::Abandoned,
    }
}

/// Whether a turn-in is open for this quest: an active or completed attempt.
pub fn reward_open(profile: &Profile, id: u32) -> bool {
    profile
        .records
        .get(&id)
        .is_some_and(|progress| matches!(progress.stage, Stage::Active | Stage::Completed))
}

/// The units that open and close a character-name argument in a captured encoded string.
pub const CHARACTER_ARGUMENT_OPEN: u16 = 0x0107;
/// See [`CHARACTER_ARGUMENT_OPEN`].
pub const CHARACTER_ARGUMENT_CLOSE: u16 = 0x0001;

/// Replace the character name a captured quest text carries with the playing character's name.
///
/// The capture was taken as one character, so a text that names the player carries that name between
/// `0x0107` and `0x0001` (`gw_quest_text_binding.h`). The replacement is explicit: the captured name is
/// matched as the whole run between those units, never as a substring of the encoded text. `None`
/// refuses a text that does not carry the argument exactly, a name over `name_limit` units, an empty
/// name, an embedded terminator, or a result over `limit` units.
pub fn bind_character_argument(
    text: &[u16],
    captured: &[u16],
    current: &[u16],
    name_limit: usize,
    limit: usize,
) -> Option<Vec<u16>> {
    if captured.is_empty() || current.is_empty() || current.len() > name_limit {
        return None;
    }
    if captured.contains(&0) || current.contains(&0) {
        return None;
    }
    let mut pattern = Vec::with_capacity(captured.len() + 2);
    pattern.push(CHARACTER_ARGUMENT_OPEN);
    pattern.extend_from_slice(captured);
    pattern.push(CHARACTER_ARGUMENT_CLOSE);
    let mut replacement = Vec::with_capacity(current.len() + 2);
    replacement.push(CHARACTER_ARGUMENT_OPEN);
    replacement.extend_from_slice(current);
    replacement.push(CHARACTER_ARGUMENT_CLOSE);
    let mut out = Vec::with_capacity(text.len());
    let mut at = 0;
    let mut replaced = 0;
    while at < text.len() {
        if text[at..].starts_with(&pattern) {
            out.extend_from_slice(&replacement);
            at += pattern.len();
            replaced += 1;
        } else {
            out.push(text[at]);
            at += 1;
        }
    }
    if replaced == 0 || out.len() > limit || out.contains(&0) {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits {
            max_records: 4,
            max_credits: 3,
        }
    }

    fn active() -> Profile {
        Profile::default()
            .accept(80, false, &limits())
            .expect("a new quest")
    }

    #[test]
    fn a_new_quest_starts_at_cycle_one_and_active() {
        let profile = active();
        let progress = profile.records.get(&80).expect("the record");
        assert_eq!(progress.cycle, 1);
        assert_eq!(progress.stage, Stage::Active);
        assert!(progress.credits.is_empty());
        assert_eq!(Profile::default().accept(0, false, &limits()), None);
    }

    #[test]
    fn only_an_abandoned_or_repeatable_quest_is_accepted_again() {
        let profile = active();
        assert_eq!(profile.accept(80, true, &limits()), None, "still active");
        let abandoned = profile.abandon(80, 1).expect("abandon");
        let again = abandoned.accept(80, false, &limits()).expect("reaccept");
        assert_eq!(again.records[&80].cycle, 2);
        assert_eq!(again.records[&80].stage, Stage::Active);
        let rewarded = profile
            .advance(80, 1, Stage::Active, Stage::Completed)
            .and_then(|p| p.advance(80, 1, Stage::Completed, Stage::Rewarded))
            .expect("rewarded");
        assert_eq!(
            rewarded.accept(80, false, &limits()),
            None,
            "not repeatable"
        );
        assert_eq!(
            rewarded
                .accept(80, true, &limits())
                .expect("repeatable")
                .records[&80]
                .cycle,
            2
        );
    }

    #[test]
    fn the_stage_path_is_active_completed_rewarded() {
        let profile = active();
        assert_eq!(profile.advance(80, 1, Stage::Active, Stage::Rewarded), None);
        assert_eq!(
            profile.advance(80, 2, Stage::Active, Stage::Completed),
            None,
            "stale cycle"
        );
        let completed = profile
            .advance(80, 1, Stage::Active, Stage::Completed)
            .expect("completed");
        assert_eq!(
            completed.advance(80, 1, Stage::Active, Stage::Completed),
            None
        );
        let rewarded = completed
            .advance(80, 1, Stage::Completed, Stage::Rewarded)
            .expect("rewarded");
        assert_eq!(rewarded.records[&80].stage, Stage::Rewarded);
    }

    #[test]
    fn abandon_keeps_the_cycle_and_refuses_a_rewarded_attempt() {
        let profile = active();
        let abandoned = profile.abandon(80, 1).expect("abandon");
        assert_eq!(abandoned.records[&80].cycle, 1);
        assert_eq!(abandoned.records[&80].stage, Stage::Abandoned);
        assert_eq!(abandoned.abandon(80, 1), None);
        let rewarded = profile
            .advance(80, 1, Stage::Active, Stage::Completed)
            .and_then(|p| p.advance(80, 1, Stage::Completed, Stage::Rewarded))
            .expect("rewarded");
        assert_eq!(rewarded.abandon(80, 1), None);
    }

    #[test]
    fn the_last_credit_completes_the_attempt() {
        let profile = active();
        let one = profile.credit(80, 1, 11, 3, &limits()).expect("one");
        assert_eq!(one.records[&80].stage, Stage::Active);
        assert_eq!(one.credit(80, 1, 11, 3, &limits()), None, "duplicate");
        let two = one.credit(80, 1, 12, 3, &limits()).expect("two");
        assert_eq!(two.records[&80].stage, Stage::Active);
        let three = two.credit(80, 1, 13, 3, &limits()).expect("three");
        assert_eq!(three.records[&80].stage, Stage::Completed);
        assert_eq!(three.credit(80, 1, 14, 3, &limits()), None, "complete");
        assert_eq!(profile.credit(80, 1, 0, 3, &limits()), None, "credit 0");
        assert_eq!(
            profile.credit(80, 1, 11, 9, &limits()),
            None,
            "over the limit"
        );
    }

    #[test]
    fn a_stored_profile_is_checked_against_the_limits() {
        let profile = active();
        assert!(profile.validate(&limits()).is_ok());
        let tight = Limits {
            max_records: 0,
            max_credits: 0,
        };
        assert!(profile.validate(&tight).is_err());
        let mut broken = profile.clone();
        broken.records.get_mut(&80).unwrap().cycle = 0;
        assert!(broken.validate(&limits()).is_err());
    }

    #[test]
    fn an_offer_and_a_turn_in_open_at_the_stages_that_own_them() {
        let empty = Profile::default();
        assert!(offer_open(&empty, 80));
        assert!(!reward_open(&empty, 80));
        assert_eq!(marker(&empty, 80), Marker::Offer);
        let active = empty.accept(80, false, &limits()).expect("accepted");
        assert!(!offer_open(&active, 80));
        assert!(reward_open(&active, 80));
        assert_eq!(marker(&active, 80), Marker::TurnIn);
        let abandoned = active.abandon(80, 1).expect("abandoned");
        assert!(offer_open(&abandoned, 80));
        assert!(!reward_open(&abandoned, 80));
        assert_eq!(marker(&abandoned, 80), Marker::Offer);
        let rewarded = active
            .advance(80, 1, Stage::Active, Stage::Completed)
            .and_then(|profile| profile.advance(80, 1, Stage::Completed, Stage::Rewarded))
            .expect("rewarded");
        assert_eq!(marker(&rewarded, 80), Marker::None);
    }

    #[test]
    fn a_captured_name_argument_is_replaced_where_it_stands() {
        let mut text: Vec<u16> = "Hello ".encode_utf16().collect();
        text.extend([CHARACTER_ARGUMENT_OPEN]);
        text.extend("Logger".encode_utf16());
        text.extend([CHARACTER_ARGUMENT_CLOSE]);
        text.extend(" and go.".encode_utf16());
        let bound = bind_character_argument(
            &text,
            &"Logger".encode_utf16().collect::<Vec<_>>(),
            &"Ari".encode_utf16().collect::<Vec<_>>(),
            19,
            127,
        )
        .expect("bound");
        let rendered: String = String::from_utf16_lossy(&bound);
        assert_eq!(rendered, "Hello \u{107}Ari\u{1} and go.");
        assert_eq!(
            bind_character_argument(
                &text,
                &"Someone".encode_utf16().collect::<Vec<_>>(),
                &"Ari".encode_utf16().collect::<Vec<_>>(),
                19,
                127
            ),
            None,
            "the captured name is not in the text"
        );
        assert_eq!(
            bind_character_argument(
                &text,
                &"Logger".encode_utf16().collect::<Vec<_>>(),
                &"A".repeat(20).encode_utf16().collect::<Vec<_>>(),
                19,
                127
            ),
            None,
            "the current name is over the client's 19 units"
        );
    }
}
