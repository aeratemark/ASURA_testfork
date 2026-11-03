//! The general service registry (S2): every service kind announces itself like a world server does.
//!
//! A row says where a service is (address, port), its build, whether it is alive (`heartbeat_at`)
//! and what state it is in. The supervisor writes a row for every service it starts; consumers read
//! them instead of hard-coding addresses. Worlds keep their own `world` table as the *load-bearing*
//! registry (placement and the front door read it, lease reaping keys on staleness); this table is
//! the general one, written beside it.
//!
//! It is a table, not a service: no component talks to the database as a proxy between services
//! (`docs/05-gw2-lessons.md`, "Not adopting: a separate DB service layer"). That distinction is
//! what keeps the catalog from becoming a message bus.

use crate::PgPool;

/// One service, as consumers see it: where it is, and whether its heartbeat is still fresh.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceRow {
    pub kind: String,
    pub name: String,
    pub address: String,
    pub port: i32,
    pub build: String,
    pub state: String,
    /// `heartbeat_at` is inside the freshness window.
    pub fresh: bool,
    /// Seconds since the last heartbeat.
    pub age_secs: f64,
}

/// Register or refresh one service.
///
/// An upsert, so a restarted service overwrites the previous life's address, build and state, and
/// repeated calls are harmless - which is what lets the supervisor publish every few seconds.
pub async fn upsert(
    pool: &PgPool,
    kind: &str,
    name: &str,
    address: &str,
    port: i32,
    build: &str,
    state: &str,
) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "INSERT INTO service (kind, name, address, port, build, state, started_at, heartbeat_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now())
         ON CONFLICT (kind, name) DO UPDATE SET
             address      = EXCLUDED.address,
             port         = EXCLUDED.port,
             build        = EXCLUDED.build,
             state        = EXCLUDED.state,
             heartbeat_at = now()",
    )
    .bind(kind)
    .bind(name)
    .bind(address)
    .bind(port)
    .bind(build)
    .bind(state)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Set a service's state (`ready`, `draining`, `stopped`, ...). Returns false when the row is gone
/// or was never registered.
pub async fn set_state(pool: &PgPool, kind: &str, name: &str, state: &str) -> sqlx::Result<bool> {
    let done = sqlx::query("UPDATE service SET state = $3 WHERE kind = $1 AND name = $2")
        .bind(kind)
        .bind(name)
        .bind(state)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Every registered service, with its freshness at `max_age_secs`. Ordered so a human can read it.
pub async fn services(pool: &PgPool, max_age_secs: f64) -> sqlx::Result<Vec<ServiceRow>> {
    // The exact column list this query reads; named so the query and the mapping stay one thing.
    type Columns = (String, String, String, i32, String, String, bool, f64);
    let rows: Vec<Columns> = sqlx::query_as(
        "SELECT kind, name, address, port, build, state,
                heartbeat_at > now() - make_interval(secs => $1::float8) AS fresh,
                EXTRACT(epoch FROM (now() - heartbeat_at))::float8    AS age_secs
           FROM service
          ORDER BY kind, name",
    )
    .bind(max_age_secs)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(kind, name, address, port, build, state, fresh, age_secs)| ServiceRow {
                kind,
                name,
                address,
                port,
                build,
                state,
                fresh,
                age_secs,
            },
        )
        .collect())
}

/// Forget a service entirely (a removed entry, or a test cleaning up). Returns false when it was
/// not registered.
pub async fn delete(pool: &PgPool, kind: &str, name: &str) -> sqlx::Result<bool> {
    let done = sqlx::query("DELETE FROM service WHERE kind = $1 AND name = $2")
        .bind(kind)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
