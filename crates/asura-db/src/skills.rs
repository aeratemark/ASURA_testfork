//! A character's skills: learned ids, its bar and its attribute ranks (`t_character_skills`,
//! migration 0034).
//!
//! Ids and ranks only; what a skill is lives in the world pack (`docs/52-skills.md`). Every call is one
//! statement: the row is read when a connection is built and written when the player changes it.

use crate::{PgPool, Uuid};

/// One character's row, as plain numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    /// Learned skill ids.
    pub learned: Vec<u32>,
    /// The eight (skill, copy) slots, slot 0 first.
    pub bar: [(u32, u32); 8],
    /// One rank per attribute index; may be shorter than the attribute count.
    pub ranks: Vec<u8>,
}

fn unsigned(values: &[i32]) -> Vec<u32> {
    values
        .iter()
        .filter_map(|value| u32::try_from(*value).ok())
        .collect()
}

fn signed(values: &[u32]) -> Vec<i32> {
    values
        .iter()
        .filter_map(|value| i32::try_from(*value).ok())
        .collect()
}

/// The character's row, or `None` when it has none yet.
pub async fn profile(pool: &PgPool, character: Uuid) -> sqlx::Result<Option<Profile>> {
    let row: Option<(Vec<i32>, Vec<i32>, Vec<i32>, Vec<i16>)> = sqlx::query_as(
        "SELECT learned, bar_skills, bar_copies, ranks FROM t_character_skills WHERE character_id = $1",
    )
    .bind(character)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(learned, skills, copies, ranks)| {
        let mut bar = [(0u32, 0u32); 8];
        for (slot, entry) in bar.iter_mut().enumerate() {
            let skill = skills
                .get(slot)
                .and_then(|id| u32::try_from(*id).ok())
                .unwrap_or(0);
            let copy = copies
                .get(slot)
                .and_then(|id| u32::try_from(*id).ok())
                .unwrap_or(0);
            *entry = (skill, copy);
        }
        Profile {
            learned: unsigned(&learned),
            bar,
            ranks: ranks
                .iter()
                .map(|rank| u8::try_from(*rank).unwrap_or(0))
                .collect(),
        }
    }))
}

/// Store the whole row, replacing what was there.
pub async fn save(pool: &PgPool, character: Uuid, profile: &Profile) -> sqlx::Result<()> {
    let skills: Vec<i32> = signed(&profile.bar.iter().map(|slot| slot.0).collect::<Vec<_>>());
    let copies: Vec<i32> = signed(&profile.bar.iter().map(|slot| slot.1).collect::<Vec<_>>());
    let ranks: Vec<i16> = profile.ranks.iter().map(|rank| i16::from(*rank)).collect();
    sqlx::query(
        "INSERT INTO t_character_skills (character_id, learned, bar_skills, bar_copies, ranks, updated_at)
         VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (character_id) DO UPDATE
             SET learned = $2, bar_skills = $3, bar_copies = $4, ranks = $5, updated_at = now()",
    )
    .bind(character)
    .bind(signed(&profile.learned))
    .bind(skills)
    .bind(copies)
    .bind(ranks)
    .execute(pool)
    .await?;
    Ok(())
}
