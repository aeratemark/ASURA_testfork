//! Login: turning the pair the client presents into an account, and reading what it may see.
//!
//! The client sends two UUIDs - a portal user id and a session id - and gets either a character list
//! or an error. That is the whole transaction, and this module is the only place it is decided.
//!
//! AzerothCore parallel: their `LoginDatabase` statement for the auth account, and the character-list
//! read that follows a successful login. The shape is theirs; the vendor is PostgreSQL
//! (docs/adr/0008-database-vendor.md).
//!
//! GWPS is the reference for the *rules*, and both of them are easy to get subtly wrong
//! (`docs/03-protocol.md` section 4):
//!
//! * **The pair is validated, never half-trusted.** `Db_GetSession(user_id, session_id)` looks up one
//!   row matching both. A right portal user with a wrong session id is not "close enough".
//! * **A closed session is never honoured.** `docs/05-gw2-lessons.md` rule 1: the newest session for
//!   an account wins and older ones are closed. A closed row therefore must not resolve, or a stolen
//!   session id would work forever.

use crate::{PgPool, Uuid};

/// A session that resolved: who the client is, and the account behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginSession {
    /// The session id as presented and confirmed.
    pub session: Uuid,
    /// The account the pair belongs to, which is what everything else is keyed by.
    pub account: Uuid,
    /// The character this session last selected, if any.
    pub character: Option<Uuid>,
}

/// One character, as the character-list message needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Character {
    pub id: Uuid,
    pub name: String,
    /// Where the character stands. The login batch reports it; the auth service does not move anyone.
    pub map_id: i32,
}

/// Validate the pair the client presented and resolve it to an account.
///
/// Returns `Ok(None)` for a pair that does not resolve **and for one that resolved in the past but is
/// closed**. The caller must treat `None` as "refuse, with the client's auth error" and must not try
/// to repair it - inventing a session here would be a login bypass (`crates/asura-db/migrations/
/// 0003_portal_identity.sql`).
pub async fn find_session(
    pool: &PgPool,
    portal_user: Uuid,
    session: Uuid,
) -> sqlx::Result<Option<LoginSession>> {
    sqlx::query_as::<_, (Uuid, Uuid, Option<Uuid>)>(
        "SELECT id, account_id, char_id
           FROM session
          WHERE portal_user = $1
            AND id = $2
            AND ended_at IS NULL",
    )
    .bind(portal_user)
    .bind(session)
    .fetch_optional(pool)
    .await
    .map(|row| {
        row.map(|(session, account, character)| LoginSession {
            session,
            account,
            character,
        })
    })
}

/// The characters this account may see, ordered by name so the list a client renders is stable.
///
/// Scoped by `account_id` in the query rather than filtered afterwards: a character list is the one
/// place where a missing `WHERE` would leak another player's data, so the scope belongs in the SQL.
pub async fn characters(pool: &PgPool, account: Uuid) -> sqlx::Result<Vec<Character>> {
    sqlx::query_as::<_, (Uuid, String, i32)>(
        "SELECT id, name, map_id
           FROM characters
          WHERE account_id = $1
          ORDER BY name",
    )
    .bind(account)
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(id, name, map_id)| Character { id, name, map_id })
            .collect()
    })
}

/// Record that a session chose this character. `char_id` is a routing hint for a reconnect
/// (`docs/02-architecture.md`, "Identity"), not an authorisation: the pair was already validated.
pub async fn select_character(pool: &PgPool, session: Uuid, character: Uuid) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE session SET char_id = $2
          WHERE id = $1 AND ended_at IS NULL",
    )
    .bind(session)
    .bind(character)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Delete one character, scoped to the account that owns it.
///
/// The `account_id` in the `WHERE` is what makes the reference's rule
/// (`AuthSrv_HandleDeleteCharacter` matches the name against *that account's* characters) hold at the
/// storage layer too: a wrong account id deletes nothing rather than someone else's character.
///
/// The session that named the character stops naming it, in the same transaction. A deleted character
/// left behind in `session.char_id` makes every travel go out for an id that no longer exists, and the
/// world answers "the character named by the transfer is gone" - measured 2026-10-28, after five
/// characters were deleted by hand.
pub async fn delete_character(pool: &PgPool, account: Uuid, character: Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;

    sqlx::query("UPDATE session SET char_id = NULL WHERE char_id = $1")
        .bind(character)
        .execute(&mut *tx)
        .await?;

    let done = sqlx::query("DELETE FROM characters WHERE account_id = $1 AND id = $2")
        .bind(account)
        .bind(character)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(done.rows_affected() > 0)
}

/// Close a session, so it can never resolve again. Idempotent: closing a closed session is a no-op,
/// which is what makes it safe to run on every disconnect and again on a stale sweep.
pub async fn close_session(pool: &PgPool, session: Uuid) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE session SET ended_at = now()
          WHERE id = $1 AND ended_at IS NULL",
    )
    .bind(session)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Close every live session of an account except the one just validated.
///
/// This is `docs/05-gw2-lessons.md` rule 1 expressed as SQL: when a player logs in again, the older
/// sessions are closed rather than left resolvable. Keeping it here - one statement, one place -
/// means "newest wins" cannot be half-implemented in the service.
pub async fn close_other_sessions(pool: &PgPool, account: Uuid, keep: Uuid) -> sqlx::Result<u64> {
    let done = sqlx::query(
        "UPDATE session SET ended_at = now()
          WHERE account_id = $1
            AND id <> $2
            AND ended_at IS NULL",
    )
    .bind(account)
    .bind(keep)
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

/// Everything a development login needs to exist: an account, a portal identity pair, and one
/// character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeededIdentity {
    pub portal_user: Uuid,
    pub session: Uuid,
    pub account: Uuid,
    pub character: Uuid,
}

/// Create (or refresh) a development identity: `portal_user` + `session` -> account -> character.
///
/// **Development and test support, not a service path.** Two reasons it lives here rather than in a
/// script full of SQL: this crate is the only place that writes SQL, and the tests need exactly this
/// to build the state they prove. It is idempotent, so re-running it reopens the same session instead
/// of stacking rows - which is also how a developer resets a session that a test closed.
///
/// The account gets its own generated id rather than reusing the portal user id: they are different
/// things (GWPS keeps them in different columns), and a seed that conflates them would hide exactly
/// the bug this schema change exists to prevent.
pub async fn seed_identity(
    pool: &PgPool,
    portal_user: Uuid,
    session: Uuid,
    account_name: &str,
    character_name: &str,
    map_id: i32,
) -> sqlx::Result<SeededIdentity> {
    let mut tx = pool.begin().await?;

    // Upsert on the unique username and return the row's id either way. `DO UPDATE` is what makes
    // RETURNING fire for an existing row; `SET username = EXCLUDED.username` is a no-op write.
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO account (id, username) VALUES (gen_random_uuid(), $1)
         ON CONFLICT (username) DO UPDATE SET username = EXCLUDED.username
         RETURNING id",
    )
    .bind(account_name)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO session (id, account_id, portal_user, started_at, ended_at)
         VALUES ($1, $2, $3, now(), NULL)
         ON CONFLICT (id) DO UPDATE SET
             account_id  = EXCLUDED.account_id,
             portal_user = EXCLUDED.portal_user,
             started_at  = now(),
             ended_at    = NULL",
    )
    .bind(session)
    .bind(account)
    .bind(portal_user)
    .execute(&mut *tx)
    .await?;

    let character: Uuid = match sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM characters WHERE account_id = $1 AND name = $2",
    )
    .bind(account)
    .bind(character_name)
    .fetch_optional(&mut *tx)
    .await?
    {
        Some(existing) => existing,
        None => {
            sqlx::query_scalar(
                "INSERT INTO characters (id, account_id, name, map_id)
             VALUES (gen_random_uuid(), $1, $2, $3)
             RETURNING id",
            )
            .bind(account)
            .bind(character_name)
            .bind(map_id)
            .fetch_one(&mut *tx)
            .await?
        }
    };

    tx.commit().await?;

    Ok(SeededIdentity {
        portal_user,
        session,
        account,
        character,
    })
}

/// Remove what [`seed_identity`] created - an account and, by cascade, its sessions and characters.
///
/// Test hygiene: a run leaves nothing behind that the next run would mistake for a live account.
pub async fn remove_account(pool: &PgPool, account_name: &str) -> sqlx::Result<bool> {
    let done = sqlx::query("DELETE FROM account WHERE username = $1")
        .bind(account_name)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
