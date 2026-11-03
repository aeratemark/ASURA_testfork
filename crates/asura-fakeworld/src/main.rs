//! The fake-world harness: a world server with no simulation, used to prove placement.
//!
//! It registers itself in `world`, publishes `world_pool` capacity, takes concurrent reservations,
//! refuses when the pool is full, goes silent, and is reaped - the whole M2 acceptance run, against
//! the real PostgreSQL, no game client involved (`docs/roadmap.md`, M2; `docs/HANDOFF.md`).
//!
//! Run it with a database configured (environment or repo `.env`):
//!
//! ```text
//! cargo run -p asura-fakeworld -- scenario [--world 8001] [--capacity 8] [--tasks 9]
//! ```
//!
//! Exit code 0 means every check passed. It is a harness: a missing database is an error, not a
//! skipped test.

use asura_db::{placement as db, PgPool, Uuid};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const DEFAULT_WORLD_ID: i32 = 8001;
const DEFAULT_CAPACITY: i32 = 8;
const DEFAULT_TASKS: usize = DEFAULT_CAPACITY as usize + 1;

#[tokio::main]
async fn main() -> ExitCode {
    let mut world_id = DEFAULT_WORLD_ID;
    let mut capacity = DEFAULT_CAPACITY;
    let mut tasks = DEFAULT_TASKS;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "scenario" => {}
            "--world" => {
                world_id = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_WORLD_ID)
            }
            "--capacity" => {
                capacity = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_CAPACITY)
            }
            "--tasks" => {
                tasks = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_TASKS)
            }
            other => {
                eprintln!("asura-fakeworld: unknown argument {other:?}");
                eprintln!("usage: asura-fakeworld scenario [--world N] [--capacity N] [--tasks N]");
                return ExitCode::FAILURE;
            }
        }
    }

    if tasks <= capacity as usize {
        eprintln!(
            "asura-fakeworld: --tasks ({tasks}) must exceed --capacity ({capacity}) so at least one \
             reservation is refused"
        );
        return ExitCode::FAILURE;
    }

    let Some(url) = database_url() else {
        eprintln!(
            "asura-fakeworld: no database URL. Set ASURA_DATABASE_URL, or copy .env.example to .env \
             with real credentials."
        );
        return ExitCode::FAILURE;
    };

    // Two pools, so the reservation race is between what SQL sees as two separate clients, which is
    // the case the design must survive between processes (`docs/02-architecture.md`).
    let pool_a = match db::connect(&url, 4).await {
        Ok(pool) => pool,
        Err(error) => {
            eprintln!("asura-fakeworld: cannot connect: {error}");
            return ExitCode::FAILURE;
        }
    };
    let pool_b = match db::connect(&url, 4).await {
        Ok(pool) => pool,
        Err(error) => {
            eprintln!("asura-fakeworld: cannot connect: {error}");
            return ExitCode::FAILURE;
        }
    };

    match run_scenario(&pool_a, &pool_b, world_id, capacity, tasks).await {
        Ok(true) => {
            println!("PASS");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("FAIL");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("asura-fakeworld: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The whole M2 acceptance run end to end. Returns whether every check passed.
async fn run_scenario(
    pool_a: &PgPool,
    pool_b: &PgPool,
    world_id: i32,
    capacity: i32,
    tasks: usize,
) -> Result<bool, String> {
    let map_id = 15; // placeholder: "Lakeside County" in the game's numbering; any id works here.
    let mut ok = true;

    println!("fake world: id={world_id} map={map_id} capacity={capacity} tasks={tasks}");

    // --- registration --------------------------------------------------------------------------
    db::register_world(
        pool_a,
        world_id,
        "fake-world",
        "127.0.0.1",
        6113,
        "m2-test",
        &[(map_id, capacity)],
    )
    .await
    .map_err(|e| format!("register_world: {e}"))?;
    let counts = pool_state(pool_a, world_id, map_id).await?;
    ok &= check(
        "registered world, published pool capacity, counter starts empty",
        counts == Some((capacity, 0)),
    );

    db::heartbeat(pool_a, world_id, 0)
        .await
        .map_err(|e| format!("heartbeat: {e}"))?;
    ok &= check(
        "world reads fresh right after its heartbeat",
        db::world_is_fresh(pool_a, world_id, 60.0)
            .await
            .map_err(|e| format!("world_is_fresh: {e}"))?,
    );

    // --- the concurrent reservation ------------------------------------------------------------
    let mut set = tokio::task::JoinSet::new();
    for i in 0..tasks {
        let pool = if i % 2 == 0 {
            pool_a.clone()
        } else {
            pool_b.clone()
        };
        let session = Uuid::new_v4();
        set.spawn(async move { db::reserve(&pool, world_id, map_id, session).await });
    }

    let mut won = 0usize;
    let mut refused = 0usize;
    while let Some(joined) = set.join_next().await {
        let outcome = joined.map_err(|e| format!("reserve task panicked: {e}"))?;
        match outcome.map_err(|e| format!("reserve: {e}"))? {
            Some(_) => won += 1,
            None => refused += 1,
        }
    }
    println!("reservations: won={won} refused={refused}");
    ok &= check(
        "exactly capacity concurrent reservations won",
        won == capacity as usize,
    );
    ok &= check(
        "exactly tasks - capacity were refused",
        refused == tasks - capacity as usize,
    );

    let counts = pool_state(pool_a, world_id, map_id).await?;
    ok &= check(
        "leased ends at capacity",
        counts == Some((capacity, capacity)),
    );
    let ledger = db::lease_count(pool_a, world_id, map_id)
        .await
        .map_err(|e| format!("lease_count: {e}"))?;
    ok &= check(
        "the lease ledger matches the counter",
        ledger == capacity as i64,
    );

    // A refusal must not touch the ledger.
    let extra = db::reserve(pool_a, world_id, map_id, Uuid::new_v4())
        .await
        .map_err(|e| format!("reserve: {e}"))?;
    ok &= check("a reservation on a full pool is refused", extra.is_none());
    let ledger_after = db::lease_count(pool_a, world_id, map_id)
        .await
        .map_err(|e| format!("lease_count: {e}"))?;
    ok &= check(
        "the refusal left the ledger untouched",
        ledger_after == ledger,
    );

    // One release frees exactly one slot and one ledger row.
    // A random session owns nothing, so its release must be a no-op.
    let released = db::release(pool_a, Uuid::new_v4())
        .await
        .map_err(|e| format!("release: {e}"))?;
    ok &= check("releasing an unknown session is a no-op", !released);

    // --- the reaper ----------------------------------------------------------------------------
    db::backdate_heartbeat(pool_a, world_id, 300.0)
        .await
        .map_err(|e| format!("backdate_heartbeat: {e}"))?;
    ok &= check(
        "a silent world reads as stale",
        !db::world_is_fresh(pool_a, world_id, 60.0)
            .await
            .map_err(|e| format!("world_is_fresh: {e}"))?,
    );

    let report = db::reap_stale(pool_a, 60.0)
        .await
        .map_err(|e| format!("reap_stale: {e}"))?;
    println!(
        "reaped: leases_freed={} pools_touched={}",
        report.leases_freed, report.pools_touched
    );
    ok &= check(
        "the reaper freed every lease of the silent world",
        report.leases_freed == capacity as i64,
    );
    let counts = pool_state(pool_a, world_id, map_id).await?;
    ok &= check(
        "leased is back to zero after the reaper",
        counts == Some((capacity, 0)),
    );
    let ledger = db::lease_count(pool_a, world_id, map_id)
        .await
        .map_err(|e| format!("lease_count: {e}"))?;
    ok &= check("the ledger is empty after the reaper", ledger == 0);

    // --- reconcile -----------------------------------------------------------------------------
    db::register_world(
        pool_a,
        world_id,
        "fake-world",
        "127.0.0.1",
        6113,
        "m2-test",
        &[(map_id, capacity)],
    )
    .await
    .map_err(|e| format!("re-register: {e}"))?;
    db::reconcile_world(pool_a, world_id)
        .await
        .map_err(|e| format!("reconcile_world: {e}"))?;
    let counts = pool_state(pool_a, world_id, map_id).await?;
    ok &= check(
        "reconcile agrees with an empty ledger",
        counts == Some((capacity, 0)),
    );

    // --- hygiene -------------------------------------------------------------------------------
    db::remove_world(pool_a, world_id)
        .await
        .map_err(|e| format!("remove_world: {e}"))?;

    Ok(ok)
}

async fn pool_state(
    pool: &PgPool,
    world_id: i32,
    map_id: i32,
) -> Result<Option<(i32, i32)>, String> {
    db::pool_counts(pool, world_id, map_id)
        .await
        .map_err(|e| format!("pool_counts: {e}"))
}

fn check(name: &str, passed: bool) -> bool {
    if passed {
        println!("[ok]   {name}");
    } else {
        println!("[FAIL] {name}");
    }
    passed
}

/// The database URL: the environment first, then the repo's `.env` (walking up from the crate).
fn database_url() -> Option<String> {
    if let Ok(url) = std::env::var("ASURA_DATABASE_URL") {
        let trimmed = url.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    let candidates = [
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env"),
        PathBuf::from(".env"),
    ];
    for path in candidates {
        if let Some(url) = read_url_from(&path) {
            return Some(url);
        }
    }
    None
}

fn read_url_from(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some(pair) => pair,
            None => continue,
        };
        if key.trim() == "ASURA_DATABASE_URL" {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}
