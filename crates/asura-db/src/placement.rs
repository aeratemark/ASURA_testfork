//! Placement: registering a world, reserving a slot atomically, and keeping the lease ledger honest.
//!
//! Every statement that mutates `world`, `world_pool` or `lease` lives here. Callers (the auth
//! service, a world server, the fake-world harness) get functions whose names say what happens,
//! never SQL strings - the rule this crate exists to enforce (`src/lib.rs`).
//!
//! AzerothCore parallel: `realmlist` registration, the atomic realm/pool update, and the
//! `DatabaseWorkerPool`'s asynchronous write discipline. The shape is theirs; the vendor is
//! PostgreSQL (docs/adr/0008-database-vendor.md).
//!
//! The two invariants this module must never break:
//!
//! * **No oversell.** A reservation is one `UPDATE ... WHERE leased < capacity RETURNING`, so two
//!   processes cannot both take the last slot (`docs/02-architecture.md`, "Travel, end to end").
//! * **The ledger and the counter agree.** A lease row is inserted only after the counter increment
//!   won, and both are released or reaped in one transaction. `count(lease) <= leased <= capacity`
//!   therefore holds at every commit, which is what makes the reaper safe.

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};

/// How many database connections a service opens by default. Small on purpose: one world server is a
/// handful of hot statements, and the queue depth is a metric, not a reason for a larger pool.
pub const DEFAULT_MAX_CONNECTIONS: u32 = 8;

/// Open a pool. The caller owns it; the pool is the unit of "one program talks to the database".
pub async fn connect(database_url: &str, max_connections: u32) -> sqlx::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
}

/// A world server registers itself and publishes its pool capacity: what it hosts and how many
/// instances it has prebuilt per map.
///
/// This is the startup half of the registry (`docs/02-architecture.md`, "The registry"). Registering
/// is also the reset point after a restart: a world rebuilds its leases by asking the database, so
/// the previous process's lease rows are cleared and its counter reset - one transaction, so a crash
/// halfway never leaves the ledger and the counter disagreeing.
pub async fn register_world(
    pool: &PgPool,
    world_id: i32,
    name: &str,
    address: &str,
    port: i32,
    build: &str,
    capacities: &[(i32, i32)],
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO world (id, name, address, port, build, draining, population, heartbeat_at, started_at)
         VALUES ($1, $2, $3, $4, $5, false, 0, now(), now())
         ON CONFLICT (id) DO UPDATE SET
             name        = EXCLUDED.name,
             address     = EXCLUDED.address,
             port        = EXCLUDED.port,
             build       = EXCLUDED.build,
             draining    = false,
             population  = 0,
             heartbeat_at = now(),
             started_at  = now()",
    )
    .bind(world_id)
    .bind(name)
    .bind(address)
    .bind(port)
    .bind(build)
    .execute(&mut *tx)
    .await?;

    // A restarted world owns nothing yet: clear leftovers from the previous process before the
    // counter reset below, or the ledger would outlive its count.
    sqlx::query("DELETE FROM lease WHERE world_id = $1")
        .bind(world_id)
        .execute(&mut *tx)
        .await?;

    for (map_id, capacity) in capacities {
        sqlx::query(
            "INSERT INTO world_pool (world_id, map_id, capacity, leased)
             VALUES ($1, $2, $3, 0)
             ON CONFLICT (world_id, map_id) DO UPDATE SET
                 capacity = EXCLUDED.capacity,
                 leased   = 0",
        )
        .bind(world_id)
        .bind(map_id)
        .bind(capacity)
        .execute(&mut *tx)
        .await?;
    }

    // A world that restarts with fewer maps must not leave stale pool rows behind: placement asks
    // who publishes a map, so a leftover row would route a client to a world that does not serve it
    // (observed live: a world started with --maps 146 was still sent a login for map 148).
    let ids: Vec<i32> = capacities.iter().map(|(map_id, _)| *map_id).collect();
    sqlx::query("DELETE FROM world_pool WHERE world_id = $1 AND NOT (map_id = ANY($2))")
        .bind(world_id)
        .bind(&ids)
        .execute(&mut *tx)
        .await?;

    tx.commit().await
}

/// Publish pool capacity for **one** map a world serves - the on-demand half of [`register_world`].
///
/// `register_world` resets `leased` because it is a restart: the world owns nothing yet. This does
/// not touch `leased`, because a travel to the same map may already hold the seat and a capacity
/// publish must never reset the counter under it.
pub async fn publish_pool(
    pool: &PgPool,
    world_id: i32,
    map_id: i32,
    capacity: i32,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO world_pool (world_id, map_id, capacity, leased)
         VALUES ($1, $2, $3, 0)
         ON CONFLICT (world_id, map_id) DO UPDATE SET capacity = EXCLUDED.capacity",
    )
    .bind(world_id)
    .bind(map_id)
    .bind(capacity)
    .execute(pool)
    .await
    .map(|_| ())
}

/// A world says it is alive, and how many players it currently holds.
pub async fn heartbeat(pool: &PgPool, world_id: i32, population: i32) -> sqlx::Result<bool> {
    let done = sqlx::query("UPDATE world SET heartbeat_at = now(), population = $2 WHERE id = $1")
        .bind(world_id)
        .bind(population)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Mark a world draining (finish its players, accept nobody new) or clear the flag.
pub async fn set_draining(pool: &PgPool, world_id: i32, draining: bool) -> sqlx::Result<bool> {
    let done = sqlx::query("UPDATE world SET draining = $2 WHERE id = $1")
        .bind(world_id)
        .bind(draining)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Whether a world has been told to drain. What the world's own heartbeat reads to decide it should
/// finish its players and leave (S3); a world that is gone reads as `false` rather than an error, so
/// the heartbeat keeps running until the row is back.
pub async fn is_draining(pool: &PgPool, world_id: i32) -> sqlx::Result<bool> {
    let row: Option<(bool,)> = sqlx::query_as("SELECT draining FROM world WHERE id = $1")
        .bind(world_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(draining,)| draining).unwrap_or(false))
}

/// Backdate a world's heartbeat, pretending it stopped saying it was alive `seconds_ago` seconds ago.
///
/// Test-harness and reaper-test only. A real world goes silent by not calling [`heartbeat`]; this
/// exists so the reaper can be tested without sleeping through a heartbeat interval.
pub async fn backdate_heartbeat(
    pool: &PgPool,
    world_id: i32,
    seconds_ago: f64,
) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE world SET heartbeat_at = now() - make_interval(secs => $2::float8) WHERE id = $1",
    )
    .bind(world_id)
    .bind(seconds_ago)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Reserve one slot in `world`/`map` for `session_id`.
///
/// Returns `Ok(Some(world_id))` when a slot was won and recorded in the lease ledger, and
/// `Ok(None)` when the pool is full. `None` is the refusal signal: it is delivered *before* the
/// client is told to move, which is the one rule (`docs/02-architecture.md`).
///
/// The reservation is one atomic statement, so concurrent callers cannot oversell the last slot.
/// It runs first, before the session's previous lease is superseded: a refused travel must leave the
/// player's existing lease intact, because the player is still standing where it was.
pub async fn reserve(
    pool: &PgPool,
    world_id: i32,
    map_id: i32,
    session_id: uuid::Uuid,
) -> sqlx::Result<Option<i32>> {
    let mut tx = pool.begin().await?;

    let won: Option<i32> = sqlx::query_scalar(
        "UPDATE world_pool SET leased = leased + 1
          WHERE world_id = $1 AND map_id = $2 AND leased < capacity
          RETURNING world_id",
    )
    .bind(world_id)
    .bind(map_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(world_id) = won else {
        // Nothing was taken, so there is nothing to undo. Commit the empty transaction (one round
        // trip) and report the refusal.
        tx.commit().await?;
        return Ok(None);
    };

    // The newest reservation is the one that counts: this session's earlier lease, if any, is
    // released in the same transaction (docs/05-gw2-lessons.md, rule 1).
    let previous: Vec<(i32, i32)> =
        sqlx::query_as("DELETE FROM lease WHERE session_id = $1 RETURNING world_id, map_id")
            .bind(session_id)
            .fetch_all(&mut *tx)
            .await?;
    for (old_world, old_map) in previous {
        decrement_lease(&mut tx, old_world, old_map).await?;
    }

    sqlx::query("INSERT INTO lease (world_id, map_id, session_id) VALUES ($1, $2, $3)")
        .bind(world_id)
        .bind(map_id)
        .bind(session_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(Some(world_id))
}

/// Release whatever lease `session_id` holds (the player disconnected, left the instance, or was
/// superseded). Returns whether a lease was found; a release for an unknown session is a harmless
/// no-op, which is what makes "the older session's release is a no-op" true by construction.
pub async fn release(pool: &PgPool, session_id: uuid::Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let rows: Vec<(i32, i32)> =
        sqlx::query_as("DELETE FROM lease WHERE session_id = $1 RETURNING world_id, map_id")
            .bind(session_id)
            .fetch_all(&mut *tx)
            .await?;
    for (world_id, map_id) in &rows {
        decrement_lease(&mut tx, *world_id, *map_id).await?;
    }
    tx.commit().await?;
    Ok(!rows.is_empty())
}

/// Release every slot of every world that stopped heartbeating more than `max_age_secs` ago.
///
/// A dead world's leases are freed so a crash costs capacity only until the reaper notices, never
/// permanently (`docs/02-architecture.md`, "Failure behaviour"). One statement, so concurrent reapers
/// cannot double-free: the first transaction's `DELETE ... RETURNING` is the only one that sees the
/// rows.
pub async fn reap_stale(pool: &PgPool, max_age_secs: f64) -> sqlx::Result<ReapReport> {
    let row: (i64, i64) = sqlx::query_as(
        "WITH dead AS (
             DELETE FROM lease l
             USING world w
             WHERE l.world_id = w.id
               AND w.heartbeat_at < now() - make_interval(secs => $1::float8)
             RETURNING l.world_id, l.map_id
         ),
         agg AS (
             SELECT world_id, map_id, count(*)::bigint AS n
             FROM dead
             GROUP BY world_id, map_id
         ),
         upd AS (
             UPDATE world_pool wp
             SET leased = GREATEST((wp.leased - agg.n)::int, 0)
             FROM agg
             WHERE wp.world_id = agg.world_id AND wp.map_id = agg.map_id
             RETURNING 1
         )
         SELECT coalesce((SELECT sum(n) FROM agg), 0)::bigint,
                (SELECT count(*) FROM upd)::bigint",
    )
    .bind(max_age_secs)
    .fetch_one(pool)
    .await?;
    Ok(ReapReport {
        leases_freed: row.0,
        pools_touched: row.1,
    })
}

/// What a reaper pass did. `leases_freed` counts player leases; `pools_touched` counts
/// `world_pool` rows whose counter was corrected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReapReport {
    pub leases_freed: i64,
    pub pools_touched: i64,
}

/// Recompute `world_pool.leased` from the lease ledger for one world.
///
/// This is the reconciler from `docs/05-gw2-lessons.md`, rule 2: the world server is the truth about
/// what it has handed out, and the ledger is how it reports it. After a world (or the auth service)
/// restarts, the counter is rebuilt from the rows instead of being trusted blindly.
pub async fn reconcile_world(pool: &PgPool, world_id: i32) -> sqlx::Result<u64> {
    let done = sqlx::query(
        "UPDATE world_pool wp
         SET leased = coalesce((
             SELECT count(*)::int
             FROM lease l
             WHERE l.world_id = wp.world_id AND l.map_id = wp.map_id
         ), 0)
         WHERE wp.world_id = $1",
    )
    .bind(world_id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

/// `(capacity, leased)` for one pool row, or `None` when the world does not publish that map.
pub async fn pool_counts(
    pool: &PgPool,
    world_id: i32,
    map_id: i32,
) -> sqlx::Result<Option<(i32, i32)>> {
    sqlx::query_as("SELECT capacity, leased FROM world_pool WHERE world_id = $1 AND map_id = $2")
        .bind(world_id)
        .bind(map_id)
        .fetch_optional(pool)
        .await
}

/// How many leases the ledger holds for one pool row.
pub async fn lease_count(pool: &PgPool, world_id: i32, map_id: i32) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM lease WHERE world_id = $1 AND map_id = $2")
        .bind(world_id)
        .bind(map_id)
        .fetch_one(pool)
        .await
}

/// Whether a world said it was alive within `max_age_secs`. Placement reads this before offering a
/// world; the reaper uses the same threshold, so "too old to offer" and "too old to keep leases"
/// cannot disagree.
pub async fn world_is_fresh(pool: &PgPool, world_id: i32, max_age_secs: f64) -> sqlx::Result<bool> {
    let fresh: bool = sqlx::query_scalar(
        "SELECT heartbeat_at > now() - make_interval(secs => $2::float8)
         FROM world WHERE id = $1",
    )
    .bind(world_id)
    .bind(max_age_secs)
    .fetch_one(pool)
    .await?;
    Ok(fresh)
}

/// A world that could serve a map right now: what the auth service needs in order to answer a travel
/// request with `AUTH_SMSG_GAME_SERVER_INFO`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldCandidate {
    pub world_id: i32,
    pub name: String,
    /// The address as the world registered it. `asura-world` registers the address the *client* may
    /// dial, not the one it binds: a server behind a NAT listens on a private address and must
    /// publish the public one, exactly as a realmlist does.
    pub address: String,
    pub port: i32,
    pub map_id: i32,
    pub leased: i32,
    pub capacity: i32,
}

/// Where a world server can be reached: what it registered, and whether it is still saying it is alive.
///
/// The auth service needs this for two different jobs, which is why it is not folded into
/// [`find_worlds`]: offering a world to a client is a *placement* question (is there a pool, is there
/// room), while routing the client's game connection is a *routing* question (which world did it
/// name, and where does that world actually live). A world can answer the second while being ineligible
/// for the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldEndpoint {
    pub world_id: i32,
    pub name: String,
    pub address: String,
    pub port: i32,
    pub fresh: bool,
}

/// The world with this id, if it was ever registered.
pub async fn world_endpoint(
    pool: &PgPool,
    world_id: i32,
    max_age_secs: f64,
) -> sqlx::Result<Option<WorldEndpoint>> {
    let row: Option<(i32, String, String, i32, bool)> = sqlx::query_as(
        "SELECT id, name, address, port,
                heartbeat_at > now() - make_interval(secs => $2::float8)
           FROM world WHERE id = $1",
    )
    .bind(world_id)
    .bind(max_age_secs)
    .fetch_optional(pool)
    .await?;
    Ok(
        row.map(|(world_id, name, address, port, fresh)| WorldEndpoint {
            world_id,
            name,
            address,
            port,
            fresh,
        }),
    )
}

/// Every world that could serve `map_id` right now, best first.
///
/// "Could" is deliberately five conditions, and each one is a way a naive lookup goes wrong:
///
/// * the world publishes the map at all (a world that does not host map 146 cannot serve a client
///   asking for it, and must not be offered as if it could);
/// * it is not draining (a restart in progress takes no new players);
/// * it said it was alive within `max_age_secs`, by the same threshold the reaper uses, so "too old
///   to offer" and "too old to keep leases" cannot disagree;
/// * it runs the **newest build** hosting this map (S3): the rolling deploy's rule is that the
///   newest build serves new arrivals and reconnects, and the old build serves only the players
///   already inside it. "Newest" is the build of the most recently started fresh world hosting the
///   map - draining or not, so once a newer build exists the older ones are never offered again,
///   even if the newest is full or itself draining (a refusal, never a fallback). This also means a
///   restart of an old build cannot masquerade as new: the registry records `started_at` per start;
/// * its counter says there is room. The count is a *hint* - only [`reserve`] can actually win a slot -
///   but offering a full world would make every travel a round trip that ends in a refusal.
///
/// Several candidates come back, not one, because the caller has to *probe* the port: a world can be
/// fresh in the database and dead on the socket, and the one thing this service must never do is hand
/// a client an address nothing is listening on. The caller walks the list until one answers.
pub async fn find_worlds(
    pool: &PgPool,
    map_id: i32,
    max_age_secs: f64,
    limit: i64,
) -> sqlx::Result<Vec<WorldCandidate>> {
    let rows: Vec<(i32, String, String, i32, i32, i32, i32)> = sqlx::query_as(
        "WITH hosting AS (
             SELECT w.id, w.build, w.started_at
               FROM world w
               JOIN world_pool p ON p.world_id = w.id
              WHERE p.map_id = $1
                AND w.heartbeat_at > now() - make_interval(secs => $2::float8)
         ), newest AS (
             SELECT build FROM hosting ORDER BY started_at DESC LIMIT 1
         )
         SELECT w.id, w.name, w.address, w.port, p.map_id, p.leased, p.capacity
           FROM world w
           JOIN world_pool p ON p.world_id = w.id
          WHERE p.map_id = $1
            AND NOT w.draining
            AND w.heartbeat_at > now() - make_interval(secs => $2::float8)
            AND w.build = (SELECT build FROM newest)
            AND p.leased < p.capacity
          ORDER BY p.leased ASC, w.id ASC
          LIMIT $3",
    )
    .bind(map_id)
    .bind(max_age_secs)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(world_id, name, address, port, map_id, leased, capacity)| WorldCandidate {
                world_id,
                name,
                address,
                port,
                map_id,
                leased,
                capacity,
            },
        )
        .collect())
}

/// Record which world a login session was sent to. Best effort: the row is a routing hint, and a
/// session that could not be updated must not fail a travel the client has already been told about.
pub async fn note_session_world(
    pool: &PgPool,
    session_id: uuid::Uuid,
    world_id: i32,
    map_id: i32,
) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE session SET world_id = $2, map_id = $3
          WHERE id = $1 AND ended_at IS NULL",
    )
    .bind(session_id)
    .bind(world_id)
    .bind(map_id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// A pending travel: what the auth service issued, and what a world server must match before it
/// believes a connecting client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    pub player_token: i64,
    pub world_id: i32,
    pub map_id: i32,
    pub session_id: uuid::Uuid,
    pub account_id: uuid::Uuid,
    pub character_id: uuid::Uuid,
}

/// Write the pending transfer for a travel that is about to be announced.
///
/// Called *before* the reply goes out, so a client that reconnects the instant it is told where to go
/// finds the row already there. The token is the auth service's random `u32`; the row is what makes
/// that number mean something to the world server.
pub async fn begin_transfer(
    pool: &PgPool,
    player_token: u32,
    world_id: i32,
    map_id: i32,
    session_id: uuid::Uuid,
    account_id: uuid::Uuid,
    character_id: uuid::Uuid,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO transfer
             (player_token, world_id, map_id, session_id, account_id, character_id, created_at,
              claimed_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), NULL)",
    )
    .bind(i64::from(player_token))
    .bind(world_id)
    .bind(map_id)
    .bind(session_id)
    .bind(account_id)
    .bind(character_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Claim the pending transfer a connecting client presented, and return it.
///
/// **The newest connection wins** (`docs/05-gw2-lessons.md` rule 1: only ever honour the most recent
/// connection, even if the older one looks alive and has work in flight). The claim therefore does not
/// skip a token that was already claimed: build 39112 opens its game connection again with the same
/// token after the first one has carried the load, that second connection is the newer one, and
/// refusing it is what produced the client's `007` on 2026-10-28. Measured in this server's own world
/// log: the token was claimed, the character joined the map, and then a second connection presenting
/// the same token was refused as "no pending transfer".
///
/// `None` means one of
///
/// * no such token for this world,
/// * the token is for another world.
pub async fn claim_transfer(
    pool: &PgPool,
    world_id: i32,
    player_token: u32,
) -> sqlx::Result<Option<Transfer>> {
    let row: Option<(i64, i32, i32, uuid::Uuid, uuid::Uuid, uuid::Uuid)> = sqlx::query_as(
        "UPDATE transfer SET claimed_at = now()
          WHERE player_token = $1 AND world_id = $2
          RETURNING player_token, world_id, map_id, session_id, account_id, character_id",
    )
    .bind(i64::from(player_token))
    .bind(world_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(
        |(player_token, world_id, map_id, session_id, account_id, character_id)| Transfer {
            player_token,
            world_id,
            map_id,
            session_id,
            account_id,
            character_id,
        },
    ))
}

/// Forget a transfer nobody claimed. Used when a travel is refused after the row was written, and by
/// the tests; a stale pending row would let a token be used long after the attempt it belonged to.
pub async fn cancel_transfer(pool: &PgPool, player_token: u32) -> sqlx::Result<bool> {
    let done = sqlx::query("DELETE FROM transfer WHERE player_token = $1 AND claimed_at IS NULL")
        .bind(i64::from(player_token))
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Test support only: write a pool counter value directly, to simulate the drift a crash can leave
/// behind (counter and ledger disagree). The reconciler is the code that repairs such drift; this is
/// how a test creates the fault it repairs. No service calls this.
pub async fn force_counter(
    pool: &PgPool,
    world_id: i32,
    map_id: i32,
    leased: i32,
) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE world_pool SET leased = $3
          WHERE world_id = $1 AND map_id = $2 AND $3 >= 0 AND $3 <= capacity",
    )
    .bind(world_id)
    .bind(map_id)
    .bind(leased)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Remove a world's registry row and everything that hangs off it (`world_pool`, `lease` rows
/// cascade). Harness and test hygiene: a scenario leaves no row behind that the next run would
/// mistake for a live world.
pub async fn remove_world(pool: &PgPool, world_id: i32) -> sqlx::Result<bool> {
    let done = sqlx::query("DELETE FROM world WHERE id = $1")
        .bind(world_id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// The one counter decrement shared by release and re-reserve, kept in one place so it cannot drift
/// from the increment above. `leased > 0` guards against a ledger row outliving its count.
async fn decrement_lease(
    tx: &mut Transaction<'_, Postgres>,
    world_id: i32,
    map_id: i32,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE world_pool SET leased = leased - 1
          WHERE world_id = $1 AND map_id = $2 AND leased > 0",
    )
    .bind(world_id)
    .bind(map_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
