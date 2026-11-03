//! The supervisor's list: what this machine runs.
//!
//! The list is data, not code (`docs/16-implementation-plan.md`, S1): adding a world server,
//! changing a port, or moving a service to another machine is an edit in `config/`, not a program
//! change. TOML, because it is commentable, diffable, and needs no tooling to read.
//!
//! The database is *not* in this file. It is checked and waited for, never started (a Windows
//! service on Windows, systemd on Linux - "managed elsewhere"); its URL comes from `.env`
//! (`ASURA_DATABASE_URL`), the same place every other program reads it.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The file this machine's list lives in, inside `config/`.
pub const SERVICES_FILE: &str = "services.toml";

/// One service.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    /// The program. Relative to the repository root; `.exe` is appended when that is what exists.
    pub exe: String,
    /// Which kind this service is in the service registry (`auth`, `webgate`, `world`, ...).
    /// Defaults to the name before the first `-`: `world-8001` -> `world`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Arguments, verbatim.
    #[serde(default)]
    pub args: Vec<String>,
    /// The port the service binds. A stop or restart waits for it to be free before anything else
    /// binds it.
    pub port: u16,
    /// Other services that must be ready before this one starts.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// How "alive" is decided. Absent means "started is enough".
    #[serde(default)]
    pub health: Option<Health>,
    /// Where the service's output goes. Defaults to `logs/<name>.log`.
    #[serde(default)]
    pub log: Option<String>,
    /// `on-failure` (default), `always`, or `never`.
    #[serde(default = "default_restart")]
    pub restart: String,
    /// Attempts before the service is reported dead. Never unbounded (D1.3).
    #[serde(default = "default_restart_max")]
    pub restart_max: u32,
    /// Delay between attempts.
    #[serde(default = "default_restart_delay_ms")]
    pub restart_delay_ms: u64,
}

/// The health checks the supervisor understands.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Health {
    /// A TCP connection is accepted at this address.
    Tcp { tcp: String },
    /// World `world` has a fresh row in the `world` table.
    World { world: i32 },
}

fn default_restart() -> String {
    "on-failure".to_string()
}

fn default_restart_max() -> u32 {
    5
}

fn default_restart_delay_ms() -> u64 {
    2000
}

/// The machine-wide settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Where the supervisor answers `status`/`stop`/`restart`/`quit`. Loopback only.
    #[serde(default = "default_control")]
    pub control: String,
    /// How long a service may take to pass its health check before it is reported failed.
    #[serde(default = "default_health_timeout")]
    pub health_timeout_secs: u64,
}

fn default_control() -> String {
    "127.0.0.1:7711".to_string()
}

fn default_health_timeout() -> u64 {
    30
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            control: default_control(),
            health_timeout_secs: default_health_timeout(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    version: u32,
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    services: BTreeMap<String, ServiceConfig>,
}

/// The loaded list.
#[derive(Debug, Clone)]
pub struct Config {
    pub settings: Settings,
    pub services: BTreeMap<String, ServiceConfig>,
}

/// The path of this machine's list: `config/services.toml` under the repository root.
pub fn default_path() -> PathBuf {
    repo_root().join("config").join(SERVICES_FILE)
}

/// The repository root, from where this crate's `Cargo.toml` sits.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name>/.. is the repository root")
        .to_path_buf()
}

/// Read and validate the list.
pub fn load(path: &Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let config = parse(&text).map_err(|err| format!("{}: {err}", path.display()))?;
    config
        .validate()
        .map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(config)
}

/// Parse the list, without checking that the programs exist (the tests use this half).
pub fn parse(text: &str) -> Result<Config, String> {
    let raw: RawConfig = toml::from_str(text).map_err(|err| err.to_string())?;
    if raw.version != 1 {
        return Err(format!(
            "version {} is not supported (this supervisor reads version 1)",
            raw.version
        ));
    }
    Ok(Config {
        settings: raw.settings,
        services: raw.services,
    })
}

impl Config {
    /// Check that every service can be run: a name, a program that exists, dependencies that are
    /// listed, and a restart policy the supervisor knows.
    pub fn validate(&self) -> Result<(), String> {
        if self.services.is_empty() {
            return Err("no services are listed".into());
        }
        for (name, service) in &self.services {
            if !valid_name(name) {
                return Err(format!(
                    "{name:?} is not a service name (letters, digits, '-', '_')"
                ));
            }
            resolve_exe(&service.exe).map_err(|err| format!("{name}: {err}"))?;
            for dep in &service.depends_on {
                if !self.services.contains_key(dep) {
                    return Err(format!("{name} depends on {dep:?}, which is not listed"));
                }
            }
            if !matches!(service.restart.as_str(), "on-failure" | "always" | "never") {
                return Err(format!(
                    "{name}: restart must be on-failure, always, or never"
                ));
            }
        }
        self.order().map(|_| ())
    }

    /// Names in start order: a service after everything it depends on.
    pub fn order(&self) -> Result<Vec<String>, String> {
        let mut seen = HashSet::new();
        let mut visiting = Vec::new();
        let mut order = Vec::new();
        for name in self.services.keys() {
            self.visit(name, &mut visiting, &mut seen, &mut order)?;
        }
        Ok(order)
    }

    fn visit(
        &self,
        name: &str,
        visiting: &mut Vec<String>,
        seen: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<(), String> {
        if seen.contains(name) {
            return Ok(());
        }
        if visiting.iter().any(|visited| visited == name) {
            let mut chain = visiting.clone();
            chain.push(name.to_string());
            return Err(format!("dependency cycle: {}", chain.join(" -> ")));
        }
        let Some(service) = self.services.get(name) else {
            return Err(format!("{name:?} is not listed"));
        };
        visiting.push(name.to_string());
        for dep in &service.depends_on {
            self.visit(dep, visiting, seen, order)?;
        }
        visiting.pop();
        seen.insert(name.to_string());
        order.push(name.to_string());
        Ok(())
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The executable to run: the path as written, or the same path with `.exe` when that is what
/// exists (the same list then works on Windows and Linux).
pub fn resolve_exe(exe: &str) -> Result<PathBuf, String> {
    let path = Path::new(exe);
    if path.exists() {
        return Ok(path.to_path_buf());
    }
    let with_exe = PathBuf::from(format!("{}.exe", path.display()));
    if with_exe.exists() {
        return Ok(with_exe);
    }
    Err(format!(
        "{exe:?} does not exist (build first: `asura-supervisor build`)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
version = 1

[settings]
control = "127.0.0.1:7711"

[services.webgate]
exe = "prog"
args = ["--bind", "127.0.0.1:6601"]
port = 6601
health = { tcp = "127.0.0.1:6601" }
log = "logs/webgate.log"

[services.world]
exe = "prog"
port = 6113
depends_on = ["auth"]
health = { world = 8001 }

[services.auth]
exe = "prog"
port = 6112
restart = "never"
restart_max = 1
restart_delay_ms = 50
"#;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("asura-supervisor-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn the_example_parses_with_defaults() {
        let config = parse(EXAMPLE).expect("the example parses");
        assert_eq!(config.services.len(), 3);
        assert_eq!(
            config.services["webgate"].args,
            vec!["--bind", "127.0.0.1:6601"]
        );
        assert!(matches!(
            config.services["world"].health,
            Some(Health::World { world: 8001 })
        ));
        assert!(matches!(
            config.services["webgate"].health,
            Some(Health::Tcp { .. })
        ));
        assert_eq!(config.services["auth"].restart, "never");
        assert_eq!(
            config.services["webgate"].restart, "on-failure",
            "the default"
        );
        assert_eq!(config.services["webgate"].restart_max, 5, "the default");
        assert_eq!(config.settings.control, "127.0.0.1:7711");
    }

    #[test]
    fn order_starts_dependencies_first() {
        let config = parse(EXAMPLE).expect("parses");
        let order = config.order().expect("an order");
        let auth = order.iter().position(|name| name == "auth").expect("auth");
        let world = order
            .iter()
            .position(|name| name == "world")
            .expect("world");
        assert!(auth < world, "auth must start before world: {order:?}");
    }

    #[test]
    fn a_cycle_is_an_error() {
        let text = "version = 1\n\n[services.a]\nexe = \"prog\"\nport = 1\ndepends_on = [\"b\"]\n\n[services.b]\nexe = \"prog\"\nport = 2\ndepends_on = [\"a\"]\n";
        let config = parse(text).expect("parses");
        let err = config.order().expect_err("a cycle must fail");
        assert!(err.contains("cycle"), "error: {err}");
    }

    #[test]
    fn a_missing_dependency_is_an_error() {
        let text =
            "version = 1\n\n[services.a]\nexe = \"prog\"\nport = 1\ndepends_on = [\"nowhere\"]\n";
        let config = parse(text).expect("parses");
        assert!(config.order().is_err());
    }

    #[test]
    fn an_unknown_key_is_an_error() {
        let text = "version = 1\n\n[services.a]\nexe = \"prog\"\nport = 1\nbogus = 1\n";
        assert!(parse(text).is_err());
    }

    #[test]
    fn validate_refuses_a_program_that_does_not_exist() {
        let config = parse(EXAMPLE).expect("parses");
        let err = config.validate().expect_err("no prog");
        assert!(err.contains("does not exist"), "error: {err}");
    }

    #[test]
    fn validate_accepts_a_program_that_exists() {
        let dir = temp_dir("validate");
        let program = dir.join("prog");
        std::fs::write(&program, "exit 0\n").expect("write program");
        let text = format!(
            "version = 1\n\n[services.a]\nexe = '{}'\nport = 1\n",
            program.display()
        );
        let config = parse(&text).expect("parses");
        config.validate().expect("validate");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_exe_appends_exe_when_that_is_what_exists() {
        let dir = temp_dir("exe");
        let program = dir.join("prog.exe");
        std::fs::write(&program, "x").expect("write program");
        let resolved = resolve_exe(&format!("{}/prog", dir.display())).expect("resolves");
        assert!(
            resolved.ends_with("prog.exe"),
            "resolved: {}",
            resolved.display()
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
