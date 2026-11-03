//! The service registry against the live database (S2): register, refresh, mark stale, stop,
//! forget. Skips cleanly when no database is configured.

use asura_db::catalog;

fn database_url() -> Option<String> {
    asura_db::config::database_url()
}

const KIND: &str = "test-catalog";
const NAME: &str = "catalog-test-1";

#[tokio::test]
async fn a_row_is_registered_refreshed_and_marked_stale() {
    let Some(url) = database_url() else {
        eprintln!("skipped: no database configured");
        return;
    };
    let pool = asura_db::PgPool::connect(&url).await.expect("database");
    catalog::delete(&pool, KIND, NAME).await.expect("clean");

    catalog::upsert(&pool, KIND, NAME, "127.0.0.1", 9999, "asura/test", "ready")
        .await
        .expect("register");
    let row = catalog::services(&pool, 60.0)
        .await
        .expect("list")
        .into_iter()
        .find(|row| row.kind == KIND && row.name == NAME)
        .expect("the row is listed");
    assert_eq!(row.port, 9999);
    assert_eq!(row.address, "127.0.0.1");
    assert!(row.fresh, "a just-registered row is fresh");

    // Refresh: the upsert updates the address and the heartbeat without duplicating the row.
    catalog::upsert(&pool, KIND, NAME, "10.0.0.1", 9999, "asura/test2", "ready")
        .await
        .expect("refresh");
    let row = catalog::services(&pool, 60.0)
        .await
        .expect("list")
        .into_iter()
        .find(|row| row.kind == KIND && row.name == NAME)
        .expect("the row is listed");
    assert_eq!(row.address, "10.0.0.1");
    assert_eq!(row.build, "asura/test2");

    // A dead service is one whose heartbeat went stale: it must not read as fresh.
    sqlx::query("UPDATE service SET heartbeat_at = now() - interval '10 minutes' WHERE kind = $1 AND name = $2")
        .bind(KIND)
        .bind(NAME)
        .execute(&pool)
        .await
        .expect("age the heartbeat");
    let row = catalog::services(&pool, 60.0)
        .await
        .expect("list")
        .into_iter()
        .find(|row| row.kind == KIND && row.name == NAME)
        .expect("the row is listed");
    assert!(!row.fresh, "a stale heartbeat must not read as fresh");
    assert!(row.age_secs > 60.0, "age: {}", row.age_secs);

    // Stopping is a state change, not a deletion: the record stays.
    catalog::set_state(&pool, KIND, NAME, "stopped")
        .await
        .expect("stop");
    let row = catalog::services(&pool, 60.0)
        .await
        .expect("list")
        .into_iter()
        .find(|row| row.kind == KIND && row.name == NAME)
        .expect("the row is listed");
    assert_eq!(row.state, "stopped");

    catalog::delete(&pool, KIND, NAME).await.expect("cleanup");
    assert!(
        catalog::services(&pool, 60.0)
            .await
            .expect("list")
            .iter()
            .all(|row| !(row.kind == KIND && row.name == NAME)),
        "the delete removed the row"
    );
}
