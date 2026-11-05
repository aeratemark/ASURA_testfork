//! Title progress: the points a character and its account have on each title, and the title it shows.
//!
//! Points only. What a point count means - its rank, the next rank, the records' names - is
//! `config/Server/titles.json`, applied by `asura_rules::titles`; migration 0028 has the reasoning. Every
//! call is one statement and none is on a hot path: progress is read once when a connection is built and
//! written when it changes.

use crate::{PgPool, Uuid};

/// Whose row a point count is in. Mirrors the title file's `scope`; the caller maps one to the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// `t_character_titles`, keyed by the character.
    Character(Uuid),
    /// `t_account_titles`, keyed by the account.
    Account(Uuid),
}

/// The points on every title that has a row, for one character and for its account. Two lists, because
/// the scope of a title is the title file's to say, not this crate's: the caller picks the list a title's
/// scope names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    /// `(title_id, points)` from `t_character_titles`.
    pub character: Vec<(u32, u32)>,
    /// `(title_id, points)` from `t_account_titles`.
    pub account: Vec<(u32, u32)>,
}

fn to_pairs(rows: Vec<(i16, i64)>) -> Vec<(u32, u32)> {
    rows.into_iter()
        .filter_map(|(title_id, points)| {
            Some((u32::try_from(title_id).ok()?, u32::try_from(points).ok()?))
        })
        .collect()
}

/// Every title row this character and its account have.
pub async fn progress(pool: &PgPool, character: Uuid, account: Uuid) -> sqlx::Result<Progress> {
    let character_rows: Vec<(i16, i64)> = sqlx::query_as(
        "SELECT title_id, points FROM t_character_titles WHERE character_id = $1 ORDER BY title_id",
    )
    .bind(character)
    .fetch_all(pool)
    .await?;
    let account_rows: Vec<(i16, i64)> = sqlx::query_as(
        "SELECT title_id, points FROM t_account_titles WHERE account_id = $1 ORDER BY title_id",
    )
    .bind(account)
    .fetch_all(pool)
    .await?;
    Ok(Progress {
        character: to_pairs(character_rows),
        account: to_pairs(account_rows),
    })
}

/// Store the points one title now has, replacing what was there. The caller computed them from the row
/// it read when the connection began plus what this session earned.
pub async fn record_points(
    pool: &PgPool,
    owner: Owner,
    title_id: u32,
    points: u32,
) -> sqlx::Result<()> {
    let (statement, key) = match owner {
        Owner::Character(id) => (
            "INSERT INTO t_character_titles (character_id, title_id, points, updated_at)
             VALUES ($1, $2, $3, now())
             ON CONFLICT (character_id, title_id) DO UPDATE SET points = $3, updated_at = now()",
            id,
        ),
        Owner::Account(id) => (
            "INSERT INTO t_account_titles (account_id, title_id, points, updated_at)
             VALUES ($1, $2, $3, now())
             ON CONFLICT (account_id, title_id) DO UPDATE SET points = $3, updated_at = now()",
            id,
        ),
    };
    sqlx::query(statement)
        .bind(key)
        .bind(title_id as i16)
        .bind(i64::from(points))
        .execute(pool)
        .await?;
    Ok(())
}

/// The title this character asked to show, if any.
pub async fn display_title(pool: &PgPool, character: Uuid) -> sqlx::Result<Option<u32>> {
    let shown: Option<Option<i16>> =
        sqlx::query_scalar("SELECT display_title FROM t_characters WHERE id = $1")
            .bind(character)
            .fetch_optional(pool)
            .await?;
    Ok(shown
        .flatten()
        .and_then(|title_id| u32::try_from(title_id).ok()))
}

/// Remember the title this character shows, or that it shows none.
pub async fn record_display_title(
    pool: &PgPool,
    character: Uuid,
    title_id: Option<u32>,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE t_characters SET display_title = $2 WHERE id = $1")
        .bind(character)
        .bind(title_id.map(|id| id as i16))
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_the_wire_cannot_carry_is_dropped_not_wrapped() {
        assert_eq!(
            to_pairs(vec![(21, 13), (-1, 5), (7, -1), (5, 10_645_000)]),
            vec![(21, 13), (5, 10_645_000)]
        );
    }
}
