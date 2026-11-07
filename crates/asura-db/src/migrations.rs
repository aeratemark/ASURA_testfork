//! Applying migrations, and answering the only question a database can be asked about itself:
//! **which changes has it had?**
//!
//! Without this, "is the database up to date?" can only be guessed at, and a service can start
//! happily against a schema that is older than the code calling it - the failure then lands on the
//! first query, somewhere far from the cause.
//!
//! The record lives in `schema_version`, written *by each migration file itself* as its last
//! statement. That is deliberate: it means the `psql`-driven path (`tools/db-setup.ps1`,
//! `tools/m3-verify.ps1`) and this runner produce the same bookkeeping, and a migration applied by
//! hand is still recorded. The version is the file's number.
//!
//! Two properties make this safe to run any number of times:
//!
//! * every migration is idempotent (the repository's rule: `IF NOT EXISTS`, or `ON CONFLICT`), so
//!   re-applying one is a no-op;
//! * each migration is applied inside one transaction, so a failure part-way leaves neither the
//!   change nor the record of it. `tools/` use `psql -1` for the same reason.

use crate::{
    PgPool, SCHEMA_V1, SCHEMA_V10, SCHEMA_V11, SCHEMA_V12, SCHEMA_V13, SCHEMA_V14, SCHEMA_V15,
    SCHEMA_V16, SCHEMA_V17, SCHEMA_V18, SCHEMA_V19, SCHEMA_V2, SCHEMA_V20, SCHEMA_V21, SCHEMA_V22,
    SCHEMA_V23, SCHEMA_V24, SCHEMA_V25, SCHEMA_V26, SCHEMA_V27, SCHEMA_V28, SCHEMA_V29, SCHEMA_V3,
    SCHEMA_V30, SCHEMA_V31, SCHEMA_V32, SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8,
    SCHEMA_V9,
};

/// One migration: a version number (its file's number) and its statements.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// The migration number, as recorded in `schema_version`.
    pub version: i32,
    /// The migration`s own SQL, compiled in.
    pub sql: &'static str,
}

/// Every migration, in application order. `SCHEMA_V*` in `lib.rs` owns the text, so a new migration
/// is added in one place and both the checked-in SQL and this list stay in step.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: SCHEMA_V1,
    },
    Migration {
        version: 2,
        sql: SCHEMA_V2,
    },
    Migration {
        version: 3,
        sql: SCHEMA_V3,
    },
    Migration {
        version: 4,
        sql: SCHEMA_V4,
    },
    Migration {
        version: 5,
        sql: SCHEMA_V5,
    },
    Migration {
        version: 6,
        sql: SCHEMA_V6,
    },
    Migration {
        version: 7,
        sql: SCHEMA_V7,
    },
    Migration {
        version: 8,
        sql: SCHEMA_V8,
    },
    Migration {
        version: 9,
        sql: SCHEMA_V9,
    },
    Migration {
        version: 10,
        sql: SCHEMA_V10,
    },
    Migration {
        version: 11,
        sql: SCHEMA_V11,
    },
    Migration {
        version: 12,
        sql: SCHEMA_V12,
    },
    Migration {
        version: 13,
        sql: SCHEMA_V13,
    },
    Migration {
        version: 14,
        sql: SCHEMA_V14,
    },
    Migration {
        version: 15,
        sql: SCHEMA_V15,
    },
    Migration {
        version: 16,
        sql: SCHEMA_V16,
    },
    Migration {
        version: 17,
        sql: SCHEMA_V17,
    },
    Migration {
        version: 18,
        sql: SCHEMA_V18,
    },
    Migration {
        version: 19,
        sql: SCHEMA_V19,
    },
    Migration {
        version: 20,
        sql: SCHEMA_V20,
    },
    Migration {
        version: 21,
        sql: SCHEMA_V21,
    },
    Migration {
        version: 22,
        sql: SCHEMA_V22,
    },
    Migration {
        version: 23,
        sql: SCHEMA_V23,
    },
    Migration {
        version: 24,
        sql: SCHEMA_V24,
    },
    Migration {
        version: 25,
        sql: SCHEMA_V25,
    },
    Migration {
        version: 26,
        sql: SCHEMA_V26,
    },
    Migration {
        version: 27,
        sql: SCHEMA_V27,
    },
    Migration {
        version: 28,
        sql: SCHEMA_V28,
    },
    Migration {
        version: 29,
        sql: SCHEMA_V29,
    },
    Migration {
        version: 30,
        sql: SCHEMA_V30,
    },
    Migration {
        version: 31,
        sql: SCHEMA_V31,
    },
    Migration {
        version: 32,
        sql: SCHEMA_V32,
    },
];

/// The newest version this build knows about. A database reporting less is behind the code.
pub const LATEST_VERSION: i32 = 32;

/// The advisory lock [`apply_pending`] holds while it applies one migration, so that two services
/// starting together against one database take turns instead of applying the same file at once.
/// Any constant works; this one spells "ASUR".
const MIGRATION_LOCK: i64 = 0x4153_5552;

/// What [`apply_pending`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Versions this call applied, in order.
    pub applied: Vec<i32>,
    /// Every version the database now has recorded.
    pub now_current: Vec<i32>,
}

/// The versions the database has recorded, ascending.
pub async fn applied_versions(pool: &PgPool) -> sqlx::Result<Vec<i32>> {
    sqlx::query_scalar("SELECT version FROM schema_version ORDER BY version")
        .fetch_all(pool)
        .await
}

/// The versions this build has that the database does not, ascending.
pub async fn pending_versions(pool: &PgPool) -> sqlx::Result<Vec<i32>> {
    let applied = applied_versions(pool).await?;
    Ok(MIGRATIONS
        .iter()
        .map(|migration| migration.version)
        .filter(|version| !applied.contains(version))
        .collect())
}

/// Apply every migration the database is missing.
///
/// Each one runs in its own transaction with the record of itself, so the database is never left
/// claiming a change it did not get. The transaction first takes [`MIGRATION_LOCK`] and asks again
/// whether the version is recorded: a world that boots while another is applying waits, then finds the
/// work done and skips it. `applied` lists only what this call applied.
pub async fn apply_pending(pool: &PgPool) -> sqlx::Result<Applied> {
    let pending: Vec<i32> = pending_versions(pool).await?;
    let mut applied = Vec::new();
    for version in &pending {
        let migration = MIGRATIONS
            .iter()
            .find(|migration| migration.version == *version)
            .expect("pending_versions only lists versions in MIGRATIONS");

        let mut tx = pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(MIGRATION_LOCK)
            .execute(&mut *tx)
            .await?;
        let recorded: Option<i32> =
            sqlx::query_scalar("SELECT version FROM schema_version WHERE version = $1")
                .bind(*version)
                .fetch_optional(&mut *tx)
                .await?;
        if recorded.is_some() {
            tx.rollback().await?;
            continue;
        }
        sqlx::raw_sql(migration.sql).execute(&mut *tx).await?;
        tx.commit().await?;
        applied.push(*version);
    }

    Ok(Applied {
        applied,
        now_current: applied_versions(pool).await?,
    })
}

/// The database is older than this build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaTooOld {
    /// The versions the database is missing, ascending.
    pub missing: Vec<i32>,
    /// The newest version this build knows about.
    pub expected: i32,
}

impl core::fmt::Display for SchemaTooOld {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "database schema is behind this build: missing version(s) {:?}, this build expects {}; \
             apply them before running (tools/m3-verify.ps1, or asura-db::migrations::apply_pending)",
            self.missing, self.expected
        )
    }
}

impl std::error::Error for SchemaTooOld {}

/// Refuse to run against a database that is older than this build.
///
/// This is the check a service performs at startup, before it binds a socket: a program that starts
/// against a stale schema fails on some later query, in a place that says nothing about the real
/// cause. Failing here names the missing versions instead.
///
/// A database that is *newer* than the build is allowed: an older program tolerating added columns
/// is ordinary, and refusing that would make rollback harder than it needs to be.
pub async fn ensure_current(pool: &PgPool) -> Result<(), EnsureError> {
    let missing = pending_versions(pool)
        .await
        .map_err(EnsureError::Database)?;
    if missing.is_empty() {
        return Ok(());
    }
    Err(EnsureError::TooOld(SchemaTooOld {
        missing,
        expected: LATEST_VERSION,
    }))
}

/// Why [`ensure_current`] refused.
#[derive(Debug)]
pub enum EnsureError {
    /// The database could not be asked. A startup check that cannot see the schema is not a pass.
    Database(sqlx::Error),
    /// The database is behind this build.
    TooOld(SchemaTooOld),
}

impl core::fmt::Display for EnsureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EnsureError::Database(err) => write!(f, "could not read the schema version: {err}"),
            EnsureError::TooOld(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for EnsureError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statements;

    #[test]
    fn the_versions_are_sequential_from_one() {
        let versions: Vec<i32> = MIGRATIONS.iter().map(|m| m.version).collect();
        let expected: Vec<i32> = (1..=MIGRATIONS.len() as i32).collect();
        assert_eq!(
            versions, expected,
            "migration versions must be 1..=n with no gaps: the runner and the record both rely on it"
        );
    }

    #[test]
    fn the_latest_version_is_the_last_migration() {
        assert_eq!(
            LATEST_VERSION,
            MIGRATIONS.last().expect("at least one migration").version,
            "LATEST_VERSION drifted from MIGRATIONS"
        );
    }

    #[test]
    fn every_migration_records_its_own_version() {
        // The record is written by the migration, not by the runner, so that the psql path and this
        // one agree. A migration that forgot its bookkeeping line would be applied forever.
        for migration in MIGRATIONS {
            let bookkeeping = format!("({}) ON CONFLICT (version) DO NOTHING", migration.version);
            let last = statements(migration.sql)
                .pop()
                .unwrap_or_else(|| panic!("migration {} is empty", migration.version));
            assert!(
                last.contains(&bookkeeping),
                "migration {} must end by recording itself; last statement was: {}",
                migration.version,
                last
            );
            assert!(
                last.contains("INSERT INTO schema_version"),
                "migration {} records itself in the wrong table",
                migration.version
            );
        }
    }

    #[test]
    fn every_migration_touches_schema_version_only_to_record_itself() {
        // schema_version is bookkeeping; a migration that changed its shape would break the record.
        // Creating the table (0001) and recording a version are the only two allowed mentions.
        for migration in MIGRATIONS {
            for statement in statements(migration.sql) {
                if !statement.contains("schema_version") {
                    continue;
                }
                let allowed = statement.contains("INSERT INTO schema_version")
                    || statement.contains("CREATE TABLE IF NOT EXISTS schema_version");
                assert!(
                    allowed,
                    "only the table's creation and version inserts may touch schema_version, found: {}",
                    statement.lines().next().unwrap_or("")
                );
            }
        }
    }
}
