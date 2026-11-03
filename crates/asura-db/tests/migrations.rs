//! The database's own record of what it has had done to it.
//!
//! Runs against the live PostgreSQL named by `ASURA_DATABASE_URL` (the environment, or the
//! repository's `.env`), and skips cleanly without one.
//!
//! What this proves, and why it is worth a test rather than a comment: a record that is merely
//! written is not a record that can be *relied* on. The last test here takes a version away and
//! checks that the startup check notices, names what is missing, and that re-applying puts it back -
//! which is the whole point of keeping the record.

use asura_db::migrations::{self, EnsureError, LATEST_VERSION};
use asura_db::PgPool;
use std::sync::OnceLock;
use tokio::sync::Mutex;

async fn pool() -> Option<PgPool> {
    // One rule for finding the settings, in `asura_db::config`, shared with the services.
    let url = asura_db::config::database_url()?;
    Some(
        asura_db::placement::connect(&url, 4)
            .await
            .expect("connect pool"),
    )
}

/// The tests below share one `schema_version` table, and one of them deliberately removes a row from
/// it. Run in parallel, that removal is visible to the other test halfway through its own assertions -
/// which is a flake, not a bug in the code under test. They take turns instead.
fn schema_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn skip() {
    println!("ASURA_DATABASE_URL not set; skipping live-database migration test");
}

#[tokio::test]
async fn the_database_records_every_migration_and_nothing_is_pending() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let _guard = schema_lock().lock().await;

    let result = migrations::apply_pending(&pool).await.expect("apply");
    assert!(
        result.now_current.contains(&LATEST_VERSION),
        "the database must record the newest version this build knows: {:?}",
        result.now_current
    );

    assert_eq!(
        migrations::pending_versions(&pool).await.expect("pending"),
        Vec::<i32>::new(),
        "nothing may be pending after applying"
    );

    // A second call must do nothing at all, including not re-running a migration.
    let again = migrations::apply_pending(&pool).await.expect("apply again");
    assert_eq!(
        again.applied,
        Vec::<i32>::new(),
        "applying twice is a no-op"
    );

    assert!(migrations::ensure_current(&pool).await.is_ok());
}

#[tokio::test]
async fn a_database_that_is_behind_is_refused_and_names_what_is_missing() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let _guard = schema_lock().lock().await;
    migrations::apply_pending(&pool).await.expect("apply");

    // Take the record away without undoing the change: exactly the state a half-finished deployment
    // leaves, and the state a startup check has to notice.
    sqlx::query("DELETE FROM schema_version WHERE version = $1")
        .bind(LATEST_VERSION)
        .execute(&pool)
        .await
        .expect("forget the newest version");

    assert_eq!(
        migrations::pending_versions(&pool).await.expect("pending"),
        vec![LATEST_VERSION]
    );

    match migrations::ensure_current(&pool).await {
        Err(EnsureError::TooOld(too_old)) => {
            assert_eq!(too_old.missing, vec![LATEST_VERSION]);
            assert_eq!(too_old.expected, LATEST_VERSION);
            let text = too_old.to_string();
            assert!(
                text.contains(&LATEST_VERSION.to_string()),
                "the refusal must name the missing version, said: {text}"
            );
        }
        other => panic!("a database behind the build must be refused, got {other:?}"),
    }

    // Re-applying restores the record, and the check passes again. A database is never left in the
    // refused state by the tooling that caused it.
    let restored = migrations::apply_pending(&pool).await.expect("re-apply");
    assert_eq!(restored.applied, vec![LATEST_VERSION]);
    assert!(migrations::ensure_current(&pool).await.is_ok());
}
