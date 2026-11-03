//! Talking to `psql`: the supervisor's one external tool for the two setup commands.
//!
//! `seed` and `schema` are data plus `psql`, so they are the same on Windows and Linux: the SQL
//! files are the repository's own (`tools/seed-dev.sql`, `tools/map-config.sql`,
//! `crates/asura-db/migrations/*.sql`), and psql is found the same way - `ASURA_PSQL` when set,
//! then PATH, then the usual Windows install.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where psql is: `ASURA_PSQL`, then `psql` (or `psql.exe`) on PATH, then the PostgreSQL install
/// on Windows.
pub fn find() -> Result<PathBuf, String> {
    if let Some(explicit) = std::env::var_os("ASURA_PSQL") {
        if !explicit.is_empty() {
            return Ok(PathBuf::from(explicit));
        }
    }
    let name = if cfg!(windows) { "psql.exe" } else { "psql" };
    if let Some(on_path) = which_on_path(name) {
        return Ok(on_path);
    }
    #[cfg(windows)]
    if let Some(from_install) = windows_install() {
        return Ok(from_install);
    }
    Err(
        "psql not found: install PostgreSQL, add its bin directory to PATH, or set ASURA_PSQL"
            .into(),
    )
}

/// Apply one SQL file: `psql -v ON_ERROR_STOP=1 -q -f <file> <url>`.
///
/// The options come before the database URI because psql reads anything after the dbname as an
/// extra argument (the same trap the old scripts carried a comment about).
pub fn apply_file(psql: &Path, url: &str, file: &Path) -> Result<(), String> {
    let status = Command::new(psql)
        .args(["-v", "ON_ERROR_STOP=1", "-q", "-f"])
        .arg(file)
        .arg(url)
        .status()
        .map_err(|err| format!("cannot run {}: {err}", psql.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("psql failed on {} (exit {status})", file.display()))
    }
}

fn which_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The newest PostgreSQL install under `%ProgramFiles%\PostgreSQL`, if any.
#[cfg(windows)]
fn windows_install() -> Option<PathBuf> {
    let root = std::env::var_os("ProgramFiles").map(PathBuf::from)?;
    let root = root.join("PostgreSQL");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("bin").join("psql.exe"))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();
    candidates.pop()
}
