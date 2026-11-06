//! Guilds: the guild row, its members, and its open invitations (migration 0029).
//!
//! Rows only. Who may create, invite, leave or set the announcement is `asura_rules::guild`; what a new guild
//! starts with is `config/Server/guilds.json`, handed in as [`NewGuild`]. Every call is a statement or one
//! short transaction, run when a connection is built or when the client asks for a change.

use sqlx::postgres::PgRow;
use sqlx::Row;

use crate::{PgPool, Uuid};

/// A guild's cape, as its row stores it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cape {
    /// Background colour.
    pub background_color: u8,
    /// Detail colour.
    pub detail_color: u8,
    /// Emblem colour.
    pub emblem_color: u8,
    /// Shape.
    pub shape: u8,
    /// Detail pattern.
    pub detail: u8,
    /// Emblem.
    pub emblem: u16,
    /// Trim.
    pub trim: u8,
}

/// One guild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guild {
    /// The guild's id, which is also the sixteen-byte key its messages carry.
    pub id: Uuid,
    /// Its name.
    pub name: String,
    /// Its tag.
    pub tag: String,
    /// `0x0121`'s feature byte.
    pub feature: u8,
    /// `0x0121`'s territory byte.
    pub territory: u8,
    /// Its cape.
    pub cape: Cape,
    /// Its allegiance.
    pub allegiance: u8,
    /// Faction points.
    pub faction: u32,
    /// Qualifier points.
    pub qualifier: u32,
    /// Ladder rating.
    pub rating: u32,
    /// Ladder position.
    pub rank: u32,
    /// The announcement text.
    pub announcement: String,
    /// Who set it.
    pub announcement_author: String,
    /// The counter `0x0133` carries; advanced once per change.
    pub revision: u32,
}

/// One member row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The guild.
    pub guild_id: Uuid,
    /// The character.
    pub character_id: Uuid,
    /// The client's member type (5 the leader).
    pub member_type: u8,
    /// The name it was invited under.
    pub invited_name: String,
    /// Who invited it.
    pub invited_by: String,
    /// When it joined, in seconds since 1970-01-01 UTC.
    pub joined_at: i64,
}

/// What a new guild's row starts with: the values in `config/Server/guilds.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewGuild {
    /// `0x0121`'s feature byte.
    pub feature: u8,
    /// `0x0121`'s territory byte.
    pub territory: u8,
    /// Allegiance.
    pub allegiance: u8,
    /// Ladder rating.
    pub rating: u32,
    /// Ladder position.
    pub rank: u32,
    /// The first value `0x0133` carries.
    pub revision: u32,
}

const GUILD_COLUMNS: &str = "g.id, g.name, g.tag, g.feature, g.territory, g.cape_background, \
    g.cape_detail_color, g.cape_emblem_color, g.cape_shape, g.cape_detail, g.cape_emblem, g.cape_trim, \
    g.allegiance, g.faction, g.qualifier, g.rating, g.rank, g.announcement, g.announcement_author, \
    g.revision";

const MEMBER_COLUMNS: &str =
    "m.guild_id, m.character_id, m.member_type, m.invited_name, m.invited_by, \
    extract(epoch from m.joined_at)::bigint";

fn byte(row: &PgRow, index: usize) -> sqlx::Result<u8> {
    let value: i16 = row.try_get(index)?;
    Ok(value.clamp(0, 255) as u8)
}

fn dword(row: &PgRow, index: usize) -> sqlx::Result<u32> {
    let value: i64 = row.try_get(index)?;
    Ok(value.clamp(0, i64::from(u32::MAX)) as u32)
}

/// A guild from [`GUILD_COLUMNS`], starting at column `at`.
fn guild_at(row: &PgRow, at: usize) -> sqlx::Result<Guild> {
    let emblem: i32 = row.try_get(at + 10)?;
    Ok(Guild {
        id: row.try_get(at)?,
        name: row.try_get(at + 1)?,
        tag: row.try_get(at + 2)?,
        feature: byte(row, at + 3)?,
        territory: byte(row, at + 4)?,
        cape: Cape {
            background_color: byte(row, at + 5)?,
            detail_color: byte(row, at + 6)?,
            emblem_color: byte(row, at + 7)?,
            shape: byte(row, at + 8)?,
            detail: byte(row, at + 9)?,
            emblem: emblem.clamp(0, 65_535) as u16,
            trim: byte(row, at + 11)?,
        },
        allegiance: byte(row, at + 12)?,
        faction: dword(row, at + 13)?,
        qualifier: dword(row, at + 14)?,
        rating: dword(row, at + 15)?,
        rank: dword(row, at + 16)?,
        announcement: row.try_get(at + 17)?,
        announcement_author: row.try_get(at + 18)?,
        revision: dword(row, at + 19)?,
    })
}

/// The number of columns [`GUILD_COLUMNS`] selects.
const GUILD_WIDTH: usize = 20;

/// A member from [`MEMBER_COLUMNS`], starting at column `at`.
fn member_at(row: &PgRow, at: usize) -> sqlx::Result<Member> {
    Ok(Member {
        guild_id: row.try_get(at)?,
        character_id: row.try_get(at + 1)?,
        member_type: byte(row, at + 2)?,
        invited_name: row.try_get(at + 3)?,
        invited_by: row.try_get(at + 4)?,
        joined_at: row.try_get(at + 5)?,
    })
}

/// The guild a character is in and its own member row, or `None` when it is in none.
pub async fn membership(pool: &PgPool, character: Uuid) -> sqlx::Result<Option<(Guild, Member)>> {
    let statement = format!(
        "SELECT {GUILD_COLUMNS}, {MEMBER_COLUMNS}
           FROM t_guild_members m JOIN t_guilds g ON g.id = m.guild_id
          WHERE m.character_id = $1"
    );
    let row = sqlx::query(&statement)
        .bind(character)
        .fetch_optional(pool)
        .await?;
    match row {
        Some(row) => Ok(Some((guild_at(&row, 0)?, member_at(&row, GUILD_WIDTH)?))),
        None => Ok(None),
    }
}

/// Create a guild with its founder as the only member, in one transaction. `None`, and nothing written,
/// when the name is taken or the founder is in a guild already: both are unique keys, so two founders
/// racing for one name cannot both win.
#[allow(clippy::too_many_arguments)]
pub async fn create(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    tag: &str,
    start: &NewGuild,
    founder: Uuid,
    founder_name: &str,
    leader_type: u8,
) -> sqlx::Result<Option<(Guild, Member)>> {
    let mut tx = pool.begin().await?;
    let made = sqlx::query(
        "INSERT INTO t_guilds (id, name, tag, feature, territory, allegiance, rating, rank, revision)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(name)
    .bind(tag)
    .bind(i16::from(start.feature))
    .bind(i16::from(start.territory))
    .bind(i16::from(start.allegiance))
    .bind(i64::from(start.rating))
    .bind(i64::from(start.rank))
    .bind(i64::from(start.revision))
    .execute(&mut *tx)
    .await?;
    if made.rows_affected() != 1 {
        tx.rollback().await?;
        return Ok(None);
    }
    let joined = sqlx::query(
        "INSERT INTO t_guild_members (character_id, guild_id, member_type, invited_name, invited_by)
         VALUES ($1, $2, $3, $4, $4)
         ON CONFLICT DO NOTHING",
    )
    .bind(founder)
    .bind(id)
    .bind(i16::from(leader_type))
    .bind(founder_name)
    .execute(&mut *tx)
    .await?;
    if joined.rows_affected() != 1 {
        tx.rollback().await?;
        return Ok(None);
    }
    tx.commit().await?;
    membership(pool, founder).await
}

/// The character with this name, matched without regard to case: its id and its name as stored.
pub async fn character_named(pool: &PgPool, name: &str) -> sqlx::Result<Option<(Uuid, String)>> {
    sqlx::query_as("SELECT id, name FROM t_characters WHERE lower(name) = lower($1) LIMIT 1")
        .bind(name)
        .fetch_optional(pool)
        .await
}

/// Whether a character is in any guild.
pub async fn is_member(pool: &PgPool, character: Uuid) -> sqlx::Result<bool> {
    let found: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM t_guild_members WHERE character_id = $1")
            .bind(character)
            .fetch_optional(pool)
            .await?;
    Ok(found.is_some())
}

/// Whether this guild has an open invitation of this character.
pub async fn is_invited(pool: &PgPool, guild: Uuid, character: Uuid) -> sqlx::Result<bool> {
    let found: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM t_guild_invites WHERE guild_id = $1 AND character_id = $2",
    )
    .bind(guild)
    .bind(character)
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

/// Open an invitation. Returns when it was made, in seconds since 1970-01-01 UTC, or `None` when this
/// guild had already invited the character.
pub async fn invite(
    pool: &PgPool,
    guild: Uuid,
    character: Uuid,
    invited_name: &str,
    invited_by: &str,
) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar(
        "INSERT INTO t_guild_invites (guild_id, character_id, invited_name, invited_by)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT DO NOTHING
         RETURNING extract(epoch from invited_at)::bigint",
    )
    .bind(guild)
    .bind(character)
    .bind(invited_name)
    .bind(invited_by)
    .fetch_optional(pool)
    .await
}

/// How many members a guild has besides this character.
pub async fn other_members(pool: &PgPool, guild: Uuid, character: Uuid) -> sqlx::Result<usize> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM t_guild_members WHERE guild_id = $1 AND character_id <> $2",
    )
    .bind(guild)
    .bind(character)
    .fetch_one(pool)
    .await?;
    Ok(usize::try_from(count).unwrap_or(0))
}

/// Take a character out of its guild. Whether there was a row to remove.
pub async fn leave(pool: &PgPool, character: Uuid) -> sqlx::Result<bool> {
    let gone = sqlx::query("DELETE FROM t_guild_members WHERE character_id = $1")
        .bind(character)
        .execute(pool)
        .await?;
    Ok(gone.rows_affected() == 1)
}

/// Delete a guild; its members and invitations go with it (`ON DELETE CASCADE`).
pub async fn dissolve(pool: &PgPool, guild: Uuid) -> sqlx::Result<bool> {
    let gone = sqlx::query("DELETE FROM t_guilds WHERE id = $1")
        .bind(guild)
        .execute(pool)
        .await?;
    Ok(gone.rows_affected() == 1)
}

/// Store the announcement and who set it.
pub async fn set_announcement(
    pool: &PgPool,
    guild: Uuid,
    text: &str,
    author: &str,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE t_guilds SET announcement = $2, announcement_author = $3 WHERE id = $1")
        .bind(guild)
        .bind(text)
        .bind(author)
        .execute(pool)
        .await?;
    Ok(())
}

/// Advance the guild's `0x0133` counter by `step` and return the new value, wrapping within a dword.
/// `None` when the guild is gone.
pub async fn advance_revision(pool: &PgPool, guild: Uuid, step: u32) -> sqlx::Result<Option<u32>> {
    let value: Option<i64> = sqlx::query_scalar(
        "UPDATE t_guilds SET revision = (revision + $2) % 4294967296 WHERE id = $1 RETURNING revision",
    )
    .bind(guild)
    .bind(i64::from(step))
    .fetch_optional(pool)
    .await?;
    Ok(value.map(|value| value.clamp(0, i64::from(u32::MAX)) as u32))
}
