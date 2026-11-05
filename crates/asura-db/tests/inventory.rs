//! M5 acceptance tests: a character's inventory and position survive a session.
//!
//! These run against the live PostgreSQL named by `ASURA_DATABASE_URL` (the environment, or the
//! repository's `.env`). Harness only: no game client, no sockets. What is proved is what the world
//! server asks the database for, and what it writes back:
//!
//! 1. a character with no bags is given the reference's default inventory -- six bags, nine items --
//!    and the *shape* of it is the capture's: the backpack carries its own item, two cupcakes sit in
//!    slots 0 and 1, and the equipment is in the equipped-items bag;
//! 2. seeding twice does nothing the second time, which is what makes it safe to run on every session
//!    (the reference's own `IsValidBag` guard);
//! 3. what is written reads back identical -- including a name that is not a string and a modifier list;
//! 4. a drop finds an item only in the bags the reference searches, and never in storage;
//! 5. a position written for a character reads back, and a character that has never been saved has no
//!    position at all rather than a coordinate at the origin.
//!
//! Every account name is unique per run (it carries a fresh UUID), so a run that panics before its
//! cleanup leaves an inert row that no later run can mistake for its own.

use asura_db::game as db_game;
use asura_db::inventory as db;
use asura_db::{PgPool, Uuid};

const MAP: i32 = 146;

async fn pool() -> Option<PgPool> {
    let url = asura_db::config::database_url()?;
    Some(
        asura_db::placement::connect(&url, 4)
            .await
            .expect("connect pool"),
    )
}

fn skip() {
    println!("ASURA_DATABASE_URL not set; skipping live-database inventory test");
}

/// One account with one character, named uniquely for this run.
async fn character(pool: &PgPool, label: &str) -> (String, Uuid) {
    let name = format!("m5-{label}-{}", Uuid::new_v4());
    let seeded = asura_db::login::seed_identity(
        pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &name,
        "Asura Inventory",
        MAP,
    )
    .await
    .expect("seed identity");
    (name, seeded.character)
}

#[tokio::test]
async fn a_new_character_is_seeded_with_the_default_inventory() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let (name, character) = character(&pool, "seed").await;

    // Nothing to read before it is seeded, and that is what tells the world server to seed.
    let before = db::load(&pool, character).await.expect("load");
    assert!(before.is_empty(), "a fresh character has no inventory");

    assert!(
        db::seed_default(&pool, character).await.expect("seed"),
        "seeding a character with no bags writes them"
    );

    let inventory = db::load(&pool, character).await.expect("load");
    assert_eq!(inventory.bags.len(), 6, "the reference's six bags");
    assert_eq!(inventory.items.len(), 9, "and its nine items");

    // Bags come back in the reference's send order, which is by model id and not by id.
    let models: Vec<i32> = inventory.bags.iter().map(|bag| bag.bag_model_id).collect();
    assert_eq!(models, vec![0, 5, 6, 7, 8, 21]);

    // The backpack's own item is a row with no slot, and the bag names it.
    let backpack = &inventory.bags[0];
    assert_eq!(backpack.bag_id, 1, "the backpack");
    let own = inventory
        .item(backpack.assoc_item_id)
        .expect("the item that is the backpack");
    assert_eq!((own.bag_id, own.slot), (None, None));
    assert_eq!(own.name_units(), vec![0x21A8, 0xD157, 0xB58F, 0x166F]);
    assert_eq!(own.modifier_dwords(), vec![0x2448_1400]);

    // Two cupcakes, drawn from the sequence, in the backpack's first two slots.
    let cupcakes = inventory.items_in(1);
    assert_eq!(cupcakes.len(), 2);
    assert_eq!(cupcakes[0].slot, Some(0));
    assert_eq!(cupcakes[1].slot, Some(1));
    assert!(cupcakes.iter().all(|item| item.quantity == 100));
    assert!(cupcakes.iter().all(|item| item.file_id == 0x47C34));

    // The equipped-items bag holds the armour, and the ids the client is told are its own.
    let worn = inventory.equipped_item_ids();
    assert_ne!(worn, [0; 9], "the character is dressed");
    assert_eq!(worn[1], 0, "and slot 1 is the off hand, which is empty");

    // Seeding again is a no-op: every session calls it, and the reference guards it the same way.
    assert!(
        !db::seed_default(&pool, character).await.expect("seed"),
        "a character that has bags is left alone"
    );
    let again = db::load(&pool, character).await.expect("load");
    assert_eq!(again, inventory, "and its inventory did not change");

    asura_db::login::remove_account(&pool, &name)
        .await
        .expect("clean up");
}

#[tokio::test]
async fn two_characters_do_not_share_item_ids() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let (first_name, first) = character(&pool, "ids-a").await;
    let (second_name, second) = character(&pool, "ids-b").await;
    db::seed_default(&pool, first).await.expect("seed");
    db::seed_default(&pool, second).await.expect("seed");

    let a = db::load(&pool, first).await.expect("load");
    let b = db::load(&pool, second).await.expect("load");

    let ids = |inventory: &db::Inventory| -> Vec<i32> {
        inventory.items.iter().map(|item| item.id).collect()
    };
    let (first_ids, second_ids) = (ids(&a), ids(&b));
    assert_eq!(first_ids.len(), 9);
    assert_eq!(second_ids.len(), 9);
    for id in &first_ids {
        assert!(
            !second_ids.contains(id),
            "item {id} belongs to two characters and the client would see one of them"
        );
    }
    assert_eq!(
        a.items.len(),
        b.items.len(),
        "and both were given the whole set"
    );

    asura_db::login::remove_account(&pool, &first_name)
        .await
        .expect("clean up");
    asura_db::login::remove_account(&pool, &second_name)
        .await
        .expect("clean up");
}

#[tokio::test]
async fn a_drop_finds_only_what_the_reference_would_find() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let (name, character) = character(&pool, "drop").await;
    db::seed_default(&pool, character).await.expect("seed");
    let inventory = db::load(&pool, character).await.expect("load");

    // A cupcake is in the backpack, which is one of the four bags `GmBag_FindItem` searches.
    let cupcake = inventory.items_in(1)[0].clone();
    let found = db::find_in_carrying_bags(&pool, character, cupcake.id)
        .await
        .expect("find");
    assert_eq!(found.as_ref(), Some(&cupcake), "the cupcake is carried");

    // The backpack's own item is not in a slot, so it is not something a drop can name.
    let own = inventory.bags[0].assoc_item_id;
    assert_eq!(
        db::find_in_carrying_bags(&pool, character, own)
            .await
            .expect("find"),
        None,
        "a bag is not an item in a slot"
    );

    // An item in storage is not droppable: the reference looks at the backpack and bags 1 and 2 only.
    let storage_bag = inventory
        .bags
        .iter()
        .find(|bag| bag.bag_model_id == 7)
        .expect("storage 1");
    let stored: i32 = sqlx::query_scalar(
        "INSERT INTO t_item (character_id, bag_id, slot, file_id, item_type, model_id, quantity)
         VALUES ($1, $2, 0, 0x47C34, 9, 22269, 1) RETURNING id",
    )
    .bind(character)
    .bind(storage_bag.bag_id)
    .fetch_one(&pool)
    .await
    .expect("write a stored item");
    assert_eq!(
        db::find_in_carrying_bags(&pool, character, stored)
            .await
            .expect("find"),
        None,
        "storage is not a bag a drop comes from, in the reference or here"
    );

    // And an id that belongs to nobody finds nothing rather than failing.
    assert_eq!(
        db::find_in_carrying_bags(&pool, character, 0)
            .await
            .expect("find"),
        None
    );

    asura_db::login::remove_account(&pool, &name)
        .await
        .expect("clean up");
}

#[tokio::test]
async fn a_stack_can_be_split_and_emptied() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let (name, character) = character(&pool, "stack").await;
    db::seed_default(&pool, character).await.expect("seed");
    let inventory = db::load(&pool, character).await.expect("load");
    let cupcake = inventory.items_in(1)[0].clone();
    assert_eq!(cupcake.quantity, 100, "the reference's cupcake is a stack");

    // One out of the stack -- which is what a client asks for and what the reference refuses.
    let taken = db::take_from_stack(&pool, character, cupcake.id, 1)
        .await
        .expect("take")
        .expect("one of a hundred can be taken");
    assert_eq!(taken.remaining, 99);
    assert_eq!(taken.item.quantity, 100, "the item as it was");
    assert_eq!(taken.item.model_id, 22269, "and what it is drawn as");

    let after = db::load(&pool, character).await.expect("load");
    let left = after.item(cupcake.id).expect("still there");
    assert_eq!(left.quantity, 99);
    assert_eq!(left.slot, cupcake.slot, "in the slot it was in");

    // More than the stack is refused, and changes nothing.
    assert_eq!(
        db::take_from_stack(&pool, character, cupcake.id, 100)
            .await
            .expect("take"),
        None
    );

    // The rest empties the row.
    let rest = db::take_from_stack(&pool, character, cupcake.id, 99)
        .await
        .expect("take")
        .expect("the rest can be taken");
    assert_eq!(rest.remaining, 0);
    let emptied = db::load(&pool, character).await.expect("load");
    assert_eq!(emptied.item(cupcake.id), None, "the row went with it");

    // Nothing left: a second attempt finds nothing, and is not an error.
    assert_eq!(
        db::take_from_stack(&pool, character, cupcake.id, 1)
            .await
            .expect("take"),
        None
    );

    // A bag's own item is not litter: it has no slot, so it cannot be taken.
    let own = emptied.bags[0].assoc_item_id;
    assert_eq!(
        db::take_from_stack(&pool, character, own, 1)
            .await
            .expect("take"),
        None,
        "a bag is not a stack"
    );

    asura_db::login::remove_account(&pool, &name)
        .await
        .expect("clean up");
}

#[tokio::test]
async fn an_item_moves_between_slots_and_refuses_the_impossible_ones() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let (name, character) = character(&pool, "move").await;
    db::seed_default(&pool, character).await.expect("seed");
    let inventory = db::load(&pool, character).await.expect("load");
    let cupcake = inventory.items_in(1)[0].clone();
    let other = inventory.items_in(1)[1].clone();

    // Into a free slot.
    match db::move_item(&pool, character, cupcake.id, 1, 5)
        .await
        .expect("move")
    {
        db::MoveOutcome::At { bag_id, slot, item } => {
            assert_eq!((bag_id, slot), (1, 5));
            assert_eq!(item.slot, Some(5), "the row it returns is the moved one");
        }
        other => panic!("expected a move, got {other:?}"),
    }
    let moved = db::load(&pool, character).await.expect("load");
    assert_eq!(moved.item(cupcake.id).and_then(|item| item.slot), Some(5));

    // Onto an occupied slot: refused, and the item is handed back where it really is.
    match db::move_item(&pool, character, cupcake.id, 1, other.slot.unwrap())
        .await
        .expect("move")
    {
        db::MoveOutcome::Refused {
            item: Some(item),
            reason,
        } => {
            assert_eq!(reason, db::Refused::Occupied);
            assert_eq!(item.slot, Some(5), "where it actually is");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    // Where it already is: nothing written.
    assert!(matches!(
        db::move_item(&pool, character, cupcake.id, 1, 5)
            .await
            .expect("move"),
        db::MoveOutcome::Unchanged { .. }
    ));

    // Outside the bag, a bag that is not this character's, an item that is not in a slot, and an item
    // that does not exist.
    assert!(matches!(
        db::move_item(&pool, character, cupcake.id, 1, 20)
            .await
            .expect("move"),
        db::MoveOutcome::Refused {
            reason: db::Refused::OutOfRange,
            ..
        }
    ));
    assert!(matches!(
        db::move_item(&pool, character, cupcake.id, 9999, 0)
            .await
            .expect("move"),
        db::MoveOutcome::Refused {
            reason: db::Refused::NoSuchBag,
            ..
        }
    ));
    assert!(matches!(
        db::move_item(&pool, character, inventory.bags[0].assoc_item_id, 1, 0)
            .await
            .expect("move"),
        db::MoveOutcome::Refused {
            reason: db::Refused::NotInASlot,
            ..
        }
    ));
    assert!(matches!(
        db::move_item(&pool, character, 999_999, 1, 0)
            .await
            .expect("move"),
        db::MoveOutcome::Refused {
            item: None,
            reason: db::Refused::NoSuchItem,
        }
    ));

    // The equipped-items bag is a bag like any other here: wearing something is a move into it, and
    // taking it off is a move out of it.
    let equipped = inventory
        .bags
        .iter()
        .find(|bag| bag.bag_model_id == db::EQUIPPED_ITEMS_MODEL)
        .expect("the equipped bag")
        .bag_id;
    let weapon = inventory
        .items_in(equipped)
        .into_iter()
        .find(|item| item.slot == Some(0))
        .cloned()
        .expect("something in the weapon slot");
    assert!(
        matches!(
            db::move_item(&pool, character, weapon.id, 1, 6)
                .await
                .expect("move"),
            db::MoveOutcome::At { .. }
        ),
        "taking the weapon off is a move"
    );

    asura_db::login::remove_account(&pool, &name)
        .await
        .expect("clean up");
}

#[tokio::test]
async fn the_first_free_carrying_slot_is_the_backpack_first() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let (name, character) = character(&pool, "free").await;
    db::seed_default(&pool, character).await.expect("seed");

    // The backpack's first two slots hold the two cupcakes, so the third is the first free one.
    assert_eq!(
        db::first_free_carrying_slot(&pool, character)
            .await
            .expect("find"),
        Some((1, 2))
    );

    // Move a cupcake into that slot and the next free one is the slot it left.
    let inventory = db::load(&pool, character).await.expect("load");
    let cupcake = inventory.items_in(1)[0].clone();
    db::move_item(&pool, character, cupcake.id, 1, 2)
        .await
        .expect("move");
    assert_eq!(
        db::first_free_carrying_slot(&pool, character)
            .await
            .expect("find"),
        Some((1, 0)),
        "the slot the cupcake left is free again, and it comes first"
    );

    asura_db::login::remove_account(&pool, &name)
        .await
        .expect("clean up");
}

#[tokio::test]
async fn a_position_is_saved_and_read_back_and_a_new_character_has_none() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let (name, character) = character(&pool, "position").await;

    // Never saved: the world server must fall back to the map's spawn, so this has to be None and not
    // (0, 0) -- which is a legal coordinate on every map and would be taken as one.
    assert_eq!(
        db_game::position(&pool, character)
            .await
            .expect("read position"),
        None,
        "a character that has never been played has no position"
    );

    assert!(
        db_game::record_position(&pool, character, MAP, 6700.5, 4400.25, 3)
            .await
            .expect("write position"),
        "the character exists, so the write lands"
    );

    let saved = db_game::position(&pool, character)
        .await
        .expect("read position")
        .expect("the character exists");
    assert_eq!(
        saved,
        db_game::Position {
            map_id: MAP,
            x: 6700.5,
            y: 4400.25,
            plane: 3,
        }
    );

    // Overwriting is the normal case: a player leaves more than once.
    db_game::record_position(&pool, character, MAP, 6602.0, 4485.0, 0)
        .await
        .expect("write position");
    let again = db_game::position(&pool, character)
        .await
        .expect("read position")
        .expect("the character exists");
    assert_eq!(again.plane, 0);
    assert_eq!((again.x, again.y), (6602.0, 4485.0));

    asura_db::login::remove_account(&pool, &name)
        .await
        .expect("clean up");
}

/// The character settings blob the auth service sends is built from this row, so what
/// `create_character` writes has to be what `settings` reads back -- including the armour pieces, which
/// are derived from the equipped bag rather than stored (the reference denormalises them).
#[tokio::test]
async fn the_character_settings_come_back_from_the_row_the_creation_wrote() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let name = format!("m5-settings-{}", Uuid::new_v4());
    asura_db::login::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &name,
        "Settings Subject",
        MAP,
    )
    .await
    .expect("seed identity");
    let account = account_of(&pool, &name).await;

    // A character created the way `CHAR_CREATION_CONFIRM` creates one: a chosen appearance, a profession,
    // the campaign the screen was in, and the equipment the profession change swapped in.
    let traits = db_game::Traits {
        appearance: 0x0310_0000, // a Warrior with hair style 3, as the reference's own blob has it
        profession: 1,
        unlocked_professions: 1 << 1,
        gold: 0,
        campaign: 1,
        secondary_profession: 0,
        level: 1,
    };
    let created = db::create_character(&pool, account, "Settings Subject Jr", MAP, traits, &[])
        .await
        .expect("create the character");

    let settings = db_game::settings(&pool, created)
        .await
        .expect("read the settings")
        .expect("the character exists");
    // Two facts, two fields: `map_id` is the map the row names (where it was created), and`r`n    // `last_outpost` is the outpost memory the login rule writes, which a creation does not set.`r`n    assert_eq!(settings.map_id, MAP, "the map the creation wrote");`r`n    assert_eq!(settings.last_outpost, 0, "no outpost placement yet");
    assert_eq!(settings.appearance, 0x0310_0000, "the appearance it chose");
    assert_eq!(settings.campaign, 1, "Prophecies");
    assert_eq!(settings.level, 1, "the reference's creation level");
    assert!(
        !settings.is_pvp,
        "a campaign character is not a pvp character"
    );
    assert_eq!(settings.secondary_profession, 0, "the confirm sets none");
    assert_eq!(settings.helm_status, 1, "HelmStatus_Show");

    // No equipment was passed, so the pieces are whatever the seed put in the equipped bag -- one per
    // armour item, and no more.
    let inventory = db::load(&pool, created).await.expect("load");
    let worn = [2u8, 3, 4, 5, 6]
        .iter()
        .filter_map(|slot| inventory.item_in_equipped_slot(*slot))
        .count();
    assert_eq!(settings.pieces.len(), worn, "one piece per armour item");

    asura_db::login::remove_account(&pool, &name)
        .await
        .expect("clean up");
}

/// Reaching an instance is what unlocks it, and this row is the record: the first arrival writes it, a
/// second arrival writes nothing, and the reached instances read back. The bitmap those rows become is
/// `asura_world::map::unlock_bitmap`, unit-tested there; this proves the store the world map depends on.
#[tokio::test]
async fn an_instance_is_unlocked_by_arriving_once() {
    let Some(pool) = pool().await else {
        return skip();
    };
    let (name, character) = character(&pool, "unlock").await;

    assert!(
        db_game::unlocked_instances(&pool, character)
            .await
            .expect("read")
            .is_empty(),
        "a character that has just been created has reached nothing"
    );
    assert!(
        db_game::unlock_instance(&pool, character, MAP)
            .await
            .expect("unlock"),
        "the first arrival is the one that records it"
    );
    assert!(
        !db_game::unlock_instance(&pool, character, MAP)
            .await
            .expect("a second arrival"),
        "arriving again records nothing new"
    );
    assert_eq!(
        db_game::unlocked_instances(&pool, character)
            .await
            .expect("read"),
        vec![MAP]
    );

    db_game::unlock_instance(&pool, character, 148)
        .await
        .expect("a second instance");
    let mut reached = db_game::unlocked_instances(&pool, character)
        .await
        .expect("read");
    reached.sort_unstable();
    assert_eq!(reached, vec![MAP, 148], "both, and only those");

    asura_db::login::remove_account(&pool, &name)
        .await
        .expect("clean up");
}

/// The account id behind a username, for a test that needs to create a character on it.
async fn account_of(pool: &PgPool, username: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM t_account WHERE username = $1")
        .bind(username)
        .fetch_one(pool)
        .await
        .expect("the account exists")
}
