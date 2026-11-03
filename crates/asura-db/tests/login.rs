//! M3 acceptance tests: the identity pair resolves, and nothing else does.
//!
//! These run against the live PostgreSQL named by `ASURA_DATABASE_URL` (the environment, or the
//! repository's `.env`). They are harness-only: no game client, no protocol. What they prove is the
//! decision the auth service will make, taken here where a failure is a test failure instead of an
//! opaque `007` in front of a player.
//!
//! With no database configured every test prints a skip line and returns.
//!
//! The rules under test, from `docs/03-protocol.md` section 4 and `docs/05-gw2-lessons.md` rule 1:
//!
//! 1. the pair is validated on **both** values -- a right portal user with a wrong session id is not
//!    close enough;
//! 2. a closed session never resolves, or a stolen session id would work forever;
//! 3. a character list is scoped to its account, in the query, not after it;
//! 4. the newest session wins and the older one is closed.
//!
//! Every account name is unique per run (it carries a fresh UUID), so a run that panics before its
//! cleanup leaves an inert row that no later run can mistake for its own.

use asura_db::login as db;
use asura_db::{PgPool, Uuid};

const MAP: i32 = 15;

async fn pool() -> Option<PgPool> {
    // One rule for finding the settings, in `asura_db::config`, shared with the services.
    let url = asura_db::config::database_url()?;
    Some(
        asura_db::placement::connect(&url, 4)
            .await
            .expect("connect pool"),
    )
}

fn skip() {
    println!("ASURA_DATABASE_URL not set; skipping live-database login test");
}

/// A unique account name for one assertion: two runs, or two tests, never collide.
fn account_name(label: &str) -> String {
    format!("m3-{label}-{}", Uuid::new_v4())
}

#[tokio::test]
async fn a_presented_pair_resolves_to_its_account() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let name = account_name("valid");
    let seeded = db::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &name,
        "Asura Test",
        MAP,
    )
    .await
    .expect("seed identity");

    let resolved = db::find_session(&pool, seeded.portal_user, seeded.session)
        .await
        .expect("query")
        .expect("the pair must resolve");

    assert_eq!(resolved.session, seeded.session);
    assert_eq!(resolved.account, seeded.account);
    assert_eq!(
        resolved.character, None,
        "nothing has been selected on a fresh session"
    );
    assert_ne!(
        seeded.account, seeded.portal_user,
        "the account is its own identity, not the portal user id"
    );

    db::remove_account(&pool, &name).await.expect("cleanup");
}

#[tokio::test]
async fn neither_half_of_the_pair_is_optional() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let name = account_name("halves");
    let seeded = db::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &name,
        "Asura Test",
        MAP,
    )
    .await
    .expect("seed identity");

    // Right portal user, wrong session id: what a stale or forged client presents.
    assert_eq!(
        db::find_session(&pool, seeded.portal_user, Uuid::new_v4())
            .await
            .expect("query"),
        None,
        "a wrong session id must not resolve"
    );

    // Right session id, wrong portal user: the same, from the other side.
    assert_eq!(
        db::find_session(&pool, Uuid::new_v4(), seeded.session)
            .await
            .expect("query"),
        None,
        "a wrong portal user must not resolve"
    );

    // Both wrong: the same answer, not a different one.
    assert_eq!(
        db::find_session(&pool, Uuid::new_v4(), Uuid::new_v4())
            .await
            .expect("query"),
        None
    );

    db::remove_account(&pool, &name).await.expect("cleanup");
}

#[tokio::test]
async fn a_closed_session_never_resolves_again() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let name = account_name("closed");
    let portal_user = Uuid::new_v4();
    let session = Uuid::new_v4();
    db::seed_identity(&pool, portal_user, session, &name, "Asura Test", MAP)
        .await
        .expect("seed identity");

    assert!(db::close_session(&pool, session).await.expect("close"));

    assert_eq!(
        db::find_session(&pool, portal_user, session)
            .await
            .expect("query"),
        None,
        "a closed session must not resolve: the pair is spent, not a permanent key"
    );

    // Closing twice is a no-op rather than an error, because a disconnect and a stale sweep can both
    // reach the same session.
    assert!(!db::close_session(&pool, session)
        .await
        .expect("close again"));

    // Re-seeding the same pair reopens it, which is how a developer resets a spent session.
    db::seed_identity(&pool, portal_user, session, &name, "Asura Test", MAP)
        .await
        .expect("reseed");
    assert!(
        db::find_session(&pool, portal_user, session)
            .await
            .expect("query")
            .is_some(),
        "re-seeding is the documented way back to a working pair"
    );

    db::remove_account(&pool, &name).await.expect("cleanup");
}

#[tokio::test]
async fn the_newest_session_wins_and_the_older_one_is_closed() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let name = account_name("newest");

    // Two live sessions on one account: the same account name resolves to the same account row, and
    // each seeding call brings its own portal user and session id, as a second client would.
    let older = db::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &name,
        "Asura Test",
        MAP,
    )
    .await
    .expect("seed the older session");
    let newer = db::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &name,
        "Asura Test",
        MAP,
    )
    .await
    .expect("seed the newer session");
    assert_eq!(older.account, newer.account, "one account, two sessions");

    assert!(
        db::find_session(&pool, older.portal_user, older.session)
            .await
            .expect("query")
            .is_some(),
        "both sessions resolve before the rule is applied"
    );

    let closed = db::close_other_sessions(&pool, newer.account, newer.session)
        .await
        .expect("close the others");
    assert_eq!(closed, 1, "exactly the older session is closed");

    assert!(
        db::find_session(&pool, newer.portal_user, newer.session)
            .await
            .expect("query")
            .is_some(),
        "the newest session resolves"
    );
    assert_eq!(
        db::find_session(&pool, older.portal_user, older.session)
            .await
            .expect("query"),
        None,
        "the older session does not"
    );

    db::remove_account(&pool, &name).await.expect("cleanup");
}

#[tokio::test]
async fn a_character_list_is_scoped_to_its_own_account() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let mine = account_name("chars-mine");
    let theirs = account_name("chars-theirs");

    // Two characters on my account, seeded under one name so they share the account row.
    let seeded = db::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &mine,
        "Asura One",
        MAP,
    )
    .await
    .expect("seed the first character");
    db::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &mine,
        "Asura Two",
        MAP,
    )
    .await
    .expect("seed the second character");

    // Another account, whose characters must never appear in my list.
    let other = db::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &theirs,
        "Someone Else",
        MAP,
    )
    .await
    .expect("seed the other account");

    let characters = db::characters(&pool, seeded.account)
        .await
        .expect("characters");
    let names: Vec<&str> = characters.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["Asura One", "Asura Two"], "ordered by name");
    assert!(
        characters.iter().all(|c| c.id != other.character),
        "another account's character must not leak into the list"
    );
    assert!(characters.iter().all(|c| c.map_id == MAP));

    let theirs_list = db::characters(&pool, other.account)
        .await
        .expect("characters");
    assert_eq!(theirs_list.len(), 1);
    assert_eq!(theirs_list[0].name, "Someone Else");

    db::remove_account(&pool, &mine).await.expect("cleanup");
    db::remove_account(&pool, &theirs).await.expect("cleanup");
}

#[tokio::test]
async fn selecting_a_character_records_it_on_the_live_session_only() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let name = account_name("select");
    let seeded = db::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &name,
        "Asura Test",
        MAP,
    )
    .await
    .expect("seed identity");

    assert!(
        db::select_character(&pool, seeded.session, seeded.character)
            .await
            .expect("select")
    );

    let resolved = db::find_session(&pool, seeded.portal_user, seeded.session)
        .await
        .expect("query")
        .expect("still resolves");
    assert_eq!(resolved.character, Some(seeded.character));

    // A closed session is not a place to record a selection.
    db::close_session(&pool, seeded.session)
        .await
        .expect("close");
    assert!(
        !db::select_character(&pool, seeded.session, seeded.character)
            .await
            .expect("select on a closed session")
    );

    db::remove_account(&pool, &name).await.expect("cleanup");
}

#[tokio::test]
async fn deleting_a_character_stops_the_session_naming_it() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let name = account_name("delete");
    let seeded = db::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &name,
        "Asura Test",
        MAP,
    )
    .await
    .expect("seed identity");

    assert!(
        db::select_character(&pool, seeded.session, seeded.character)
            .await
            .expect("select")
    );
    assert_eq!(
        db::find_session(&pool, seeded.portal_user, seeded.session)
            .await
            .expect("query")
            .expect("resolves")
            .character,
        Some(seeded.character)
    );

    assert!(
        db::delete_character(&pool, seeded.account, seeded.character)
            .await
            .expect("delete")
    );

    // The row is gone and the session no longer names it: a travel for this session cannot go out
    // for a character that does not exist.
    assert_eq!(
        db::find_session(&pool, seeded.portal_user, seeded.session)
            .await
            .expect("query")
            .expect("the session still resolves")
            .character,
        None,
        "the deleted character is not named by the session any more"
    );

    db::remove_account(&pool, &name).await.expect("cleanup");
}
