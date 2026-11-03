//! Configuration: the one place that knows how this project finds its settings.
//!
//! Right now that is the database URL, which several programs and several tests need and which used
//! to be parsed in four separate places. Reading it once, in the crate whose job is the database,
//! means the rule is stated once: the environment wins, then the repository's `.env` (which
//! `tools/db-setup.ps1` writes and which is gitignored).
//!
//! The `.env` file is searched for from the crate's own directory upwards, then in the working
//! directory, because a test binary's working directory is the crate root while a developer's is the
//! repository root.

use std::path::{Path, PathBuf};

/// The name of the variable, used by the error messages.
pub const DATABASE_URL_VARIABLE: &str = "ASURA_DATABASE_URL";

/// The database URL: the environment first, then `.env`.
///
/// `None` means "not configured", which the tests treat as "skip" rather than "fail": a developer
/// without PostgreSQL should still be able to run the suite.
pub fn database_url() -> Option<String> {
    if let Ok(url) = std::env::var(DATABASE_URL_VARIABLE) {
        let trimmed = url.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }

    for path in dotenv_candidates() {
        if let Some(url) = read_from(&path) {
            return Some(url);
        }
    }
    None
}

/// The `.env` files to try, in order.
fn dotenv_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    // A test binary and a compiled binary both have CARGO_MANIFEST_DIR baked in, so this finds the
    // repository's `.env` from inside the crate: crates/<name>/../../.env.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(root) = manifest.parent().and_then(Path::parent) {
        candidates.push(root.join(".env"));
    }
    candidates.push(manifest.join(".env"));
    candidates.push(PathBuf::from(".env"));
    candidates
}

/// Read one variable out of a `.env`-shaped file.
fn read_from(path: &Path) -> Option<String> {
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
        if key.trim() != DATABASE_URL_VARIABLE {
            continue;
        }
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_can_be_read_from_a_dotenv_file() {
        let directory = std::env::temp_dir().join(format!("asura-config-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp directory");
        let path = directory.join(".env");
        std::fs::write(
            &path,
            "# a comment\n\nOTHER=1\nASURA_DATABASE_URL=\"postgres://example/db\"\n",
        )
        .expect("write");

        assert_eq!(
            read_from(&path),
            Some("postgres://example/db".to_string()),
            "quotes are stripped, and the key is matched exactly"
        );

        std::fs::write(&path, "ASURA_DATABASE_URL=\n").expect("write");
        assert_eq!(read_from(&path), None, "an empty value is not configured");

        std::fs::write(&path, "ASURA_DATABASE_URL_NOT=postgres://wrong/db\n").expect("write");
        assert_eq!(
            read_from(&path),
            None,
            "a longer name that merely starts the same way must not match"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        assert_eq!(read_from(Path::new("definitely-not-here.env")), None);
    }

    #[test]
    fn the_repository_env_is_among_the_candidates() {
        // The first candidate is the repository root, which is what a developer's shell will not
        // necessarily be in.
        let candidates = dotenv_candidates();
        assert!(
            candidates[0].ends_with(".env"),
            "candidates: {candidates:?}"
        );
        assert!(
            candidates[0]
                .ancestors()
                .any(|path| path.ends_with("ASURA")),
            "the first candidate should sit at the repository root: {:?}",
            candidates[0]
        );
    }
}
