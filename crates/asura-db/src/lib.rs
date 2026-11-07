//! Schema ownership and migrations.
//!
//! The rule this crate exists to enforce: **nothing outside this crate writes SQL.** Both services go
//! through functions here, so a schema change has one place to land and the translation from GWPS's
//! SQLite cannot leak into service code.
//!
//! AzerothCore parallel: their `DatabaseWorkerPool` plus per-database prepared statements. The
//! difference in shape is that their pools live inside each program; here the pool configuration is
//! ours and the statements are functions whose names say what they do.
//!
//! Vendor: PostgreSQL, a recorded deviation from AzerothCore's MySQL
//! (`docs/adr/0008-database-vendor.md`). The *structure* stays theirs; only the driver and the DDL
//! dialect differ, and both live behind this crate.
//!
//! ## What this database is for, and what is only passing through
//!
//! Two kinds of row live here, and they are leaving in different directions
//! (`docs/38-layer-decisions.md`, D20, D21, D23).
//!
//! **Operational state stays.** A row that changes while the server runs belongs here: `characters`,
//! `bag`, `item`, `session`, `account`, `transfer`, `lease`, `world`, `world_pool`, `service`,
//! `instance_unlock`, and the saved position. A character's gold changes while the server runs, and
//! nothing else may own that number.
//!
//! **Content is passing through.** A row that only *describes* the game will leave for a package
//! folder, where an author can copy and edit it without SQL: `map_config`, `map_spawn`, `map_portal`,
//! and the 214 `INSERT`s of map content inside migrations `0013`, `0014` and `0016`-`0022`. Until that
//! move is scheduled those tables are still how the world and the auth service read a map, so they
//! stay correct - but no new content row belongs in a migration. A new map is a row in `map_config`
//! only because the move has not happened yet.
//!
//! One sentence decides which side a new table is on: does the row change while the server runs? If
//! it does, it is state and stays. If it only describes the game, it is content and will move.

/// The initial schema, compiled into the binary so a developer can apply it without hunting for files.
pub const SCHEMA_V1: &str = include_str!("../migrations/0001_init.sql");

/// The lease ledger (M2): one row per reserved slot, so a lease can be identified, released and
/// reaped. See the migration for why the counter and the ledger coexist.
pub const SCHEMA_V2: &str = include_str!("../migrations/0002_leases.sql");

/// The portal identity (M3): the "who you are" UUID the client presents, stored beside the session id
/// because the login lookup matches both. See the migration for why it is NOT NULL.
pub const SCHEMA_V3: &str = include_str!("../migrations/0003_portal_identity.sql");

/// The pending transfer (M4): the token the auth service issues and the world server claims. See the
/// migration for why the coordination is a row rather than a handed-over socket.
pub const SCHEMA_V4: &str = include_str!("../migrations/0004_transfers.sql");

/// Where a character stands in a map (M4): the spawn coordinate and plane, verified against a live
/// client in the reference rig and stored as data rather than as a constant.
pub const SCHEMA_V5: &str = include_str!("../migrations/0005_map_spawns.sql");

/// The inventory (M5): the items a character carries, the bags that hold them, and the position it
/// stands in. Replaces the replayed item messages with rows. See the migration for why the reference's
/// own `items`/`bags` tables are not copied verbatim.
pub const SCHEMA_V6: &str = include_str!("../migrations/0006_inventory.sql");

/// A map's name, so /maps can answer with something better than integers (M5.1).
pub const SCHEMA_V7: &str = include_str!("../migrations/0007_map_names.sql");

/// The character traits the reference stores and this server used to send as constants.
pub const SCHEMA_V8: &str = include_str!("../migrations/0008_character_traits.sql");

/// The rest of the character settings blob: level, campaign, the helmet status, the secondary profession
/// and the is-pvp flag, all of which `CharacterSettings_FromDbCharacter` (`GmChar.c`) puts on the wire.
pub const SCHEMA_V9: &str = include_str!("../migrations/0009_character_settings.sql");

/// The general service registry (S2): every service kind announces itself like a world server does.
/// Written by the supervisor for the services it starts; read by consumers that need to find them.
/// See the migration for the "table consumers read, not a network hop" rule.
pub const SCHEMA_V10: &str = include_str!("../migrations/0010_service_registry.sql");

/// A world's start time (S3): "the newest build" is the build of the most recently started fresh
/// world hosting the map. See the migration for why a restart must not masquerade as a newer build.
pub const SCHEMA_V11: &str = include_str!("../migrations/0011_world_started_at.sql");

/// The EULA version an account accepted (M3).
pub const SCHEMA_V12: &str = include_str!("../migrations/0012_account_eula.sql");

/// The creation instance, as a map a world can host (M5).
pub const SCHEMA_V13: &str = include_str!("../migrations/0013_creation_instance.sql");

/// The outpost a new character starts in, for the campaign the client named.
pub const SCHEMA_V14: &str = include_str!("../migrations/0014_starter_outpost.sql");

/// Undoing the creation-instance map rows of 0014.
pub const SCHEMA_V15: &str = include_str!("../migrations/0015_revert_creation_maps.sql");

/// The creation instance again, with the map type the client asks for.
pub const SCHEMA_V16: &str = include_str!("../migrations/0016_creation_instance_again.sql");

/// The starter outpost corrected against a live creation.
pub const SCHEMA_V17: &str = include_str!("../migrations/0017_starter_outpost.sql");

/// Outposts measured from the client.
pub const SCHEMA_V18: &str = include_str!("../migrations/0018_measured_outposts.sql");

/// Monastery Overlook, the Factions creation destination.
pub const SCHEMA_V19: &str = include_str!("../migrations/0019_monastery_overlook.sql");

/// A spawn point per map, measured.
pub const SCHEMA_V20: &str = include_str!("../migrations/0020_map_spawn_points.sql");

/// More maps and their spawn points, measured.
pub const SCHEMA_V21: &str = include_str!("../migrations/0021_measured_maps_and_spawns.sql");

/// The third map tour, as rows.
pub const SCHEMA_V22: &str = include_str!("../migrations/0022_tour3_maps.sql");

/// Which instance a character has unlocked.
pub const SCHEMA_V23: &str = include_str!("../migrations/0023_instance_unlocks.sql");

/// The last outpost a character was placed in (D28).
pub const SCHEMA_V24: &str = include_str!("../migrations/0024_last_outpost.sql");

/// The doorways, each row carrying the capture it came from.
pub const SCHEMA_V25: &str = include_str!("../migrations/0025_map_portals.sql");

/// Every data table carries a `t_` prefix (D31): `t_session` is a row, `session` is the login. It
/// reaches a database through `tools/db-setup.ps1` with 0012-0025, one file at a time, so it is not
/// part of `SCHEMA` below: that list is the runner's 1..11 and this file does not widen it.
///
/// The rename is the last statement that touches names, so a database that reaches it has the tables
/// every query in this crate names.
pub const SCHEMA_V26: &str = include_str!("../migrations/0026_t_prefix.sql");

/// A character's explored cells: one packed grid per (character, continent), the operational state
/// a map surfaces draw from (`docs/38-layer-decisions.md`, D21, D31, D37).
pub const SCHEMA_V27: &str = include_str!("../migrations/0027_character_cartography.sql");

/// Title progress: points per (character, title) and per (account, title), and the title a character
/// shows (`docs/40-titles-system-research.md`, section 10.2).
pub const SCHEMA_V28: &str = include_str!("../migrations/0028_titles.sql");

/// Guilds: the guild, its members and its open invitations (`docs/41-guild-system-research.md`).
pub const SCHEMA_V29: &str = include_str!("../migrations/0029_guilds.sql");

/// Parties that travel: one row per move of a party, one per character's place in it
/// (`docs/45-party-travel.md`).
pub const SCHEMA_V30: &str = include_str!("../migrations/0030_parties.sql");

/// The source map of a doorway transfer; ordinary login and map travel carry no doorway origin.
pub const SCHEMA_V31: &str = include_str!("../migrations/0031_transfer_arrival_origin.sql");

/// Every migration, in application order. A developer applying the schema gets one string per
/// migration file, so a new file is picked up without editing the tooling.
pub const SCHEMA: &[&str] = &[
    SCHEMA_V1, SCHEMA_V2, SCHEMA_V3, SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8,
    SCHEMA_V9, SCHEMA_V10, SCHEMA_V11, SCHEMA_V12, SCHEMA_V13, SCHEMA_V14, SCHEMA_V15, SCHEMA_V16,
    SCHEMA_V17, SCHEMA_V18, SCHEMA_V19, SCHEMA_V20, SCHEMA_V21, SCHEMA_V22, SCHEMA_V23, SCHEMA_V24,
    SCHEMA_V25, SCHEMA_V26, SCHEMA_V27, SCHEMA_V28, SCHEMA_V29, SCHEMA_V30, SCHEMA_V31,
];

/// The tables the coordination model depends on. Used by the startup check: a service refuses to run
/// against a database that is older than the code, loudly, rather than failing on the first query.
///
/// Prefixed, like every data table (`docs/38-layer-decisions.md`, D31).
pub const REQUIRED_TABLES: &[&str] = &[
    "t_world",
    "t_world_pool",
    "t_lease",
    "t_session",
    "t_account",
    "t_characters",
    "t_map_config",
    "t_bag",
    "t_item",
    "t_service",
    "t_character_cartography",
    "t_character_titles",
    "t_account_titles",
    "t_guilds",
    "t_guild_members",
    "t_guild_invites",
    "t_parties",
    "t_party_members",
];

/// The account row itself: the EULA version it accepted, and whatever later column a login needs.
pub mod account;

/// The queries placement needs: the atomic reservation, the lease ledger, and the reaper. Every
/// statement that mutates a pool lives in this module, behind a function whose name says what it does.
pub mod placement;

/// The queries login needs: resolving the presented identity pair to an account, and reading that
/// account's characters.
pub mod login;

/// The general service registry (S2): every service kind announces itself like a world server does.
pub mod catalog;

/// The reads a world server needs: the map to load, where to place a character, and its name.
pub mod game;

/// A character's explored cells, one packed grid per (character, continent).
pub mod cartography;

/// Title progress: the points per title, per character and per account, and the title a character shows.
pub mod titles;

/// Guilds: the guild row, its members and its open invitations.
pub mod guilds;

/// Parties that travel: the ledger a party crosses from one instance into another with.
pub mod parties;

/// The inventory: a character's bags and items, the default set the reference seeds, and the one query
/// a drop needs. Replaces the replayed item messages with rows.
pub mod inventory;

/// The migration runner, and the startup check that refuses a database older than the code.
pub mod migrations;

/// The portal identity the web gate hands the client (the pair the auth service resolves).
pub mod portal;

/// Where the programs find their settings: the environment, then the repository's `.env`.
pub mod config;

// Re-exported so the harness and the integration tests (separate crates) can name the pool and the
// session id without depending on sqlx and uuid themselves.
pub use sqlx::PgConnection;
pub use sqlx::PgPool;
pub use uuid::Uuid;

/// The database error type, re-exported for the same reason: a service should be able to say "the
/// database failed" without taking sqlx as its own dependency.
pub use sqlx::Error as DatabaseError;

/// Table names declared by `CREATE TABLE` statements in a schema.
///
/// Parsed rather than hard-coded, so the test below cannot pass by matching a string the schema does
/// not actually contain. `IF NOT EXISTS` is handled because every statement in our migrations uses it.
pub fn declared_tables(schema: &str) -> Vec<String> {
    schema
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("--"))
        .filter_map(|line| {
            let upper = line.to_ascii_uppercase();
            let rest = upper.strip_prefix("CREATE TABLE")?;
            let mut words = rest.split_whitespace();
            let first = words.next()?;
            let name = if first == "IF" {
                // IF NOT EXISTS <name> (
                words.next()?; // NOT
                words.next()?; // EXISTS
                words.next()?
            } else {
                first
            };
            let cleaned = name.trim_matches(|c: char| c == '(' || c == '"' || c == '`');
            if cleaned.is_empty() {
                None
            } else {
                Some(cleaned.to_ascii_lowercase())
            }
        })
        .collect()
}

/// Statements in a schema, with comments removed and empty fragments dropped.
///
/// Splitting on `;` alone is wrong twice over: comments contain punctuation, and the indentation left
/// behind would produce blank fragments that a naive check treats as statements; and a `$$ ... $$` body
/// carries its own semicolons, so a plain split would cut one statement into several. Both are handled
/// here, which is why a caller sees one entry per real statement.
pub fn statements(schema: &str) -> Vec<String> {
    let without_comments: String = schema
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut pieces: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_dollar_quote = false;
    let mut rest = without_comments.as_str();
    while let Some(ch) = rest.chars().next() {
        if rest.starts_with("$$") {
            in_dollar_quote = !in_dollar_quote;
            current.push_str("$$");
            rest = &rest[2..];
            continue;
        }
        if ch == ';' && !in_dollar_quote {
            pieces.push(current.trim().to_owned());
            current.clear();
        } else {
            current.push(ch);
        }
        rest = &rest[ch.len_utf8()..];
    }
    pieces.push(current.trim().to_owned());

    pieces
        .into_iter()
        .filter(|statement| !statement.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_schema() -> String {
        SCHEMA.join("\n")
    }

    /// A table a schema text ends up with: created by `CREATE TABLE`, or renamed to by the prefix
    /// migration, whose targets are single-quoted inside a plpgsql array.
    fn declared_or_renamed(schema: &str) -> Vec<String> {
        let mut names = declared_tables(schema);
        // Comments go first: the migrations quote the reference in them, apostrophes included, and a
        // stray one would pair up with a real literal and hide it.
        let bare: String = schema
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        let (mut quoted, mut token) = (false, String::new());
        for ch in bare.chars() {
            match (quoted, ch) {
                (false, '\'') => quoted = true,
                (true, '\'') => {
                    quoted = false;
                    let tail = token.strip_prefix("t_").unwrap_or("");
                    if !tail.is_empty()
                        && tail
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    {
                        names.push(token.clone());
                    }
                    token.clear();
                }
                (true, _) => token.push(ch),
                (false, _) => {}
            }
        }
        names
    }

    #[test]
    fn schema_defines_every_required_table() {
        let declared = declared_or_renamed(&format!("{}\n{}", all_schema(), SCHEMA_V26));
        for table in REQUIRED_TABLES {
            assert!(
                declared.iter().any(|name| name == table),
                "schema is missing table `{}`; declared: {:?}",
                table,
                declared
            );
        }
    }

    #[test]
    fn pool_invariants_are_enforced_by_the_schema() {
        let schema = all_schema();
        // A reservation that could exceed capacity would let placement oversell a map.
        assert!(schema.contains("CHECK (leased <= capacity)"));
        // A negative lease would silently inflate capacity forever.
        assert!(schema.contains("CHECK (leased >= 0)"));
    }

    #[test]
    fn vendor_is_postgres_and_the_deviation_is_recorded() {
        let schema = all_schema();
        // The structure follows AzerothCore; the vendor is a recorded deviation, not drift.
        assert!(
            schema.contains("timestamptz"),
            "expected PostgreSQL timestamp types"
        );
        assert!(
            schema.contains("WHERE ended_at IS NULL"),
            "expected the partial index"
        );
        assert!(
            schema.contains("docs/adr/0008-database-vendor.md"),
            "a deviation must cite its decision"
        );
    }

    #[test]
    fn every_statement_is_idempotent() {
        // Migrations get applied to development databases repeatedly. `IF NOT EXISTS` covers
        // creating things; `ON CONFLICT` covers the bookkeeping insert each migration ends with.
        // A `DELETE` or an `UPDATE` needs neither marker: reaching the same rows again is the same
        // state, which is what 0015's revert of two map ids is. `INSERT ... SELECT ... WHERE NOT
        // EXISTS` is the third guard, used by 0022 where the row has no unique key to conflict on.
        for statement in statements(&all_schema()) {
            let head = statement.to_ascii_uppercase();
            let repeatable = head.starts_with("DELETE FROM")
                || head.starts_with("UPDATE ")
                || head.contains("WHERE NOT EXISTS")
                // A rename cannot be written to be repeatable: the old name is gone after it runs, and
                // both paths apply each file exactly once (`0026_t_prefix.sql` says so).
                // Conditional DDL: `0026_t_prefix.sql` renames only when the old name exists and the
                // new one does not, which is the only shape a rename can take that repeats cleanly.
                || head.contains("TO_REGCLASS(");
            assert!(
                head.contains("IF NOT EXISTS") || head.contains("ON CONFLICT") || repeatable,
                "statement is neither guarded nor idempotent: {}",
                statement.lines().next().unwrap_or("")
            );
        }
    }

    #[test]
    fn the_lease_ledger_is_in_a_migration_of_its_own() {
        // The counter lives in 0001; the identifiable ledger is added by 0002, so a database created
        // before M2 upgrades instead of being recreated.
        assert!(declared_tables(SCHEMA_V1).iter().all(|t| t != "lease"));
        assert!(declared_tables(SCHEMA_V2).iter().any(|t| t == "lease"));
    }

    #[test]
    fn the_portal_identity_is_required_and_separate_from_the_session_id() {
        // 0001 has a session id and no portal user; 0003 adds the second half of the credential.
        assert!(!SCHEMA_V1.contains("portal_user"));
        assert!(!SCHEMA_V2.contains("portal_user"));
        assert!(SCHEMA_V3.contains("portal_user uuid NOT NULL"));

        // It must be its own column. Folding it into `id` or `account_id` is the conflation the
        // migration exists to prevent, so the statement must name the new column outright.
        let changes: Vec<String> = statements(SCHEMA_V3)
            .into_iter()
            .filter(|statement| !statement.contains("schema_version"))
            .collect();
        assert_eq!(changes.len(), 1, "one change, one column");
        assert!(changes[0].contains("ADD COLUMN IF NOT EXISTS portal_user"));
    }

    #[test]
    fn declared_tables_parses_both_forms() {
        let sample =
            "CREATE TABLE IF NOT EXISTS alpha (\n  x int\n);\nCREATE TABLE beta (\n  y int\n);\n";
        assert_eq!(
            declared_tables(sample),
            vec!["alpha".to_string(), "beta".to_string()]
        );
    }
}
