//! A character's quest attempts: one row per (character, quest).
//!
//! The stage values are the rule's (`asura_rules::quest::Stage`): 1 active, 2 completed, 3 rewarded,
//! 4 abandoned. This module stores rows and applies the writes a quest needs - an acceptance, a completed
//! objective, an abandon and a turn-in - and knows no rule. The caller decides whether either may happen and hands the values
//! in; the content and the wire stay outside this crate (`docs/58-quests.md`).

use crate::{PgPool, Uuid};

/// One stored attempt, as the row carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The client's quest id.
    pub quest_id: u32,
    /// The attempt number.
    pub cycle: u32,
    /// 1 active, 2 completed, 3 rewarded, 4 abandoned.
    pub stage: i16,
}

/// Read every attempt one character has, in quest-id order.
pub async fn load(pool: &PgPool, character: Uuid) -> sqlx::Result<Vec<Row>> {
    let rows: Vec<(i32, i32, i16)> = sqlx::query_as(
        "SELECT quest_id, cycle, stage FROM t_character_quests
          WHERE character_id = $1 ORDER BY quest_id",
    )
    .bind(character)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(quest_id, cycle, stage)| {
            let quest_id = u32::try_from(quest_id)
                .map_err(|_| sqlx::Error::Protocol("quest id is negative".to_string()))?;
            let cycle = u32::try_from(cycle)
                .map_err(|_| sqlx::Error::Protocol("quest cycle is negative".to_string()))?;
            Ok(Row {
                quest_id,
                cycle,
                stage,
            })
        })
        .collect()
}

/// Record an acceptance: the quest becomes stage 1 at the cycle the rule chose.
///
/// The upsert is intentional: a reacceptance replaces an abandoned or repeatable-rewarded row's cycle.
/// The caller has already checked that the transition is allowed; one connection drives one character,
/// so there is no second writer to race.
pub async fn accept(
    pool: &PgPool,
    character: Uuid,
    quest_id: u32,
    cycle: u32,
) -> sqlx::Result<bool> {
    let quest_id = i32::try_from(quest_id).unwrap_or(i32::MAX);
    let cycle = i32::try_from(cycle).unwrap_or(i32::MAX);
    let result = sqlx::query(
        "INSERT INTO t_character_quests (character_id, quest_id, cycle, stage)
              VALUES ($1, $2, $3, 1)
         ON CONFLICT (character_id, quest_id) DO UPDATE
                SET cycle = EXCLUDED.cycle, stage = 1, updated_at = now()",
    )
    .bind(character)
    .bind(quest_id)
    .bind(cycle)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Record a completed objective: an active attempt at this cycle becomes stage 2.
///
/// The cycle and the stage are the fence; a row that has moved on changes nothing and answers `false`.
pub async fn complete(
    pool: &PgPool,
    character: Uuid,
    quest_id: u32,
    cycle: u32,
) -> sqlx::Result<bool> {
    let quest_id = i32::try_from(quest_id).unwrap_or(i32::MAX);
    let cycle = i32::try_from(cycle).unwrap_or(i32::MAX);
    let result = sqlx::query(
        "UPDATE t_character_quests
            SET stage = 2, updated_at = now()
          WHERE character_id = $1 AND quest_id = $2 AND cycle = $3 AND stage = 1",
    )
    .bind(character)
    .bind(quest_id)
    .bind(cycle)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Record an abandon: an active or completed attempt at this cycle becomes stage 4.
///
/// The cycle and the stage are the fence, as for a turn-in: a rewarded attempt, or one that has moved on
/// since, changes no row and the call answers `false`. The cycle is kept, so a later acceptance starts
/// the next one.
pub async fn abandon(
    pool: &PgPool,
    character: Uuid,
    quest_id: u32,
    cycle: u32,
) -> sqlx::Result<bool> {
    let quest_id = i32::try_from(quest_id).unwrap_or(i32::MAX);
    let cycle = i32::try_from(cycle).unwrap_or(i32::MAX);
    let result = sqlx::query(
        "UPDATE t_character_quests
            SET stage = 4, updated_at = now()
          WHERE character_id = $1 AND quest_id = $2 AND cycle = $3 AND stage IN (1, 2)",
    )
    .bind(character)
    .bind(quest_id)
    .bind(cycle)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Record a turn-in and the reward it pays, in one transaction.
///
/// The quest update is the fence: it succeeds only for an active or completed attempt at the cycle the
/// caller named. If it changes no row, the character's experience and gold are not written either. The
/// caller sends the client's messages only after this returns `true`.
#[allow(clippy::too_many_arguments)]
pub async fn reward(
    pool: &PgPool,
    character: Uuid,
    quest_id: u32,
    cycle: u32,
    experience: u32,
    level: u16,
    skill_points: u32,
    skill_points_total: u32,
    gold: u32,
) -> sqlx::Result<bool> {
    let quest_id = i32::try_from(quest_id).unwrap_or(i32::MAX);
    let cycle = i32::try_from(cycle).unwrap_or(i32::MAX);
    let mut transaction = pool.begin().await?;
    let changed = sqlx::query(
        "UPDATE t_character_quests
            SET stage = 3, updated_at = now()
          WHERE character_id = $1 AND quest_id = $2 AND cycle = $3 AND stage IN (1, 2)",
    )
    .bind(character)
    .bind(quest_id)
    .bind(cycle)
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() != 1 {
        transaction.rollback().await?;
        return Ok(false);
    }
    sqlx::query(
        "UPDATE t_characters
            SET experience = $2, level = $3, skill_points = $4, skill_points_total = $5,
                gold = gold + $6
          WHERE id = $1",
    )
    .bind(character)
    .bind(i32::try_from(experience).unwrap_or(i32::MAX))
    .bind(i16::try_from(level).unwrap_or(i16::MAX))
    .bind(i32::try_from(skill_points).unwrap_or(i32::MAX))
    .bind(i32::try_from(skill_points_total).unwrap_or(i32::MAX))
    .bind(i32::try_from(gold).unwrap_or(i32::MAX))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(true)
}
