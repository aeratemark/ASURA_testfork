//! The account: the row every other table is keyed by, and its own state.
//!
//! Everything else in this crate names an account by id and never reads the row; this module is the
//! one place that does. It holds what the login path needs and grows when a feature needs a column,
//! not before.
//!
//! `eula_accepted` is the version of the EULA this account accepted, and 0 is "never". The client
//! shows the page while the number in `AUTH_SMSG_ACCOUNT_INFO` is lower than the version it knows,
//! and `AUTH_CMSG_ACCEPT_EULA` carries the version a person accepted
//! (`crates/asura-db/migrations/0012_account_eula.sql` has the measurement).
//!
//! `account_settings` is the client's own settings blob - the UI layout and interface options - as
//! it uploaded it with `AUTH_CMSG_SETTING_UPDATE_SIZE` and `AUTH_CMSG_SETTING_UPDATE_CONTENT`. The
//! login path sends it back verbatim in `AUTH_SMSG_ACCOUNT_SETTINGS`; the bytes are never decoded
//! (`crates/asura-db/migrations/0032_account_settings.sql`, `docs/46-ui-settings-persistence.md`).

use crate::{PgPool, Uuid};

/// One account row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// The row`s own identifier.
    pub id: Uuid,
    /// The account name the client presents.
    pub username: String,
    /// The EULA version this account accepted; `0` for never.
    pub eula_accepted: i16,
    /// The client's uploaded settings blob, if it ever sent one. `None` means the login path answers
    /// with the committed default (`config/Server/account_settings.json`).
    pub account_settings: Option<Vec<u8>>,
}

/// Read one account. `None` means no such row, which the login path treats as a refusal.
pub async fn load(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<Account>> {
    let row = sqlx::query_as::<_, (Uuid, String, i16, Option<Vec<u8>>)>(
        "SELECT id, username, eula_accepted, account_settings FROM t_account WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(id, username, eula_accepted, account_settings)| Account {
        id,
        username,
        eula_accepted,
        account_settings,
    }))
}

/// Record the settings blob the client last uploaded. `false` means no row matched, which the caller
/// can only report: the upload was already acknowledged, and refusing after the fact would make the
/// client retry a write that has nowhere to land.
pub async fn record_settings(pool: &PgPool, id: Uuid, settings: &[u8]) -> sqlx::Result<bool> {
    let done = sqlx::query("UPDATE t_account SET account_settings = $2 WHERE id = $1")
        .bind(id)
        .bind(settings)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() == 1)
}

/// Record the EULA version an account accepted. `false` means no row matched, which the caller can
/// only report: the client has already been answered either way, and refusing after the fact would
/// ask it again in a loop.
pub async fn record_eula(pool: &PgPool, id: Uuid, version: i16) -> sqlx::Result<bool> {
    let done = sqlx::query("UPDATE t_account SET eula_accepted = $2 WHERE id = $1")
        .bind(id)
        .bind(version)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() == 1)
}
