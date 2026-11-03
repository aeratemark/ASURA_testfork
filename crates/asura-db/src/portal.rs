//! The portal identity: which account an email is, and the pair the web gate hands the client.
//!
//! The client's login does not ask an account database who it is - it asks the **portal web gate**
//! (`/Spawned/WebGate/...`) and then presents the pair it was given: the portal user id and the
//! session id (`docs/03-protocol.md` #4, `docs/06-reference-session.md`). The reference's
//! `tools/webgate.py` answers from a hard-coded table of three emails and UUID pairs; this project
//! answers from the rows `tools/seed-dev.ps1` writes, which are the same pairs.
//!
//! The lookup is exact on both UUIDs - `WHERE portal_user = $1 AND id = $2` in the auth service -
//! so the gate's answer is not a claim about identity, it is the identity the auth service will
//! resolve. The pair must come from one **live** session: a closed session is a credential that was
//! surrendered, exactly as the auth service treats it (`docs/05-gw2-lessons.md`, rule 1).

use crate::{PgPool, Uuid};

/// The portal identity an email resolves to: `(portal_user, session id)`.
///
/// `None` means "no live session for that account", which the web gate answers the way the
/// reference does: HTTP 400, `who are you?`.
pub async fn identity_for_email(pool: &PgPool, email: &str) -> sqlx::Result<Option<(Uuid, Uuid)>> {
    sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT s.portal_user, s.id
           FROM session s
           JOIN account a ON a.id = s.account_id
          WHERE a.username = $1
            AND s.ended_at IS NULL
          ORDER BY s.started_at DESC
          LIMIT 1",
    )
    .bind(email.trim())
    .fetch_optional(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A live database with a fresh account and session, or `None` to skip.
    async fn pool_with_identity(label: &str) -> Option<(PgPool, String, Uuid, Uuid)> {
        let url = crate::config::database_url()?;
        let pool = crate::placement::connect(&url, 2).await.ok()?;
        let email = format!("webgate-{label}-{}@example.test", std::process::id());
        let portal = Uuid::new_v4();
        let session = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO account (id, username) VALUES (gen_random_uuid(), $1)
             ON CONFLICT (username) DO NOTHING",
        )
        .bind(&email)
        .execute(&pool)
        .await
        .ok()?;
        sqlx::query(
            "INSERT INTO session (id, account_id, portal_user, started_at)
             SELECT $1, a.id, $2, now() FROM account a WHERE a.username = $3
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(session)
        .bind(portal)
        .bind(&email)
        .execute(&pool)
        .await
        .ok()?;
        Some((pool, email, portal, session))
    }

    async fn clean(pool: &PgPool, email: &str) {
        let _ = sqlx::query("DELETE FROM account WHERE username = $1")
            .bind(email)
            .execute(pool)
            .await;
    }

    #[tokio::test]
    async fn an_email_resolves_to_its_live_portal_pair() {
        let Some((pool, email, portal, session)) = pool_with_identity("live").await else {
            println!(
                "ASURA_DATABASE_URL not set (or unreachable); skipping live-database portal test"
            );
            return;
        };

        let found = identity_for_email(&pool, &email).await.expect("lookup");
        assert_eq!(found, Some((portal, session)), "the row's own pair");
        assert_eq!(
            identity_for_email(&pool, "nobody-here@example.test")
                .await
                .expect("lookup"),
            None,
            "an unknown email has no identity"
        );

        clean(&pool, &email).await;
    }

    #[tokio::test]
    async fn a_closed_session_does_not_resolve() {
        let Some((pool, email, _portal, session)) = pool_with_identity("closed").await else {
            println!(
                "ASURA_DATABASE_URL not set (or unreachable); skipping live-database portal test"
            );
            return;
        };
        sqlx::query("UPDATE session SET ended_at = now() WHERE id = $1")
            .bind(session)
            .execute(&pool)
            .await
            .expect("close the session");

        assert_eq!(
            identity_for_email(&pool, &email).await.expect("lookup"),
            None,
            "a surrendered session is not a credential"
        );

        clean(&pool, &email).await;
    }
}
