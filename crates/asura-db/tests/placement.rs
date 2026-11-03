//! M2 acceptance tests: placement is safe under concurrency, and the reaper bounds a dead world.
//!
//! These tests run against the live PostgreSQL named by `ASURA_DATABASE_URL` (the environment, or the
//! repository's `.env`). They are harness-only: no game client, no protocol, no simulation - the point
//! of M2 is that placement can be trusted before anything client-facing exists (`docs/roadmap.md`).
//!
//! With no database configured a test prints a skip line and returns; `tools/m2-verify.ps1` is the
//! one-command way to run them for real.

use asura_db::{placement as db, PgPool, Uuid};

const MAP: i32 = 15; // a placeholder map id; the reservations do not care which map it is
const STALE_AFTER_SECS: f64 = 60.0;

/// Two pools over the same database: the reservation race is between what SQL sees as two
/// separate clients, which is the case the design must survive between processes.
async fn pools() -> Option<(PgPool, PgPool)> {
    // The URL rule lives in one place (`asura_db::config`), so a test cannot disagree with a service
    // about where the settings come from.
    let url = asura_db::config::database_url()?;
    let a = db::connect(&url, 4).await.expect("connect pool a");
    let b = db::connect(&url, 4).await.expect("connect pool b");
    Some((a, b))
}

fn skip() {
    println!("ASURA_DATABASE_URL not set; skipping live-database placement test");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reservations_never_oversell() {
    let Some((pool_a, pool_b)) = pools().await else {
        skip();
        return;
    };

    // capacity N-1, N concurrent reservations: exactly N-1 win. Run it repeatedly: a race that passes
    // once is not proven.
    for round in 0..5 {
        let world_id = 9100 + round;
        let capacity = 4;
        let tasks = 5;

        db::register_world(
            &pool_a,
            world_id,
            "race-world",
            "127.0.0.1",
            6200,
            "m2-test",
            &[(MAP, capacity)],
        )
        .await
        .expect("register");

        let mut set = tokio::task::JoinSet::new();
        let mut sessions = Vec::with_capacity(tasks as usize);
        for i in 0..tasks {
            let pool = if i % 2 == 0 {
                pool_a.clone()
            } else {
                pool_b.clone()
            };
            let session = Uuid::new_v4();
            sessions.push(session);
            set.spawn(async move { db::reserve(&pool, world_id, MAP, session).await });
        }

        let mut won = 0usize;
        let mut refused = 0usize;
        while let Some(joined) = set.join_next().await {
            let outcome = joined.expect("reservation task panicked");
            match outcome.expect("reserve failed") {
                Some(_) => won += 1,
                None => refused += 1,
            }
        }

        assert_eq!(
            won, capacity as usize,
            "round {round}: exactly N-1 reservations win"
        );
        assert_eq!(
            refused,
            tasks as usize - capacity as usize,
            "round {round}: the rest are refused"
        );

        let counts = db::pool_counts(&pool_a, world_id, MAP)
            .await
            .expect("pool_counts")
            .expect("pool row");
        assert_eq!(
            counts,
            (capacity, capacity),
            "round {round}: leased ends at capacity"
        );

        let ledger = db::lease_count(&pool_a, world_id, MAP)
            .await
            .expect("lease_count");
        assert_eq!(
            ledger, capacity as i64,
            "round {round}: one ledger row per lease"
        );

        // Release every winner: the pool empties again.
        for session in &sessions {
            db::release(&pool_a, *session).await.expect("release");
        }
        let counts = db::pool_counts(&pool_a, world_id, MAP)
            .await
            .expect("pool_counts")
            .expect("pool row");
        assert_eq!(
            counts,
            (capacity, 0),
            "round {round}: releases empty the pool"
        );
        let ledger = db::lease_count(&pool_a, world_id, MAP)
            .await
            .expect("lease_count");
        assert_eq!(ledger, 0, "round {round}: releases empty the ledger");

        db::remove_world(&pool_a, world_id)
            .await
            .expect("remove_world");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_full_pool_refuses_without_disturbing_the_holder() {
    let Some((pool_a, _pool_b)) = pools().await else {
        skip();
        return;
    };
    let world_id = 9200;

    db::register_world(
        &pool_a,
        world_id,
        "full-world",
        "127.0.0.1",
        6201,
        "m2-test",
        &[(MAP, 1)],
    )
    .await
    .expect("register");

    let holder = Uuid::new_v4();
    let outsider = Uuid::new_v4();

    let first = db::reserve(&pool_a, world_id, MAP, holder)
        .await
        .expect("reserve holder");
    assert!(first.is_some(), "the holder wins the only slot");

    // The refusal signal: None, before the client would be told to move (the one rule).
    let second = db::reserve(&pool_a, world_id, MAP, outsider)
        .await
        .expect("reserve outsider");
    assert!(second.is_none(), "a full pool refuses");

    // A refused travel never takes away the lease the player already holds.
    let counts = db::pool_counts(&pool_a, world_id, MAP)
        .await
        .expect("pool_counts")
        .expect("pool row");
    assert_eq!(counts, (1, 1), "the holder's lease survives the refusal");
    let ledger = db::lease_count(&pool_a, world_id, MAP)
        .await
        .expect("lease_count");
    assert_eq!(ledger, 1, "the refusal did not touch the ledger");

    assert!(
        db::release(&pool_a, holder).await.expect("release holder"),
        "the holder releases its lease"
    );
    assert!(
        !db::release(&pool_a, outsider)
            .await
            .expect("release outsider"),
        "a session that never held a lease releases nothing"
    );
    let counts = db::pool_counts(&pool_a, world_id, MAP)
        .await
        .expect("pool_counts")
        .expect("pool row");
    assert_eq!(counts, (1, 0), "the slot is free again");

    db::remove_world(&pool_a, world_id)
        .await
        .expect("remove_world");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_reaper_frees_a_silent_world_but_not_a_fresh_one() {
    let Some((pool_a, _pool_b)) = pools().await else {
        skip();
        return;
    };
    let silent = 9300;
    let fresh = 9301;

    db::register_world(
        &pool_a,
        silent,
        "silent-world",
        "127.0.0.1",
        6202,
        "m2-test",
        &[(MAP, 2)],
    )
    .await
    .expect("register silent");
    db::register_world(
        &pool_a,
        fresh,
        "fresh-world",
        "127.0.0.1",
        6203,
        "m2-test",
        &[(MAP, 2)],
    )
    .await
    .expect("register fresh");

    // The silent world takes two players and then stops saying it is alive.
    let s1 = Uuid::new_v4();
    let s2 = Uuid::new_v4();
    let s3 = Uuid::new_v4();
    assert!(db::reserve(&pool_a, silent, MAP, s1)
        .await
        .expect("reserve")
        .is_some());
    assert!(db::reserve(&pool_a, silent, MAP, s2)
        .await
        .expect("reserve")
        .is_some());
    // The fresh world keeps heartbeating and holds one player.
    assert!(db::reserve(&pool_a, fresh, MAP, s3)
        .await
        .expect("reserve")
        .is_some());
    db::backdate_heartbeat(&pool_a, silent, 300.0)
        .await
        .expect("backdate");

    assert!(
        !db::world_is_fresh(&pool_a, silent, STALE_AFTER_SECS)
            .await
            .expect("world_is_fresh"),
        "the silent world reads as stale"
    );
    assert!(
        db::world_is_fresh(&pool_a, fresh, STALE_AFTER_SECS)
            .await
            .expect("world_is_fresh"),
        "the fresh world still reads as alive"
    );

    let report = db::reap_stale(&pool_a, STALE_AFTER_SECS)
        .await
        .expect("reap_stale");
    // Not an exact count: the reaper is global by design, and a development database may hold other
    // worlds that went silent (an earlier run that stopped halfway through, a service that was
    // killed). Asserting `== 2` here made this test fail on litter rather than on behaviour, so the
    // claim is "at least this world's two, and this world's own state is what the next assertions
    // check".
    assert!(
        report.leases_freed >= 2,
        "the silent world's two leases are freed (report: {})",
        report.leases_freed
    );

    let counts = db::pool_counts(&pool_a, silent, MAP)
        .await
        .expect("pool_counts")
        .expect("pool row");
    assert_eq!(counts, (2, 0), "the silent world's counter is back to zero");
    let ledger = db::lease_count(&pool_a, silent, MAP)
        .await
        .expect("lease_count");
    assert_eq!(ledger, 0, "the silent world's ledger is empty");

    // The fresh world is untouched.
    let counts = db::pool_counts(&pool_a, fresh, MAP)
        .await
        .expect("pool_counts")
        .expect("pool row");
    assert_eq!(counts, (2, 1), "a fresh world keeps its lease");
    assert!(
        db::release(&pool_a, s3).await.expect("release s3"),
        "the fresh world's session still owns its lease"
    );

    db::remove_world(&pool_a, silent)
        .await
        .expect("remove_world");
    db::remove_world(&pool_a, fresh)
        .await
        .expect("remove_world");
}

#[tokio::test]
async fn reconcile_rebuilds_the_counter_from_the_ledger() {
    let Some((pool_a, _pool_b)) = pools().await else {
        skip();
        return;
    };
    let world_id = 9400;

    db::register_world(
        &pool_a,
        world_id,
        "drift-world",
        "127.0.0.1",
        6204,
        "m2-test",
        &[(MAP, 2)],
    )
    .await
    .expect("register");

    let s1 = Uuid::new_v4();
    let s2 = Uuid::new_v4();
    assert!(db::reserve(&pool_a, world_id, MAP, s1)
        .await
        .expect("reserve")
        .is_some());
    assert!(db::reserve(&pool_a, world_id, MAP, s2)
        .await
        .expect("reserve")
        .is_some());

    // Simulate the drift a crash can leave: the counter disagrees with the ledger.
    assert!(
        db::force_counter(&pool_a, world_id, MAP, 0)
            .await
            .expect("force_counter"),
        "the forced drift is applied"
    );

    // The reconciler rebuilds the counter from the ledger, not the other way round.
    db::reconcile_world(&pool_a, world_id)
        .await
        .expect("reconcile_world");
    let counts = db::pool_counts(&pool_a, world_id, MAP)
        .await
        .expect("pool_counts")
        .expect("pool row");
    assert_eq!(
        counts,
        (2, 2),
        "reconcile restores the counter from the ledger"
    );

    // A release after the rebuild keeps them consistent.
    assert!(db::release(&pool_a, s1).await.expect("release s1"));
    db::reconcile_world(&pool_a, world_id)
        .await
        .expect("reconcile_world");
    let counts = db::pool_counts(&pool_a, world_id, MAP)
        .await
        .expect("pool_counts")
        .expect("pool row");
    assert_eq!(counts, (2, 1), "reconcile stays consistent after a release");

    db::remove_world(&pool_a, world_id)
        .await
        .expect("remove_world");
}

// ---------------------------------------------------------------------------------------------
// S3: the newest build is the only one offered.
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_newest_build_is_the_only_one_offered() {
    let Some((pool, _pool_b)) = pools().await else {
        skip();
        return;
    };
    // A map of its own: the other live tests share MAP, and "the newest build hosting the map" would
    // then be whichever test registered last (they run in parallel). The world ids avoid the other
    // tests' ranges too (9100-, 9200, 9300-, 9400).
    const NEWEST_MAP: i32 = 16;
    let old_world = 9500;
    let new_world = 9501;

    // Two builds hosting the same map; the new one started later (backdated old one, so the test
    // cannot depend on how fast two registrations run).
    db::register_world(
        &pool,
        old_world,
        "old-build",
        "127.0.0.1",
        6206,
        "build-old",
        &[(NEWEST_MAP, 1)],
    )
    .await
    .expect("register old");
    db::register_world(
        &pool,
        new_world,
        "new-build",
        "127.0.0.1",
        6207,
        "build-new",
        &[(NEWEST_MAP, 1)],
    )
    .await
    .expect("register new");
    sqlx::query("UPDATE world SET started_at = now() - interval '1 minute' WHERE id = $1")
        .bind(old_world)
        .execute(&pool)
        .await
        .expect("backdate the old build");

    // The newest build is the only one offered: the old build serves only the players already
    // inside it, never a new placement.
    let offered = db::find_worlds(&pool, NEWEST_MAP, STALE_AFTER_SECS, 10)
        .await
        .expect("find_worlds");
    assert_eq!(
        offered
            .iter()
            .map(|candidate| candidate.world_id)
            .collect::<Vec<_>>(),
        vec![new_world],
        "only the newest build is offered: {offered:?}"
    );

    // A full newest build is a refusal, not a fallback to the old one: the one rule.
    let session = Uuid::new_v4();
    assert!(db::reserve(&pool, new_world, NEWEST_MAP, session)
        .await
        .expect("reserve")
        .is_some());
    let offered = db::find_worlds(&pool, NEWEST_MAP, STALE_AFTER_SECS, 10)
        .await
        .expect("find_worlds");
    assert!(
        offered.is_empty(),
        "a full newest build refuses; it does not fall back to the old build: {offered:?}"
    );

    // Draining the newest build does not bring the old one back either.
    db::set_draining(&pool, new_world, true)
        .await
        .expect("drain the new build");
    let offered = db::find_worlds(&pool, NEWEST_MAP, STALE_AFTER_SECS, 10)
        .await
        .expect("find_worlds");
    assert!(
        offered.is_empty(),
        "no fallback to an older build once a newer one exists: {offered:?}"
    );

    db::remove_world(&pool, old_world).await.ok();
    db::remove_world(&pool, new_world).await.ok();
}
