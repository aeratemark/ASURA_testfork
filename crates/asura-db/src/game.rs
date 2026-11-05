//! The reads a world server needs: which map blob to load, where to spawn, and who the character is.
//!
//! Small on purpose. A world server owns the gameplay connection and its own in-memory simulation;
//! what it takes from the database is the handful of facts a client cannot be told without, and what
//! it writes back arrives with the milestone that writes state (`docs/roadmap.md`, M5).
//!
//! Two of those facts are a character's position, and this is where it is read and written: the world
//! saves it when a player leaves and starts from it when the player comes back. **The reference does not
//! do this** - it re-spawns every character at its map's spawn point on every session, and keeps only
//! the map a character was last in (`characters.last_outpost`). ASURA keeps the coordinate because the
//! whole of placement exists to answer "who is where", and a player who logs back in where it logged out
//! is the smallest thing that answer is worth. The seed leaves the column NULL, so a character that has
//! never been played still spawns where the map says.

use crate::{PgPool, Uuid};

/// What a map needs in order to be loaded: the map blob's file id, whether the map is an outpost or
/// an explorable area, and where a character stands in it.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnMap {
    /// The map the row is about.
    pub map_id: i32,
    /// 0 for an outpost/town, 1 for an explorable area (the reference's `MapType`).
    pub map_type: i32,
    /// The `Gw.dat` file id of the map blob. The client streams the map from this number; a map
    /// without one loads to a black screen.
    pub map_file_id: i64,
    /// The spawn coordinate, as the client's world coordinates. Verified against a live client in the
    /// reference rig for the development maps (`GmMapsConfig.c`).
    pub spawn_x: f32,
    /// The spawn coordinate the map row carries.
    pub spawn_y: f32,
    /// The plane inside the map blob, not a height: 0 for every pre-Searing map measured so far.
    pub spawn_plane: i16,
    /// What to call it, so `/maps` and the log can name a map instead of printing its number. Empty when
    /// the database has no name for it - a worse answer, not a broken one.
    pub name: String,
}

/// A map row as [`SpawnMap`] reads it. One column list, used by both queries.
type MapRow = (i32, i32, i64, f32, f32, i16, String);

fn spawn_map_from_row(row: MapRow) -> SpawnMap {
    let (map_id, map_type, map_file_id, spawn_x, spawn_y, spawn_plane, name) = row;
    SpawnMap {
        map_id,
        map_type,
        map_file_id,
        spawn_x,
        spawn_y,
        spawn_plane,
        name,
    }
}

/// The configured map, if the database has it.
pub async fn spawn_map(pool: &PgPool, map_id: i32) -> sqlx::Result<Option<SpawnMap>> {
    let row: Option<MapRow> = sqlx::query_as(
        "SELECT map_id, map_type, map_file_id, spawn_x, spawn_y, spawn_plane, name
           FROM t_map_config WHERE map_id = $1",
    )
    .bind(map_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(spawn_map_from_row))
}

/// Every spawn point a map holds, in the reference's order (`GmMapsConfig.h`: `spawnpoints[]`), index 0
/// first. The world spawns a character at index 0, which is what `GameSrv.c:1075` does; the rest are the
/// capture's, kept because a map entered from a different portal has more than one.
pub async fn spawn_points(pool: &PgPool, map_id: i32) -> sqlx::Result<Vec<(f32, f32, i16)>> {
    sqlx::query_as::<_, (f32, f32, i16)>(
        "SELECT x, y, plane FROM t_map_spawn WHERE map_id = $1 ORDER BY idx",
    )
    .bind(map_id)
    .fetch_all(pool)
    .await
}

/// One doorway of `map_portal`: the instance it stands in, where, and - when known - the instance it
/// leads to and where a crossing character lands there.
pub type PortalRow = (i32, f32, f32, Option<i32>, Option<f32>, Option<f32>);

/// Every doorway the content defines (`0025_map_portals.sql`), map by map.
///
/// An `Err` here usually means the table does not exist yet (the schema has not been re-applied); the
/// world then keeps its compiled doorway table, which is the behaviour it had before the table existed.
pub async fn map_portals(pool: &PgPool) -> sqlx::Result<Vec<PortalRow>> {
    sqlx::query_as::<_, PortalRow>(
        "SELECT map_id, x, y, to_map, arrive_x, arrive_y FROM t_map_portal ORDER BY map_id, idx",
    )
    .fetch_all(pool)
    .await
}

/// Every map the catalogue describes, in id order.
///
/// What a world hosts when it is not told otherwise: the reference pre-spawns one server per configured
/// map, so a freshly started world here serves the same set. `--maps` narrows it on purpose (a soak, a
/// single-map test), never by default - a world hosting one map answers `/tp` with "not hosted" for the
/// rest, and `/maps` prints one alias.
pub async fn configured_map_ids(pool: &PgPool) -> sqlx::Result<Vec<i32>> {
    sqlx::query_scalar("SELECT map_id FROM t_map_config ORDER BY map_id")
        .fetch_all(pool)
        .await
}

/// A character's name, as the client should see it in the map.
pub async fn character_name(pool: &PgPool, character: Uuid) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar("SELECT name FROM t_characters WHERE id = $1")
        .bind(character)
        .fetch_optional(pool)
        .await
}

/// The character fields the load block carries, as `GameSrv_HandleCharCreationConfirm` stores them.
///
/// Each one is sent today: `appearance` is the packed word `GAME_SMSG_PLAYER_CREATE` carries (the
/// reference's `sex`/`height`/`skin_color`/`hair_color`/`face_style`/`hair_style`/`race`, packed by
/// `Appearance`), `profession` and `unlocked_professions` ride the two profession messages, and `gold` is
/// `UPDATE_GOLD_STORAGE`. Before this existed the load block sent the reference's *default* character to
/// everyone - a real liberty, recorded in `docs/09-parity-audit.md` #9.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Traits {
    /// The packed appearance word, as `CREATE_NAMED_ITEM` and the load block carry it.
    pub appearance: u32,
    /// The profession the item was made for, or 0.
    pub profession: u8,
    /// The professions this account may pick, as the reference packs them.
    pub unlocked_professions: u32,
    /// The character`s gold, as `UPDATE_GOLD_STORAGE` reports it.
    pub gold: u32,
    /// `Campaign` (`GmChar.h`): 0 none (a pvp character), 1 Prophecies, 2 Factions, 3 Nightfall,
    /// 4 Eye of the North. The character settings blob carries four bits of it.
    pub campaign: u8,
    /// The secondary profession, four bits on the wire.
    pub secondary_profession: u8,
    /// The character's level (`characters.level`, migration 0009): one of the fifteen dwords
    /// `GAME_SMSG_CHARACTER_UPDATE_FACTIONS` carries, and the number the client's own record shows.
    pub level: u16,
}

impl Default for Traits {
    /// What a character that predates the columns looks like: no appearance bits, no profession, no gold.
    /// The load block falls back to the reference's defaults for a character with no row, which is the same
    /// answer the reference gives a row it cannot read.
    fn default() -> Self {
        Traits {
            appearance: 0,
            profession: 0,
            unlocked_professions: 0,
            gold: 0,
            campaign: 0,
            secondary_profession: 0,
            level: 1,
        }
    }
}

/// The traits of one character, or `None` if there is no such row.
pub async fn traits(pool: &PgPool, character: Uuid) -> sqlx::Result<Option<Traits>> {
    let row: Option<(i64, i32, i64, i32, i32, i32, i32)> = sqlx::query_as(
        "SELECT appearance, primary_profession, unlocked_professions, gold, campaign::int,
                secondary_profession::int, level::int
           FROM t_characters WHERE id = $1",
    )
    .bind(character)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(appearance, profession, unlocked, gold, campaign, secondary, level)| Traits {
            appearance: appearance as u32,
            profession: profession as u8,
            unlocked_professions: unlocked as u32,
            gold: gold as u32,
            campaign: campaign as u8,
            secondary_profession: secondary as u8,
            level: level as u16,
        },
    ))
}

/// One armour piece of the character settings blob: `GmChar.h`'s inline `pieces[]` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArmourPiece {
    /// The piece's `file_id`, truncated to sixteen bits -- what `GameSrv.c:1903` stores.
    pub file_id: u16,
    /// The four dye nibbles, packed as the item's `dye_colors` holds them.
    pub colors: u16,
}

/// Everything `CharacterSettings_FromDbCharacter` (`GmChar.c`) reads, which is what the auth service sends
/// for each character in the login reply's character list.
///
/// The reference keeps the armour pieces denormalised on the character row (`file_id_body`..`colors_gloves`)
/// and fills them in at creation from the equipped bag (`GameSrv.c:1893-1921`). This derives them from the
/// equipped bag on every read instead: same answer, one copy of it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    /// The map the character was last in (`last_outpost` in the reference's row).
    pub last_outpost: u16,
    /// The row's own `characters.map_id`: the map the character is in, before the outpost rule runs.
    /// A creation confirm writes the starter outpost here, which is why the auth service reads it.
    pub map_id: i32,
    /// The packed `Appearance` word, exactly as `create_character` stored it.
    pub appearance: u32,
    /// 0 none, 1 Prophecies, 2 Factions, 3 Nightfall, 4 Eye of the North.
    pub campaign: u8,
    /// The character`s level, from `characters.level`.
    pub level: u8,
    /// Whether the character was created for PvP.
    pub is_pvp: bool,
    /// The secondary profession, four bits on the wire.
    pub secondary_profession: u8,
    /// 0 none, 1 show, 2 hide.
    pub helm_status: u8,
    /// Up to five pieces, in the reference's order: body, legs, head, boots, gloves. A piece the character
    /// is not wearing is absent, not zero-filled -- `number_of_pieces` is the count.
    pub pieces: Vec<ArmourPiece>,
}

/// One `characters` row, as [`settings`] reads it: the columns the settings block is built from.
struct CharacterRow {
    last_outpost: i32,
    map_id: i32,
    appearance: i64,
    campaign: i32,
    level: i32,
    is_pvp: bool,
    secondary_profession: i32,
    helm_status: i32,
}

// Decoded by column name, which is why the query casts every narrow column to `int`: one row type for
// the settings block, and a column renamed in the SQL fails here rather than in a client assertion.
// Hand-written because the crate does not enable sqlx's derive macros - see `Cargo.toml`.
impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for CharacterRow {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(CharacterRow {
            last_outpost: row.try_get("last_outpost")?,
            map_id: row.try_get("map_id")?,
            appearance: row.try_get("appearance")?,
            campaign: row.try_get("campaign")?,
            level: row.try_get("level")?,
            is_pvp: row.try_get("is_pvp")?,
            secondary_profession: row.try_get("secondary_profession")?,
            helm_status: row.try_get("helm_status")?,
        })
    }
}
/// The settings of one character, or `None` if there is no such row.
pub async fn settings(pool: &PgPool, character: Uuid) -> sqlx::Result<Option<Settings>> {
    // The casts are not decoration: `smallint` and `integer` decode into different Rust types, and a
    // database that has one of each (a development database that has had migrations applied by hand) is
    // not worth a runtime error. Everything comes back as `int` and is narrowed below.
    // One row, decoded in one go. Named fields rather than an eight-wide tuple, so a column added in
    // the wrong order is a compile error instead of a swapped value (`docs/38-layer-decisions.md`, D16).
    let row: Option<CharacterRow> = sqlx::query_as(
        "SELECT last_outpost::int, map_id, appearance, campaign::int, level::int, is_pvp,
                secondary_profession::int, helm_status::int
           FROM t_characters WHERE id = $1",
    )
    .bind(character)
    .fetch_optional(pool)
    .await?;
    let Some(CharacterRow {
        last_outpost,
        map_id: row_map_id,
        appearance,
        campaign,
        level,
        is_pvp,
        secondary_profession: secondary,
        helm_status: helm,
    }) = row
    else {
        return Ok(None);
    };

    // The five armour slots of the equipped bag, in the reference's order (`EquippedItemSlot_Body` = 2 ..
    // `_Gloves` = 6). Read through the inventory because that is where the items are.
    let inventory = crate::inventory::load(pool, character).await?;
    let mut pieces = Vec::with_capacity(5);
    for slot in 2..=6u8 {
        let Some(item) = inventory.item_in_equipped_slot(slot) else {
            continue;
        };
        pieces.push(ArmourPiece {
            file_id: (item.file_id as u64 & 0xFFFF) as u16,
            colors: (item.dye_colors as u32 & 0xFFFF) as u16,
        });
    }

    Ok(Some(Settings {
        // The outpost the character was last *placed* in. It is not `map_id`, which records the map it was
        // last in - explorables included (docs/22-world-model.md, the outpost rule).
        last_outpost: u16::try_from(last_outpost).unwrap_or(0),
        map_id: row_map_id,
        appearance: appearance as u32,
        campaign: campaign as u8,
        level: level as u8,
        is_pvp,
        secondary_profession: secondary as u8,
        helm_status: helm as u8,
        pieces,
    }))
}

/// Where a character last stood.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    /// The map the position belongs to. A position is only usable on the map it was taken in.
    pub map_id: i32,
    /// The coordinate the client reports.
    pub x: f32,
    /// The coordinate that goes with it.
    pub y: f32,
    /// The plane inside the map blob, the client's third coordinate.
    pub plane: i16,
}

/// The position a character was saved at, or `None` if it has never been saved.
///
/// `characters.pos_plane IS NULL` is what "never saved" means, and the whole row is then ignored: a
/// character that has never been played has `pos_x`/`pos_y` at their default of 0, which is a legal
/// coordinate (it is outside the walkable geometry on every map measured so far, but it is not out of
/// range), so a caller must not be handed it as if it were a position.
pub async fn position(pool: &PgPool, character: Uuid) -> sqlx::Result<Option<Position>> {
    let row: Option<(i32, f32, f32, Option<i16>)> =
        sqlx::query_as("SELECT map_id, pos_x, pos_y, pos_plane FROM t_characters WHERE id = $1")
            .bind(character)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(map_id, x, y, plane)| {
        plane.map(|plane| Position {
            map_id,
            x,
            y,
            plane,
        })
    }))
}

/// Save where a character stands, and which map it stands in.
///
/// Written when the player leaves. The reference persists a character's *map* and nothing else about its
/// position, so the row this writes is one this project adds to the reference's model rather than one it
/// copies (`crates/asura-db/migrations/0006_inventory.sql`).
pub async fn record_position(
    pool: &PgPool,
    character: Uuid,
    map_id: i32,
    x: f32,
    y: f32,
    plane: i16,
) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE t_characters
            SET map_id = $2, pos_x = $3, pos_y = $4, pos_plane = $5, updated_at = now()
          WHERE id = $1",
    )
    .bind(character)
    .bind(map_id)
    .bind(x)
    .bind(y)
    .bind(plane)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Forget where a character stood, so the next arrival cannot resume it.
///
/// A travel does not preserve a position - only a disconnect does (`docs/22-world-model.md`, the
/// owner's rule of 2026-11-05) - so a travel that is not a portal walk calls this before the
/// redirect: the destination then stands where its own spawn list says instead of where the
/// character last was. `pos_plane IS NULL` is what "never saved" means
/// ([`position`]), so clearing it reads back as a character that has never been played. `map_id` is
/// left alone: it is the map the character is in, which the arrival itself records.
pub async fn forget_position(pool: &PgPool, character: Uuid) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE t_characters
            SET pos_plane = NULL, updated_at = now()
          WHERE id = $1",
    )
    .bind(character)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Record the last **outpost** a character was placed in.
///
/// The owner's base rule (docs/22-world-model.md, "Spawn points, and the outpost rule"): a character that
/// enters the game - at character select, not on a reconnect - is moved to the last outpost it visited.
/// `characters.last_outpost` is that memory, and it is written only when the placement is an outpost, so a
/// travel into an explorable cannot overwrite it.
pub async fn remember_outpost(pool: &PgPool, character: Uuid, map_id: i32) -> sqlx::Result<bool> {
    let done = sqlx::query("UPDATE t_characters SET last_outpost = $2 WHERE id = $1")
        .bind(character)
        .bind(map_id as i16)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
/// Mark one instance as reached by one character.
///
/// The owner's model (`docs/22-world-model.md`): arriving sets the bit. Idempotent - a character that has
/// already been here keeps the timestamp of its first arrival, which is the only thing the row records.
/// `Ok(true)` means this call is the one that unlocked it.
pub async fn unlock_instance(pool: &PgPool, character: Uuid, map_id: i32) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "INSERT INTO t_instance_unlock (character_id, map_id) VALUES ($1, $2)
         ON CONFLICT (character_id, map_id) DO NOTHING",
    )
    .bind(character)
    .bind(map_id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Every instance this character has reached, in the order it reached them.
///
/// The bitmap those rows become is built by `asura_world::map::unlock_bitmap`, because the addressing is
/// the client's, not the database's.
pub async fn unlocked_instances(pool: &PgPool, character: Uuid) -> sqlx::Result<Vec<i32>> {
    let rows: Vec<(i32,)> = sqlx::query_as(
        "SELECT map_id FROM t_instance_unlock WHERE character_id = $1 ORDER BY unlocked_at, map_id",
    )
    .bind(character)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(map_id,)| map_id).collect())
}

/// Every map the database knows, so a world server can publish a pool for each of them at startup.
pub async fn all_maps(pool: &PgPool) -> sqlx::Result<Vec<SpawnMap>> {
    let rows: Vec<MapRow> = sqlx::query_as(
        "SELECT map_id, map_type, map_file_id, spawn_x, spawn_y, spawn_plane, name
           FROM t_map_config ORDER BY map_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(spawn_map_from_row).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name a world server puts in the map is the character's own, and the spawn comes from the
    /// map row: a map without a coordinate spawns a character at the origin, which on every map
    /// measured so far is outside the walkable geometry.
    #[test]
    fn a_spawn_map_carries_the_map_key_the_blob_and_the_spawn() {
        let map = SpawnMap {
            map_id: 146,
            map_type: 0,
            map_file_id: 0x1B97D,
            spawn_x: 6602.0,
            spawn_y: 4485.0,
            spawn_plane: 0,
            name: "Lakeside County".to_string(),
        };
        assert_eq!(map.map_id, 146, "Lakeside County");
        assert_eq!(map.name, "Lakeside County", "and it says so");
        assert_eq!(map.map_type, 0, "an outpost");
        assert_eq!(map.map_file_id, 113021, "the pre-Searing blob, 0x1B97D");
        assert_eq!(
            map.spawn_plane, 0,
            "the plane the tester read back from a standing character"
        );
        assert_ne!(map.spawn_x, 0.0, "and not the origin");
    }
}
