//! Titles: which rank a track is on, which records a load sends, and whether a title may be shown.
//!
//! Every function here takes the title definitions as an argument - they are configuration
//! (`config/Server/titles.json`), read by `asura-config` - and the points as the caller read them from the
//! database. Nothing here knows a title by name. The wire shapes are `asura_protocol::game::title`; the
//! evidence for every rule below is `docs/40-titles-system-research.md`.
//!
//! The model, in the client's terms:
//!
//! * A **rank record** (`0x00F3`) is one rung: a global id, a flag word, a value (the rank number, or a
//!   per-mille threshold on a percentage track) and a coded name. The client keeps them in one table keyed
//!   by id, so an id names one record everywhere.
//! * A **track** (`0x00F6`) is one title's progress: its points, the rung it is on and the rung after it,
//!   each with the points that reach it. A maxed track points at a sentinel record whose value is
//!   `0xFFFFFFFF`, with `0xFFFFFFFF` as the next minimum.
//! * A **display binding** (`0x00F4`) is the rung a player shows under its name.

use asura_protocol::game::title::{
    TitleTrackInfo, TITLE_ID_COUNT, TITLE_NO_NEXT, TITLE_STRING_UNITS,
};

/// Whose progress a title is: the character's own, or shared by every character on the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Progress belongs to one character (Cartographer, Drunkard, LDoA, ...).
    Character,
    /// Progress is shared across the account (Lucky, Treasure Hunter, Wisdom, ...).
    Account,
}

/// What earns a title points without anyone calling `/title`. The world measures the input; this crate only
/// turns it into points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Earning {
    /// The share of pre-Searing's world-map rectangles the character has explored, in per-mille
    /// ([`per_mille`]). An owner's rule for this server (2026-11-05): retail counts no pre-Searing area
    /// toward any Cartographer title.
    PreSearingExploration,
}

/// `part` of `whole` in per-mille, rounded down and capped at 1000 - the unit a percentage track's
/// points are in (Canthan Cartographer at 1 point is 0.1%, `next_minimum` 600 is 60%). `whole` 0 is 0.
pub fn per_mille(part: u64, whole: u64) -> u32 {
    if whole == 0 {
        return 0;
    }
    (part.min(whole) * 1000 / whole) as u32
}

/// One rung of a title, as `0x00F3` carries it, and the points that reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rank {
    /// The global key into the client's rank table.
    pub rank_id: u32,
    /// `0x00F3` field 2: bit 0 set means `value` is a per-mille threshold.
    pub flags: u32,
    /// `0x00F3` field 3: the rank number shown in parentheses, or the per-mille threshold.
    pub value: u32,
    /// The points at which a track reaches this rung: `0x00F6` field 5 when it is current, field 8 when it
    /// is next. Not on the wire with the record itself.
    pub minimum: u32,
    /// The coded name the Hero panel draws: at most seven units.
    pub name: Vec<u16>,
}

/// One title: everything a track message needs that does not change with play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Title {
    /// The client's title id, `0..48`.
    pub title_id: u32,
    /// What a person calls it - the file's `name` and `aliases` - for a command to find it by. The client
    /// never sees these: it draws a title's name from its own table.
    pub names: Vec<String>,
    /// Whose progress it is.
    pub scope: Scope,
    /// `0x00F6` field 2, carried as configured.
    pub flags: u32,
    /// `0x00F6` field 6, `title_mode_flags`.
    pub mode_flags: u32,
    /// `0x00F6` field 10.
    pub base_rank_id: u32,
    /// `0x00F6` field 11, a coded string.
    pub progress_template: Vec<u16>,
    /// `0x00F6` field 12, a coded string.
    pub description_template: Vec<u16>,
    /// Every rung from the unranked tier (rank number 0, minimum 0) to the last, ascending. The highest
    /// rank number - `0x00F6` field 9 - is `ranks.len() - 1`.
    pub ranks: Vec<Rank>,
    /// The record a maxed track names as its next rung. `None` on a title whose last rung is never
    /// reached in this world; a track that reaches it anyway is refused rather than sent without one.
    pub maxed: Option<Rank>,
    /// The first rank number a player may show under its name. `None` means not established, and every
    /// request to show this title is refused with that reason.
    pub display_from_rank: Option<u32>,
    /// What earns it points on its own, if anything. `None`: only `/title` and later earning rules.
    pub earned_by: Option<Earning>,
}

/// Where a track stands for a given point count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing<'a> {
    /// The rank number the track is on: an index into [`Title::ranks`].
    pub rank_number: u32,
    /// The rung it is on.
    pub current: &'a Rank,
    /// The rung after it, or the title's sentinel on a maxed track.
    pub next: &'a Rank,
    /// The points the next rung needs, or `0xFFFFFFFF` on a maxed track.
    pub next_minimum: u32,
}

/// The title with this id, if one is defined.
pub fn find(titles: &[Title], title_id: u32) -> Option<&Title> {
    titles.iter().find(|title| title.title_id == title_id)
}

/// The rung a track with `points` is on, and the one after it.
///
/// `None` when the title has no rungs, or is maxed and has no sentinel to name as its next rung - a track
/// that cannot be sent without naming a record that does not exist.
pub fn standing(title: &Title, points: u32) -> Option<Standing<'_>> {
    let index = title
        .ranks
        .iter()
        .rposition(|rank| rank.minimum <= points)?;
    let current = &title.ranks[index];
    let (next, next_minimum) = match title.ranks.get(index + 1) {
        Some(next) => (next, next.minimum),
        None => (title.maxed.as_ref()?, TITLE_NO_NEXT),
    };
    Some(Standing {
        rank_number: index as u32,
        current,
        next,
        next_minimum,
    })
}

/// What a load sends for one track: the two rank records it names, in the order they go out, and the track
/// itself. Retail's shape (build 39112, doc 40 section 5.4): this title's current and next records, then its
/// track, title by title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackGroup<'a> {
    /// The records to seed before the track, current first.
    pub ranks: [&'a Rank; 2],
    /// The track message's fields.
    pub track: TitleTrackInfo<'a>,
}

/// The group a load sends for `title` at `points`.
///
/// `None` for a track a load does not send: one with no points (retail sends only tracks with progress,
/// doc 40 section 5.4), or one [`standing`] cannot place.
pub fn track_group(title: &Title, points: u32) -> Option<TrackGroup<'_>> {
    if points == 0 {
        return None;
    }
    let at = standing(title, points)?;
    Some(TrackGroup {
        ranks: [at.current, at.next],
        track: TitleTrackInfo {
            title_id: title.title_id,
            flags: title.flags,
            points,
            current_rank_id: at.current.rank_id,
            current_minimum: at.current.minimum,
            mode_flags: title.mode_flags,
            next_rank_id: at.next.rank_id,
            next_minimum: at.next_minimum,
            max_rank: title.ranks.len().saturating_sub(1) as u32,
            base_rank_id: title.base_rank_id,
            progress_template: &title.progress_template,
            description_template: &title.description_template,
        },
    })
}

/// What a change of points sends, for a track whose points went from `before` to `after`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointChange {
    /// Nothing: the points did not change, or the track has none and is not shown.
    Nothing,
    /// `0x00F5` with the new points: the track is on the client and is still on the same rung.
    Patch,
    /// The whole group again (`0x00F3` x2, `0x00F6`): the track is new to this connection, or it changed
    /// rung and the client must be told the new current and next records.
    Resend,
}

/// Decide how a point change reaches the client. `sent` is whether this connection was already sent the
/// track's `0x00F6`; a patch to a track the client does not have is inert (doc 40 section 6 rule 3).
pub fn point_change(title: &Title, before: u32, after: u32, sent: bool) -> PointChange {
    if before == after || after == 0 {
        return PointChange::Nothing;
    }
    if !sent {
        return PointChange::Resend;
    }
    let rung = |points| standing(title, points).map(|at| at.rank_number);
    if rung(before) == rung(after) {
        PointChange::Patch
    } else {
        PointChange::Resend
    }
}

/// Why a request to show a title was refused. The caller logs it; the rule does not print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayRefused {
    /// The client named a title id this server has no definition for.
    Unknown,
    /// The definition does not say from which rank the title may be shown.
    NotEstablished,
    /// The track is below the rank it may be shown from.
    BelowDisplayRank {
        /// The rank number the track is on.
        rank: u32,
        /// The rank number it may be shown from.
        from: u32,
    },
}

/// The rung a player shows when it asks to show `title_id` with `points` on that track.
pub fn display_rank(titles: &[Title], title_id: u32, points: u32) -> Result<&Rank, DisplayRefused> {
    let title = find(titles, title_id).ok_or(DisplayRefused::Unknown)?;
    let from = title
        .display_from_rank
        .ok_or(DisplayRefused::NotEstablished)?;
    let at = standing(title, points).ok_or(DisplayRefused::BelowDisplayRank { rank: 0, from })?;
    if at.rank_number < from.max(1) {
        return Err(DisplayRefused::BelowDisplayRank {
            rank: at.rank_number,
            from,
        });
    }
    Ok(at.current)
}

/// One thing wrong with a set of title definitions: the title it is in and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The title the problem was found in.
    pub title_id: u32,
    /// What is wrong, in a sentence.
    pub what: String,
}

/// Every reason a set of definitions would put a client at risk, or none.
///
/// The checks are the measured crash rules (doc 40 section 6) and the invariants the rest of this module
/// relies on: a title id the client has a row for, strings the field admits, a ladder that starts at 0 and
/// climbs, and one record per rank id across every title - the client keeps one table, so two different
/// records under one id would show whichever came last.
pub fn validate(titles: &[Title]) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut seen_titles = Vec::new();
    let mut seen_ranks: Vec<&Rank> = Vec::new();
    for title in titles {
        let mut problem = |what: String| {
            problems.push(Problem {
                title_id: title.title_id,
                what,
            })
        };
        if title.title_id >= TITLE_ID_COUNT {
            problem(format!(
                "title id {} is outside the client's 0..{TITLE_ID_COUNT}",
                title.title_id
            ));
        }
        if seen_titles.contains(&title.title_id) {
            problem("the title is defined twice".to_string());
        }
        seen_titles.push(title.title_id);
        for (field, units) in [
            ("progress_template", &title.progress_template),
            ("description_template", &title.description_template),
        ] {
            if units.len() > TITLE_STRING_UNITS {
                problem(format!(
                    "{field} has {} units; the field admits {TITLE_STRING_UNITS}",
                    units.len()
                ));
            }
        }
        match title.ranks.first() {
            None => problem("the title has no ranks".to_string()),
            Some(first) if first.minimum != 0 => problem(format!(
                "the first rank ({}) starts at {} points, not 0",
                first.rank_id, first.minimum
            )),
            Some(_) => {}
        }
        for pair in title.ranks.windows(2) {
            if pair[1].minimum <= pair[0].minimum {
                problem(format!(
                    "rank {} ({} points) does not climb above rank {} ({} points)",
                    pair[1].rank_id, pair[1].minimum, pair[0].rank_id, pair[0].minimum
                ));
            }
        }
        if let Some(from) = title.display_from_rank {
            if from as usize >= title.ranks.len() {
                problem(format!(
                    "display_from_rank {from} is past the last rank number {}",
                    title.ranks.len().saturating_sub(1)
                ));
            }
        }
        for rank in title.ranks.iter().chain(title.maxed.iter()) {
            if rank.name.len() > TITLE_STRING_UNITS {
                problem(format!(
                    "rank {} has a {}-unit name; the field admits {TITLE_STRING_UNITS}",
                    rank.rank_id,
                    rank.name.len()
                ));
            }
            match seen_ranks.iter().find(|seen| seen.rank_id == rank.rank_id) {
                Some(seen)
                    if seen.flags != rank.flags
                        || seen.value != rank.value
                        || seen.name != rank.name =>
                {
                    problem(format!(
                        "rank id {} is defined twice with different records",
                        rank.rank_id
                    ));
                }
                Some(_) => {}
                None => seen_ranks.push(rank),
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank(rank_id: u32, value: u32, minimum: u32) -> Rank {
        Rank {
            rank_id,
            flags: 0,
            value,
            minimum,
            name: vec![0x0101],
        }
    }

    /// Legendary Defender of Ascalon as build 39112 sends it (doc 40 section 5.1): one rank at 20, rank
    /// records 176 and 177, base 177, flags 2. The names are placeholders.
    fn ldoa() -> Title {
        Title {
            title_id: 21,
            names: vec![
                "Legendary Defender of Ascalon".to_string(),
                "ldoa".to_string(),
            ],
            scope: Scope::Character,
            flags: 2,
            mode_flags: 0,
            base_rank_id: 177,
            progress_template: vec![0x0101],
            description_template: vec![0x0101],
            ranks: vec![rank(176, 0, 0), rank(177, 1, 20)],
            maxed: Some(rank(178, TITLE_NO_NEXT, 0)),
            display_from_rank: Some(1),
            earned_by: None,
        }
    }

    #[test]
    fn a_track_with_points_reproduces_the_captured_ldoa_record() {
        let title = ldoa();
        let group = track_group(&title, 13).expect("13 points is a track");
        assert_eq!(group.ranks[0].rank_id, 176);
        assert_eq!(group.ranks[1].rank_id, 177);
        let track = group.track;
        assert_eq!(
            (
                track.title_id,
                track.flags,
                track.points,
                track.current_rank_id,
                track.current_minimum,
                track.mode_flags,
                track.next_rank_id,
                track.next_minimum,
                track.max_rank,
                track.base_rank_id
            ),
            (21, 2, 13, 176, 0, 0, 177, 20, 1, 177),
            "the ten dwords of seq 732 in the portal-battery capture"
        );
    }

    #[test]
    fn a_maxed_track_names_the_sentinel_and_no_next_minimum() {
        let title = ldoa();
        let at = standing(&title, 25).expect("placed");
        assert_eq!(at.rank_number, 1);
        assert_eq!(at.current.rank_id, 177);
        assert_eq!(at.next.rank_id, 178);
        assert_eq!(at.next_minimum, TITLE_NO_NEXT);

        let mut no_sentinel = ldoa();
        no_sentinel.maxed = None;
        assert_eq!(
            standing(&no_sentinel, 25),
            None,
            "refused, not sent naming nothing"
        );
        assert!(standing(&no_sentinel, 19).is_some());
    }

    #[test]
    fn exploration_is_per_mille_rounded_down_and_capped() {
        assert_eq!(per_mille(0, 4000), 0);
        assert_eq!(per_mille(4, 4000), 1);
        assert_eq!(per_mille(2399, 4000), 599);
        assert_eq!(per_mille(2400, 4000), 600);
        assert_eq!(per_mille(4000, 4000), 1000);
        assert_eq!(per_mille(5000, 4000), 1000);
        assert_eq!(per_mille(5, 0), 0);
    }

    #[test]
    fn a_track_with_no_points_is_not_sent() {
        assert_eq!(track_group(&ldoa(), 0), None);
    }

    #[test]
    fn a_point_change_patches_within_a_rung_and_resends_across_one() {
        let title = ldoa();
        assert_eq!(point_change(&title, 13, 14, true), PointChange::Patch);
        assert_eq!(point_change(&title, 19, 20, true), PointChange::Resend);
        assert_eq!(point_change(&title, 13, 14, false), PointChange::Resend);
        assert_eq!(point_change(&title, 13, 13, true), PointChange::Nothing);
        assert_eq!(point_change(&title, 0, 1, false), PointChange::Resend);
    }

    #[test]
    fn showing_a_title_needs_a_known_title_an_established_rule_and_the_rank() {
        let titles = vec![ldoa()];
        assert_eq!(display_rank(&titles, 21, 20).map(|r| r.rank_id), Ok(177));
        assert_eq!(
            display_rank(&titles, 21, 13),
            Err(DisplayRefused::BelowDisplayRank { rank: 0, from: 1 })
        );
        assert_eq!(display_rank(&titles, 7, 100), Err(DisplayRefused::Unknown));
        let mut unknown_rule = ldoa();
        unknown_rule.display_from_rank = None;
        assert_eq!(
            display_rank(&[unknown_rule], 21, 20),
            Err(DisplayRefused::NotEstablished)
        );
    }

    #[test]
    fn the_shipped_shape_validates_and_each_crash_rule_is_caught() {
        assert_eq!(validate(&[ldoa()]), vec![]);

        let mut out_of_range = ldoa();
        out_of_range.title_id = 48;
        assert_eq!(validate(&[out_of_range]).len(), 1);

        let mut long_name = ldoa();
        long_name.ranks[1].name = vec![0x0101; 8];
        assert_eq!(validate(&[long_name]).len(), 1);

        let mut flat = ldoa();
        flat.ranks[1].minimum = 0;
        assert_eq!(validate(&[flat]).len(), 1);

        let mut late_start = ldoa();
        late_start.ranks[0].minimum = 1;
        assert_eq!(validate(&[late_start]).len(), 1);

        let mut clash = ldoa();
        clash.title_id = 22;
        clash.ranks[1].value = 9;
        assert_eq!(
            validate(&[ldoa(), clash]),
            vec![Problem {
                title_id: 22,
                what: "rank id 177 is defined twice with different records".to_string()
            }]
        );

        assert_eq!(
            validate(&[ldoa(), ldoa()]).len(),
            1,
            "a title defined twice"
        );
    }
}
