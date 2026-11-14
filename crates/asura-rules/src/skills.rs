//! Skills and attributes: whether a bar may hold what a client asks for, and whether a rank may change.
//!
//! Pure decisions over values the caller read: the learned set and the bar from the character's row, a
//! skill's profession and flags from the world pack's `skills.json`, the attribute table from its
//! `attributes.json`. Nothing here holds a number of its own (`docs/38-layer-decisions.md`, D34, D35).
//!
//! The rules are preforged's validators (`gw_skillbar.h` `validate`, `gw_attributes.h` `validate` and
//! `propose`), restated: a learned skill, one copy per slot, the bar's professions, one elite, three
//! PvE-only skills, and attributes within their professions, their rank cap and the earned points.

/// One slot: a skill id and which of its copies (0 for the only one).
pub type Slot = (u32, u32);

/// What a bar check needs to know about one skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillFacts {
    /// 0 for a common skill.
    pub profession: u8,
    /// An elite skill: one per bar.
    pub elite: bool,
    /// A PvE-only skill: three per bar.
    pub pve_only: bool,
}

/// Why a bar edit was refused. The caller answers with the bar as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarRefusal {
    /// Skills change only in a town or an outpost.
    NotHere,
    /// A slot index past the eighth.
    Slot,
    /// The skill is not in this world's catalogue.
    UnknownSkill,
    /// The character has not learned it.
    NotLearned,
    /// A copy other than 0, or the same skill twice.
    Copy,
    /// Neither the primary nor the secondary profession's.
    Profession,
    /// A second elite.
    MoreThanOneElite,
    /// A fourth PvE-only skill.
    MoreThanThreePveOnly,
    /// A swap or move named a skill that is not on the bar.
    Missing,
    /// A move onto a slot another skill holds.
    Occupied,
}

/// A client's request to change its bar, read from `GAME_CMSG_SKILLBAR_*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarEdit {
    /// `0x805C`: put a skill (or nothing) in a slot.
    Set {
        /// The slot, 0..7.
        slot: u32,
        /// The skill and copy; skill 0 empties the slot.
        skill: Slot,
    },
    /// `0x805D`: replace the whole bar with these ids; a repeated id is its next copy.
    Load {
        /// Up to eight skill ids, slot 0 first.
        ids: Vec<u32>,
    },
    /// `0x805E`: exchange two skills on the bar.
    Swap {
        /// One skill on the bar.
        first: Slot,
        /// The other.
        second: Slot,
    },
    /// `0x805F`: move a skill on the bar to a slot.
    Move {
        /// The skill on the bar.
        skill: Slot,
        /// Where it goes.
        slot: u32,
    },
}

/// The bar a well-formed edit proposes, before it is checked.
pub fn propose(bar: &[Slot; 8], edit: &BarEdit) -> Result<[Slot; 8], BarRefusal> {
    let mut proposed = *bar;
    let find = |bar: &[Slot; 8], wanted: Slot| bar.iter().position(|slot| *slot == wanted);
    match edit {
        BarEdit::Set { slot, skill } => {
            let index = usize::try_from(*slot)
                .ok()
                .filter(|index| *index < 8)
                .ok_or(BarRefusal::Slot)?;
            // A skill already on the bar moves rather than appearing twice, as the client draws it.
            if skill.0 != 0 {
                if let Some(old) = find(&proposed, *skill) {
                    proposed[old] = (0, 0);
                }
            }
            proposed[index] = if skill.0 == 0 { (0, 0) } else { *skill };
        }
        BarEdit::Load { ids } => {
            if ids.len() > 8 {
                return Err(BarRefusal::Slot);
            }
            proposed = [(0, 0); 8];
            for (index, id) in ids.iter().enumerate() {
                let copy = if *id == 0 {
                    0
                } else {
                    ids[..index].iter().filter(|earlier| *earlier == id).count() as u32
                };
                proposed[index] = if *id == 0 { (0, 0) } else { (*id, copy) };
            }
        }
        BarEdit::Swap { first, second } => {
            if first.0 == 0 || second.0 == 0 {
                return Err(BarRefusal::Missing);
            }
            let a = find(&proposed, *first).ok_or(BarRefusal::Missing)?;
            let b = find(&proposed, *second).ok_or(BarRefusal::Missing)?;
            proposed.swap(a, b);
        }
        BarEdit::Move { skill, slot } => {
            let index = usize::try_from(*slot)
                .ok()
                .filter(|index| *index < 8)
                .ok_or(BarRefusal::Slot)?;
            if skill.0 == 0 {
                return Err(BarRefusal::Missing);
            }
            let from = find(&proposed, *skill).ok_or(BarRefusal::Missing)?;
            if proposed[index].0 != 0 && proposed[index] != *skill {
                return Err(BarRefusal::Occupied);
            }
            proposed[from] = (0, 0);
            proposed[index] = *skill;
        }
    }
    Ok(proposed)
}

/// Whether a bar may stand: every skill known to the catalogue (`facts`), learned, once, of the bar's
/// professions, with at most one elite and three PvE-only skills.
pub fn validate(
    bar: &[Slot; 8],
    primary: u8,
    secondary: u8,
    learned: &dyn Fn(u32) -> bool,
    facts: &dyn Fn(u32) -> Option<SkillFacts>,
) -> Result<(), BarRefusal> {
    let mut elites = 0;
    let mut pve = 0;
    for (index, slot) in bar.iter().enumerate() {
        if slot.0 == 0 {
            if slot.1 != 0 {
                return Err(BarRefusal::Copy);
            }
            continue;
        }
        let skill = facts(slot.0).ok_or(BarRefusal::UnknownSkill)?;
        if !learned(slot.0) {
            return Err(BarRefusal::NotLearned);
        }
        // This world keeps one copy of each learned skill (Signet of Capture is not in pre-Searing).
        if slot.1 != 0 || bar[..index].iter().any(|earlier| earlier.0 == slot.0) {
            return Err(BarRefusal::Copy);
        }
        if skill.profession != 0 && skill.profession != primary && skill.profession != secondary {
            return Err(BarRefusal::Profession);
        }
        if skill.elite {
            elites += 1;
            if elites > 1 {
                return Err(BarRefusal::MoreThanOneElite);
            }
        }
        if skill.pve_only {
            pve += 1;
            if pve > 3 {
                return Err(BarRefusal::MoreThanThreePveOnly);
            }
        }
    }
    Ok(())
}

/// The learned set as the client reads it in `GAME_SMSG_UPDATE_UNLOCKED_SKILLS`: skill `n` is bit
/// `n % 32` of word `n / 32`, and the list is only as long as its last set word.
pub fn learned_bitmap(learned: impl IntoIterator<Item = u32>) -> Vec<u32> {
    let mut words: Vec<u32> = Vec::new();
    for id in learned {
        if id == 0 {
            continue;
        }
        let word = (id / 32) as usize;
        if words.len() <= word {
            words.resize(word + 1, 0);
        }
        words[word] |= 1 << (id % 32);
    }
    words
}

// ---- attributes ----------------------------------------------------------------------------

/// `attributes.json`, as the caller read it.
#[derive(Debug, Clone, Copy)]
pub struct AttributeTable<'a> {
    /// Per attribute index: its profession and whether only a primary may raise it.
    pub professions: &'a [(u8, bool)],
    /// Total cost of each rank 0..=max_rank.
    pub rank_cost: &'a [u32],
    /// Points earned at levels 1..=20.
    pub level_points: &'a [u32],
    /// The highest rank points can buy.
    pub max_rank: u8,
    /// The most attributes an allocation may name.
    pub max_attributes: usize,
}

impl AttributeTable<'_> {
    /// Whether a character with these professions may hold ranks in an attribute.
    pub fn available(&self, attribute: usize, primary: u8, secondary: u8) -> bool {
        match self.professions.get(attribute) {
            Some((profession, primary_only)) => {
                *profession != 0
                    && *profession <= 10
                    && (*profession == primary || (*profession == secondary && !primary_only))
            }
            None => false,
        }
    }

    /// The attributes a character with these professions has, in index order: what the attribute
    /// window lists and what `GAME_SMSG_AGENT_UPDATE_ATTRIBUTES` carries.
    pub fn listed(&self, primary: u8, secondary: u8) -> Vec<usize> {
        (0..self.professions.len())
            .filter(|attribute| self.available(*attribute, primary, secondary))
            .collect()
    }

    /// Points a level has earned. A level outside 1..=20 is clamped into it.
    pub fn points(&self, level: u32) -> u32 {
        let index = level.clamp(1, 20) as usize - 1;
        self.level_points.get(index).copied().unwrap_or(0)
    }

    /// Points the ranks spend.
    pub fn spent(&self, ranks: &[u8]) -> u32 {
        ranks
            .iter()
            .map(|rank| {
                self.rank_cost
                    .get(usize::from(*rank))
                    .copied()
                    .unwrap_or(u32::MAX / 64)
            })
            .sum()
    }
}

/// Why an attribute change was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeRefusal {
    /// Not one of the character's professions' attributes.
    Attribute,
    /// Below 0 or above the cap.
    Rank,
    /// The same attribute twice in a load.
    Duplicate,
    /// More than the earned points.
    Points,
    /// Refunds happen only in a town or an outpost.
    NotHere,
}

/// A client's request to change its ranks (`GAME_CMSG_ATTRIBUTE_*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeEdit {
    /// One rank more in this attribute.
    Increase(u32),
    /// One rank less in this attribute.
    Decrease(u32),
    /// The whole allocation: (attribute, rank) pairs; every other attribute goes to 0.
    Load(Vec<(u32, u32)>),
}

/// Whether a set of ranks may stand.
pub fn validate_ranks(
    ranks: &[u8],
    primary: u8,
    secondary: u8,
    level: u32,
    table: &AttributeTable<'_>,
) -> Result<(), AttributeRefusal> {
    for (attribute, rank) in ranks.iter().enumerate() {
        if *rank > table.max_rank {
            return Err(AttributeRefusal::Rank);
        }
        if *rank > 0 && !table.available(attribute, primary, secondary) {
            return Err(AttributeRefusal::Attribute);
        }
    }
    if table.spent(ranks) > table.points(level) {
        return Err(AttributeRefusal::Points);
    }
    Ok(())
}

/// The ranks an edit proposes, checked. `may_refund` is whether ranks may go down here.
#[allow(clippy::too_many_arguments)]
pub fn apply_attribute_edit(
    ranks: &[u8],
    edit: &AttributeEdit,
    primary: u8,
    secondary: u8,
    level: u32,
    may_refund: bool,
    table: &AttributeTable<'_>,
) -> Result<Vec<u8>, AttributeRefusal> {
    let mut proposed = ranks.to_vec();
    let length = proposed.len();
    let index = |attribute: u32| -> Result<usize, AttributeRefusal> {
        let index = attribute as usize;
        if index < length && table.available(index, primary, secondary) {
            Ok(index)
        } else {
            Err(AttributeRefusal::Attribute)
        }
    };
    match edit {
        AttributeEdit::Increase(attribute) => {
            let at = index(*attribute)?;
            if proposed[at] >= table.max_rank {
                return Err(AttributeRefusal::Rank);
            }
            proposed[at] += 1;
        }
        AttributeEdit::Decrease(attribute) => {
            let at = index(*attribute)?;
            if proposed[at] == 0 {
                return Err(AttributeRefusal::Rank);
            }
            proposed[at] -= 1;
        }
        AttributeEdit::Load(pairs) => {
            if pairs.len() > table.max_attributes {
                return Err(AttributeRefusal::Attribute);
            }
            proposed.iter_mut().for_each(|rank| *rank = 0);
            let mut seen = Vec::new();
            for (attribute, rank) in pairs {
                let at = index(*attribute)?;
                if seen.contains(&at) {
                    return Err(AttributeRefusal::Duplicate);
                }
                seen.push(at);
                proposed[at] = u8::try_from(*rank)
                    .ok()
                    .filter(|rank| *rank <= table.max_rank)
                    .ok_or(AttributeRefusal::Rank)?;
            }
        }
    }
    if !may_refund && proposed.iter().zip(ranks).any(|(new, old)| new < old) {
        return Err(AttributeRefusal::NotHere);
    }
    validate_ranks(&proposed, primary, secondary, level, table)?;
    Ok(proposed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(id: u32) -> Option<SkillFacts> {
        match id {
            1 => Some(SkillFacts {
                profession: 1,
                elite: false,
                pve_only: false,
            }),
            2 => Some(SkillFacts {
                profession: 0,
                elite: false,
                pve_only: false,
            }),
            281 => Some(SkillFacts {
                profession: 3,
                elite: false,
                pve_only: false,
            }),
            900 => Some(SkillFacts {
                profession: 1,
                elite: true,
                pve_only: false,
            }),
            901 => Some(SkillFacts {
                profession: 1,
                elite: true,
                pve_only: false,
            }),
            _ => None,
        }
    }

    fn knows(id: u32) -> bool {
        matches!(id, 1 | 2 | 281 | 900 | 901)
    }

    #[test]
    fn a_bar_holds_learned_skills_of_its_professions_once_each() {
        let mut bar = [(0, 0); 8];
        bar[0] = (1, 0);
        bar[1] = (2, 0);
        assert_eq!(validate(&bar, 1, 0, &knows, &facts), Ok(()));
        bar[2] = (281, 0);
        assert_eq!(
            validate(&bar, 1, 0, &knows, &facts),
            Err(BarRefusal::Profession)
        );
        assert_eq!(
            validate(&bar, 1, 3, &knows, &facts),
            Ok(()),
            "a Monk secondary"
        );
        bar[3] = (1, 0);
        assert_eq!(validate(&bar, 1, 3, &knows, &facts), Err(BarRefusal::Copy));
        bar[3] = (0, 0);
        bar[4] = (900, 0);
        bar[5] = (901, 0);
        assert_eq!(
            validate(&bar, 1, 3, &knows, &facts),
            Err(BarRefusal::MoreThanOneElite)
        );
        bar[5] = (5, 0);
        assert_eq!(
            validate(&bar, 1, 3, &knows, &facts),
            Err(BarRefusal::UnknownSkill)
        );
    }

    #[test]
    fn the_four_edits_propose_what_the_client_drew() {
        let mut bar = [(0, 0); 8];
        bar[0] = (1, 0);
        bar[1] = (2, 0);
        let set = propose(
            &bar,
            &BarEdit::Set {
                slot: 5,
                skill: (1, 0),
            },
        )
        .unwrap();
        assert_eq!(
            (set[0], set[5]),
            ((0, 0), (1, 0)),
            "a skill set elsewhere moves"
        );
        let swap = propose(
            &bar,
            &BarEdit::Swap {
                first: (1, 0),
                second: (2, 0),
            },
        )
        .unwrap();
        assert_eq!((swap[0], swap[1]), ((2, 0), (1, 0)));
        assert_eq!(
            propose(
                &bar,
                &BarEdit::Move {
                    skill: (1, 0),
                    slot: 1
                }
            ),
            Err(BarRefusal::Occupied)
        );
        let moved = propose(
            &bar,
            &BarEdit::Move {
                skill: (1, 0),
                slot: 7,
            },
        )
        .unwrap();
        assert_eq!((moved[0], moved[7]), ((0, 0), (1, 0)));
        let load = propose(
            &bar,
            &BarEdit::Load {
                ids: vec![281, 0, 281],
            },
        )
        .unwrap();
        assert_eq!(
            &load[..3],
            &[(281, 0), (0, 0), (281, 1)],
            "a repeated id is its next copy"
        );
        assert_eq!(
            propose(
                &bar,
                &BarEdit::Set {
                    slot: 8,
                    skill: (1, 0)
                }
            ),
            Err(BarRefusal::Slot)
        );
    }

    #[test]
    fn the_learned_bitmap_is_addressed_like_the_unlock_list() {
        assert_eq!(learned_bitmap([1, 2, 33]), vec![0b110, 0b10]);
        assert!(learned_bitmap([]).is_empty());
    }

    const PROFESSIONS: &[(u8, bool)] = &[
        (5, true),
        (5, false),
        (5, false),
        (5, false),
        (4, false),
        (4, false),
        (4, true),
        (4, false),
        (6, false),
        (6, false),
        (6, false),
        (6, false),
        (6, true),
        (3, false),
        (3, false),
        (3, false),
        (3, true),
        (1, true),
        (1, false),
        (1, false),
        (1, false),
        (1, false),
        (2, false),
        (2, true),
        (2, false),
        (2, false),
    ];
    const COST: &[u32] = &[0, 1, 3, 6, 10, 15, 21, 28, 37, 48, 61, 77, 97];
    const LEVELS: &[u32] = &[
        0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 55, 65, 75, 85, 95, 110, 125, 140, 155, 170,
    ];

    fn table() -> AttributeTable<'static> {
        AttributeTable {
            professions: PROFESSIONS,
            rank_cost: COST,
            level_points: LEVELS,
            max_rank: 12,
            max_attributes: 16,
        }
    }

    #[test]
    fn attributes_follow_the_professions_the_cap_and_the_points() {
        let table = table();
        let ranks = vec![0u8; 26];
        assert_eq!(
            table.listed(3, 1),
            vec![13, 14, 15, 16, 18, 19, 20, 21],
            "Monk/Warrior: no Strength"
        );
        assert_eq!(
            apply_attribute_edit(&ranks, &AttributeEdit::Increase(13), 3, 0, 1, true, &table),
            Err(AttributeRefusal::Points),
            "level 1 has no points"
        );
        let raised =
            apply_attribute_edit(&ranks, &AttributeEdit::Increase(13), 3, 0, 2, true, &table)
                .unwrap();
        assert_eq!(raised[13], 1);
        assert_eq!(
            apply_attribute_edit(
                &raised,
                &AttributeEdit::Decrease(13),
                3,
                0,
                2,
                false,
                &table
            ),
            Err(AttributeRefusal::NotHere)
        );
        assert_eq!(
            apply_attribute_edit(&ranks, &AttributeEdit::Increase(17), 3, 1, 20, true, &table),
            Err(AttributeRefusal::Attribute),
            "Strength is a primary Warrior's"
        );
        let loaded = apply_attribute_edit(
            &ranks,
            &AttributeEdit::Load(vec![(13, 12), (16, 9)]),
            3,
            0,
            20,
            true,
            &table,
        )
        .unwrap();
        assert_eq!((loaded[13], loaded[16]), (12, 9));
        assert_eq!(table.spent(&loaded), 97 + 48);
        assert_eq!(
            apply_attribute_edit(
                &ranks,
                &AttributeEdit::Load(vec![(13, 12), (16, 12)]),
                3,
                0,
                20,
                true,
                &table
            ),
            Err(AttributeRefusal::Points)
        );
    }
}
