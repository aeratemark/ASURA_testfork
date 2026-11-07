//! The numbers this machine runs at, and the files that hold them.
//!
//! These are the numbers a client's behaviour is measured against: how often the world ticks, how often
//! it asks whether the client is still there, how long login waits for a world's port, how old a
//! world's heartbeat may be before it counts as gone, and how far a client may place itself when it
//! cancels a walk. They were constants in four different crates (`asura-world`'s `connection.rs` and
//! `agent.rs`, `asura-auth`'s `travel.rs`, `asura-rules` for the freshness window), so changing one meant
//! editing Rust and rebuilding, and no single place answered "what cadence is this server using"
//! (`docs/38-layer-decisions.md`, D33).
//!
//! Two files, one concern each: `config/Server/intervals.json` for the cadences ([`Intervals`]) and
//! `config/Server/movement.json` for the movement tolerances ([`Movement`]), plus the spawn placement
//! (`config/Server/spawns.json`, [`Spawns`]). All are read once per process and then held, because
//! re-reading one mid-session would make one server run by two rules.
//!
//! Precedence, and it is deliberate: an environment variable overrides the file, and the file overrides
//! the built-in default. The env variable is what a one-off experiment uses; the file is what a machine
//! runs on; the default is what happens when a checkout has no `config/` yet.

use std::sync::OnceLock;

pub mod directional;
pub mod world;

/// Half the side of a legacy point doorway's box, in map units. An unreadable or invalid value
/// disables these boxes and reports why; measured wall endpoints do not need this value.
pub fn portal_box_half_extent() -> f32 {
    #[derive(serde::Deserialize)]
    struct Portals {
        point_box_half_extent: Described<f32>,
    }
    static VALUE: OnceLock<f32> = OnceLock::new();
    *VALUE.get_or_init(|| {
        let path = resolve("config/Server/portals.json");
        let result = std::fs::read_to_string(&path)
            .map_err(|err| err.to_string())
            .and_then(|text| serde_json::from_str::<Portals>(&text).map_err(|err| err.to_string()))
            .and_then(|config| {
                let value = config.point_box_half_extent.value;
                if value.is_finite() && value > 0.0 {
                    Ok(value)
                } else {
                    Err("point_box_half_extent must be positive and finite".to_string())
                }
            });
        match result {
            Ok(value) => value,
            Err(err) => {
                eprintln!(
                    "asura: {}: {err}; point doorway boxes disabled",
                    path.display()
                );
                0.0
            }
        }
    })
}

/// The file a machine's intervals are read from.
pub const DEFAULT_FILE: &str = "config/Server/intervals.json";

/// The variable that names a different file, for an experiment.
pub const FILE_VARIABLE: &str = "ASURA_INTERVALS";

/// The variable that overrides `world_max_age_secs` for one run. Spelled here, next to the file that
/// carries the default, so both readers name it the same way.
pub const MAX_AGE_VARIABLE: &str = "ASURA_WORLD_MAX_AGE_SECS";

/// Every interval this server runs at.
///
/// The defaults are the numbers the three crates already used, so a checkout with no file behaves
/// exactly as it did before the file existed. `deny_unknown_fields` is on purpose: a misspelled key in a
/// hand-edited file would otherwise be silently ignored, and the server would run at the default while
/// its operator believed otherwise.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Intervals {
    /// How often the world wakes up: the tick that carries elapsed time to the client.
    ///
    /// Measured from the live client (`recordings/client/feature/movement/movement-sniff.log`): a
    /// `0x001E` every ~100 ms, median 81 ms. The reference's own floor is 500 ms.
    pub tick_interval_ms: u64,
    /// How often the world asks the client whether it is still there. The reference's constant is 5000 ms
    /// (`GAME_SRV_TIME_BETWEEN_PING_MS`).
    pub ping_interval_secs: u64,
    /// The value the world answers a ping with. **Not a measurement**: GWPS sends 20 and this server has
    /// never verified it against our client.
    pub ping_value: u32,
    /// How long login waits for a candidate world's port to answer before dropping it from the list.
    /// A loopback world answers in microseconds; a silent one should not cost the client the operating
    /// system's connect timeout.
    pub world_probe_timeout_ms: u64,
    /// Free text for whoever opens the file, including where each number came from. Never read by the
    /// server: JSON has no comments, so the file carries one string instead.
    #[serde(default)]
    pub note: Option<String>,
    /// How stale a world''s heartbeat may be and still be offered, routed to, and believed.
    ///
    /// Two programs read it - the login path and the socket layer - so it lives in one place; disagreeing
    /// means a client is sent to a world the next step refuses (`docs/38`, D29).
    pub world_max_age_secs: f64,
    /// How long the exploration marker waits between two `0x008C` marks.
    pub cartography_mark_interval_ms: u64,
    /// How long after a load finishes the stored exploration replay starts.
    pub cartography_replay_delay_ms: u64,
}

impl Default for Intervals {
    fn default() -> Self {
        Intervals {
            tick_interval_ms: 100,
            ping_interval_secs: 5,
            ping_value: 20,
            world_probe_timeout_ms: 500,
            note: None,
            world_max_age_secs: 60.0,
            cartography_mark_interval_ms: 100,
            cartography_replay_delay_ms: 50,
        }
    }
}

impl Intervals {
    /// The tick as a duration.
    pub fn tick(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.tick_interval_ms)
    }

    /// The ping cadence as a duration.
    pub fn ping(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.ping_interval_secs)
    }

    /// The probe timeout as a duration.
    pub fn probe_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.world_probe_timeout_ms)
    }

    /// The exploration mark cadence as a duration.
    pub fn cartography_mark_interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.cartography_mark_interval_ms)
    }

    /// The wait after a load before the stored-exploration replay starts.
    pub fn cartography_replay_delay(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.cartography_replay_delay_ms)
    }

    /// Parse the file's text. Every key is optional: a file may state only what this machine changes.
    pub fn from_json(text: &str) -> Result<Intervals, String> {
        serde_json::from_str(text).map_err(|err| err.to_string())
    }
}

/// Where the values in force came from, so a start-up can say it once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Read from this file.
    File(String),
    /// No file: the built-in defaults are in force.
    Defaults,
}

/// The intervals in force, and where they came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// The values.
    pub intervals: Intervals,
    /// Where they came from.
    pub source: Source,
}

/// The workspace root, as this crate's own manifest directory sees it.
///
/// A service is started from the repository root and finds `config/` in its working directory; a test is
/// started from the crate's directory and does not. Both are the same file, so both are tried, working
/// directory first.
fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Resolve a configured path: absolute stays as it is, and a relative one is tried against the working
/// directory and then against the workspace root.
fn resolve(path: &str) -> std::path::PathBuf {
    let candidate = std::path::Path::new(path);
    if candidate.is_absolute() || candidate.exists() {
        return candidate.to_path_buf();
    }
    let rooted = workspace_root().join(candidate);
    if rooted.exists() {
        return rooted;
    }
    candidate.to_path_buf()
}

/// The path in force: `ASURA_INTERVALS`, or [`DEFAULT_FILE`].
pub fn path() -> String {
    match std::env::var(FILE_VARIABLE) {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => DEFAULT_FILE.to_string(),
    }
}

/// Read the file. A missing file is the defaults, because a checkout without `config/` still runs; a file
/// that exists and does not parse is an error, because a server running at numbers nobody chose is worse
/// than a server that refuses to start and says why.
pub fn load() -> Result<Loaded, String> {
    let path = resolve(&path());
    let path = path.display().to_string();
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Loaded {
            intervals: Intervals::from_json(&text)
                .map_err(|err| format!("{path} is not a valid intervals file: {err}"))?,
            source: Source::File(path),
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Loaded {
            intervals: Intervals::default(),
            source: Source::Defaults,
        }),
        Err(err) => Err(format!("{path} is not readable: {err}")),
    }
}

/// The process's intervals, read once.
///
/// Every value is a cadence, so it is read at first use and then held: a file edited mid-session does not
/// make one connection tick at 100 ms and the next at 200.
pub fn intervals() -> &'static Intervals {
    static IN_FORCE: OnceLock<Intervals> = OnceLock::new();
    IN_FORCE.get_or_init(|| match load() {
        Ok(loaded) => {
            match &loaded.source {
                Source::File(path) => eprintln!("asura: intervals from {path}"),
                Source::Defaults => eprintln!(
                    "asura: no intervals file at {}; using the built-in defaults",
                    path()
                ),
            }
            loaded.intervals
        }
        Err(err) => {
            // A broken file is not worth crashing a running world over here, but it must be visible:
            // silently falling back would run the server at numbers nobody asked for.
            eprintln!("asura: {err}; using the built-in defaults");
            Intervals::default()
        }
    })
}

/// The file that holds this machine's movement tolerances.
pub const MOVEMENT_FILE: &str = "config/Server/movement.json";

/// The variable that names a different movement file, for an experiment.
pub const MOVEMENT_VARIABLE: &str = "ASURA_MOVEMENT";

/// The movement tolerances this server judges a client's reports by.
///
/// Same shape and same precedence as [`Intervals`]: an environment variable overrides the file, the file
/// overrides the built-in default, and the built-in default is what a checkout with no `config/` runs on.
/// Read once per process, because a tolerance that changed mid-session would judge two stops in one walk
/// by two rules.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Movement {
    /// How far a client may place itself when it cancels a walk and still be believed.
    pub max_allowed_correction: Described<f32>,
}

impl Default for Movement {
    fn default() -> Self {
        Movement {
            max_allowed_correction: Described {
                value: 100.0,
                doc: None,
                immutable: false,
            },
        }
    }
}

impl Movement {
    /// Parse the file's text.
    pub fn from_json(text: &str) -> Result<Movement, String> {
        serde_json::from_str(text).map_err(|err| err.to_string())
    }

    /// The correction ceiling, as the number rather than the wrapper.
    pub fn max_allowed_correction(&self) -> f32 {
        self.max_allowed_correction.value
    }
}

/// The movement tolerances in force, read once per process.
pub fn movement() -> &'static Movement {
    static IN_FORCE: OnceLock<Movement> = OnceLock::new();
    IN_FORCE.get_or_init(|| {
        let path = match std::env::var(MOVEMENT_VARIABLE) {
            Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => MOVEMENT_FILE.to_string(),
        };
        let path = resolve(&path).display().to_string();
        match std::fs::read_to_string(&path) {
            Ok(text) => match Movement::from_json(&text) {
                Ok(movement) => {
                    eprintln!("asura: movement tolerances from {path}");
                    movement
                }
                Err(err) => {
                    // A broken file is not worth crashing a running world over, but it must be visible:
                    // silently falling back would judge every stop by a number nobody chose.
                    eprintln!("asura: {path} is not a valid movement file: {err}; using the built-in defaults");
                    Movement::default()
                }
            },
            Err(err) => {
                eprintln!(
                    "asura: movement file {path} is not readable: {err}; using the built-in defaults"
                );
                Movement::default()
            }
        }
    })
}

/// The file that holds this machine's spawn placement.
pub const SPAWNS_FILE: &str = "config/Server/spawns.json";

/// The variable that names a different spawns file, for an experiment.
pub const SPAWNS_VARIABLE: &str = "ASURA_SPAWNS";

/// How this machine places a character around the spawn point it entered at.
///
/// Same shape and same precedence as [`Movement`]: an environment variable overrides the file, the
/// file overrides the built-in default, and the built-in default is what a checkout with no
/// `config/` runs on. Read once per process, because a radius that changed mid-session would spread
/// one arrival and stack the next.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spawns {
    /// How far from its spawn point a character lands: the radius of the circle the arrival is
    /// scattered over (`crates/asura-world/src/agent.rs`, `Standing::at_spawn`).
    pub spawn_scatter: Described<f32>,
}

impl Default for Spawns {
    fn default() -> Self {
        Spawns {
            spawn_scatter: Described {
                value: 250.0,
                doc: None,
                immutable: false,
            },
        }
    }
}

impl Spawns {
    /// Parse the file's text.
    pub fn from_json(text: &str) -> Result<Spawns, String> {
        serde_json::from_str(text).map_err(|err| err.to_string())
    }

    /// The scatter radius, as the number rather than the wrapper.
    pub fn spawn_scatter(&self) -> f32 {
        self.spawn_scatter.value
    }
}

/// The spawn placement in force, read once per process.
pub fn spawns() -> &'static Spawns {
    static IN_FORCE: OnceLock<Spawns> = OnceLock::new();
    IN_FORCE.get_or_init(|| {
        let path = match std::env::var(SPAWNS_VARIABLE) {
            Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => SPAWNS_FILE.to_string(),
        };
        let path = resolve(&path).display().to_string();
        match std::fs::read_to_string(&path) {
            Ok(text) => match Spawns::from_json(&text) {
                Ok(spawns) => {
                    eprintln!("asura: spawn placement from {path}");
                    spawns
                }
                Err(err) => {
                    // A broken file is not worth crashing a running world over, but it must be visible:
                    // silently falling back would scatter every arrival over a radius nobody chose.
                    eprintln!("asura: {path} is not a valid spawns file: {err}; using the built-in defaults");
                    Spawns::default()
                }
            },
            Err(err) => {
                eprintln!(
                    "asura: spawns file {path} is not readable: {err}; using the built-in defaults"
                );
                Spawns::default()
            }
        }
    })
}

/// The file that holds this machine's cartography behaviour.
pub const CARTOGRAPHY_FILE: &str = "config/Server/cartography.json";

/// The variable that names a different cartography file, for an experiment.
pub const CARTOGRAPHY_VARIABLE: &str = "ASURA_CARTOGRAPHY_CONFIG";

/// How this machine serves a character's exploration grid.
///
/// Same shape and same precedence as [`Movement`]: an environment variable overrides the file, the
/// file overrides the built-in default.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cartography {
    /// Whether the `0x008B`/`0x008A` init carries the character's saved cells.
    ///
    /// The working path until a sparse stored init is proven against the client is false: the init
    /// sends the empty grid and the stored cells are replayed as `0x008C` marks.
    pub stored_init: Described<bool>,
}

impl Default for Cartography {
    fn default() -> Self {
        Cartography {
            stored_init: Described {
                value: false,
                doc: None,
                immutable: false,
            },
        }
    }
}

impl Cartography {
    /// Parse the file's text.
    pub fn from_json(text: &str) -> Result<Cartography, String> {
        serde_json::from_str(text).map_err(|err| err.to_string())
    }

    /// Whether the load init carries the stored grid.
    pub fn stored_init(&self) -> bool {
        self.stored_init.value
    }
}

/// The cartography behaviour in force, read once per process.
pub fn cartography() -> &'static Cartography {
    static IN_FORCE: OnceLock<Cartography> = OnceLock::new();
    IN_FORCE.get_or_init(|| {
        let path = match std::env::var(CARTOGRAPHY_VARIABLE) {
            Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => CARTOGRAPHY_FILE.to_string(),
        };
        let path = resolve(&path).display().to_string();
        match std::fs::read_to_string(&path) {
            Ok(text) => match Cartography::from_json(&text) {
                Ok(cartography) => {
                    eprintln!("asura: cartography from {path}");
                    cartography
                }
                Err(err) => {
                    eprintln!("asura: {path} is not a valid cartography file: {err}; using the built-in defaults");
                    Cartography::default()
                }
            },
            Err(err) => {
                eprintln!(
                    "asura: cartography file {path} is not readable: {err}; using the built-in defaults"
                );
                Cartography::default()
            }
        }
    })
}

/// The file that holds the values a world falls back on when a row says nothing.
pub const DEFAULTS_FILE: &str = "config/Server/defaults.json";

/// The variable that names a different defaults file.
pub const DEFAULTS_VARIABLE: &str = "ASURA_DEFAULTS";

/// One documented value: the number, and the sentence that says what it means.
///
/// The documentation travels with the value because JSON has no comments and a file of bare numbers is a
/// file nobody can review. A key without `doc` is accepted - a name can be enough - but the base game's
/// own file documents every one.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Described<T> {
    /// The value itself.
    pub value: T,
    /// What it means, in the words the person reading the file needs.
    #[serde(default)]
    pub doc: Option<String>,
    /// A note to whoever opens the file: `true` means do not change this value - it is one of the
    /// client's own numbers, and changing it breaks what the client expects. It is advice, not a check:
    /// the server reads the value either way. Absent means the value is the owner's to change
    /// (`docs/38-layer-decisions.md`, D35).
    #[serde(default)]
    pub immutable: bool,
}

/// The file that holds the login batch's settings payload.
pub const ACCOUNT_SETTINGS_FILE: &str = "config/Server/account_settings.json";

/// The variable that names a different account-settings file.
pub const ACCOUNT_SETTINGS_VARIABLE: &str = "ASURA_ACCOUNT_SETTINGS";

/// The login batch's settings blob.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct AccountSettings {
    /// The blob, as a hex string.
    pub data: HexBytes,
}

impl AccountSettings {
    /// Parse the file's text.
    pub fn from_json(text: &str) -> Result<AccountSettings, String> {
        serde_json::from_str(text).map_err(|err| err.to_string())
    }

    /// The bytes the login is answered with.
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        self.data.bytes()
    }
}

/// The account-settings blob in force, read once per process.
pub fn account_settings() -> Result<&'static AccountSettings, String> {
    static IN_FORCE: OnceLock<Result<AccountSettings, String>> = OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let path = match std::env::var(ACCOUNT_SETTINGS_VARIABLE) {
                Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
                _ => ACCOUNT_SETTINGS_FILE.to_string(),
            };
            let path = resolve(&path).display().to_string();
            match std::fs::read_to_string(&path) {
                Ok(text) => AccountSettings::from_json(&text)
                    .map_err(|err| format!("{path} is not a valid account-settings file: {err}"))
                    .map(|settings| {
                        eprintln!("asura: account settings from {path}");
                        settings
                    }),
                Err(err) => Err(format!("{path} is not readable: {err}")),
            }
        })
        .as_ref()
        .map_err(|err| err.clone())
}

/// The file that holds the two byte strings of the client's file check.
pub const FILE_CHECK_FILE: &str = "config/Server/file_check.json";

/// The variable that names a different file-check file.
pub const FILE_CHECK_VARIABLE: &str = "ASURA_FILE_CHECK";

/// One byte string and what it is for.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct HexBytes {
    /// The bytes, as space-separated hex pairs.
    pub hex: String,
    /// What the string is, and where the numbers came from.
    #[serde(default)]
    pub doc: Option<String>,
}

impl HexBytes {
    /// The bytes themselves.
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        self.hex
            .split_whitespace()
            .map(|pair| u8::from_str_radix(pair, 16).map_err(|err| format!("{pair:?}: {err}")))
            .collect()
    }
}

/// The client's file check: the question it asks and the answer this server gives.
///
/// The bytes are the client's own - it compares the whole question and parses the answer as six
/// `Gw.dat` file ids - so they are configuration rather than code: they change when the client's files
/// change, and `docs/19-running.md` has the procedure for refreshing them.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct FileCheck {
    /// The 21-byte question.
    pub request: HexBytes,
    /// The 32-byte answer.
    pub response: HexBytes,
}

impl FileCheck {
    /// Parse the file's text.
    pub fn from_json(text: &str) -> Result<FileCheck, String> {
        serde_json::from_str(text).map_err(|err| err.to_string())
    }

    /// The question's bytes.
    pub fn request_bytes(&self) -> Result<Vec<u8>, String> {
        self.request.bytes()
    }

    /// The answer's bytes.
    pub fn response_bytes(&self) -> Result<Vec<u8>, String> {
        self.response.bytes()
    }
}

/// The file check in force, read once per process.
///
/// A missing file is an error: a socket layer that cannot answer the check refuses the client outright,
/// which is the failure this file exists to prevent, so it says so at start-up rather than at the first
/// login.
pub fn file_check() -> Result<&'static FileCheck, String> {
    static IN_FORCE: OnceLock<Result<FileCheck, String>> = OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let path = match std::env::var(FILE_CHECK_VARIABLE) {
                Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
                _ => FILE_CHECK_FILE.to_string(),
            };
            let path = resolve(&path).display().to_string();
            match std::fs::read_to_string(&path) {
                Ok(text) => FileCheck::from_json(&text)
                    .map_err(|err| format!("{path} is not a valid file-check file: {err}"))
                    .map(|check| {
                        eprintln!("asura: file check from {path}");
                        check
                    }),
                Err(err) => Err(format!("{path} is not readable: {err}")),
            }
        })
        .as_ref()
        .map_err(|err| err.clone())
}

/// The values a world falls back on when its rows say nothing.
///
/// Every key is required and there is no built-in value: a world that placed a character somewhere
/// nobody configured would be running on a number that does not exist (`docs/38-layer-decisions.md`,
/// D34).
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    /// Where a character with no placement history is put. Written by character creation, by a scripted
    /// event and by travelling for every other case; this is the fallback for a row where none of them
    /// happened.
    pub default_outpost: Described<i32>,
    /// The creation instance's map id: the destination that is a conversation rather than a place, and
    /// the value a row carries when nothing has been written yet.
    pub creation_map: Described<i32>,
    /// The client's `MapType` value for a town or outpost.
    pub map_type_main_town: Described<i32>,
    /// The client's `MapType` value for an explorable area.
    pub map_type_main_explorable: Described<i32>,
    /// The client's `MapType` value for the character-creation instance.
    pub map_type_character_creation: Described<i32>,
    /// The unlocked-professions bitmap the creation screen is told, before a character exists.
    pub creation_screen_unlocked_professions: Described<u32>,
    /// How many UTF-16 units of a character name the confirm keeps.
    pub name_max_units: Described<usize>,
}

impl Defaults {
    /// Parse the file's text. A file without `default_outpost` is an error, not a default.
    pub fn from_json(text: &str) -> Result<Defaults, String> {
        serde_json::from_str(text).map_err(|err| err.to_string())
    }

    /// The fallback outpost, as the number rather than the wrapper.
    pub fn default_outpost(&self) -> i32 {
        self.default_outpost.value
    }

    /// The creation instance's map id, as the number rather than the wrapper.
    pub fn creation_map(&self) -> i32 {
        self.creation_map.value
    }

    /// The keys this file marks immutable, in a stable order, for the start-up line.
    ///
    /// Only the client's own numbers are here; every other value in the file is the owner's to change.
    pub fn immutable_keys(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        if self.creation_map.immutable {
            keys.push("creation_map");
        }
        if self.map_type_main_town.immutable {
            keys.push("map_type_main_town");
        }
        if self.map_type_main_explorable.immutable {
            keys.push("map_type_main_explorable");
        }
        if self.map_type_character_creation.immutable {
            keys.push("map_type_character_creation");
        }
        keys
    }

    /// The creation screen's unlocked-professions bitmap.
    pub fn creation_screen_unlocked_professions(&self) -> u32 {
        self.creation_screen_unlocked_professions.value
    }

    /// How many UTF-16 units of a character name are kept.
    pub fn name_max_units(&self) -> usize {
        self.name_max_units.value
    }

    /// The three map kinds the placement rule asks about, as one value.
    pub fn map_kinds(&self) -> asura_protocol::game::MapKinds {
        asura_protocol::game::MapKinds {
            main_town: self.map_type_main_town.value,
            main_explorable: self.map_type_main_explorable.value,
            character_creation: self.map_type_character_creation.value,
        }
    }
}

/// The defaults in force, read once per process.
///
/// There is no built-in value: a missing file, an unreadable one, or one without `default_outpost` is an
/// error, and the service that needs it refuses to start.
pub fn defaults() -> Result<&'static Defaults, String> {
    static IN_FORCE: OnceLock<Result<Defaults, String>> = OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let path = match std::env::var(DEFAULTS_VARIABLE) {
                Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
                _ => DEFAULTS_FILE.to_string(),
            };
            let path = resolve(&path).display().to_string();
            match std::fs::read_to_string(&path) {
                Ok(text) => Defaults::from_json(&text)
                    .map_err(|err| format!("{path} is not a valid defaults file: {err}"))
                    .map(|defaults| {
                        eprintln!("asura: defaults from {path}");
                        let fixed = defaults.immutable_keys();
                        if !fixed.is_empty() {
                            eprintln!(
                                "asura: immutable (the client's contract, stated for review): {}",
                                fixed.join(", ")
                            );
                        }
                        defaults
                    }),
                Err(err) => Err(format!("{path} is not readable: {err}")),
            }
        })
        .as_ref()
        .map_err(|err| err.clone())
}

/// The file that holds what a new guild starts with and what guild requests cost.
pub const GUILDS_FILE: &str = "config/Server/guilds.json";

/// The variable that names a different guilds file.
pub const GUILDS_VARIABLE: &str = "ASURA_GUILDS";

/// What a new guild starts with, the `0x0133` counter's start and step, and the two prices.
///
/// Same rule as [`Defaults`]: every key is required and there is no built-in value. A world whose file is
/// missing refuses guild requests with the reason logged, rather than making guilds out of numbers nobody
/// chose (`docs/41-guild-system-research.md`).
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guilds {
    /// Gold a creation costs. A placeholder: not charged.
    pub creation_price: Described<u32>,
    /// Gold an invitation costs. A placeholder: not charged.
    pub invite_fee: Described<u32>,
    /// `0x0121`'s feature byte on a new guild.
    pub new_guild_feature: Described<u8>,
    /// `0x0121`'s territory byte on a new guild.
    pub new_guild_territory: Described<u8>,
    /// A new guild's allegiance.
    pub new_guild_allegiance: Described<u8>,
    /// A new guild's ladder rating.
    pub new_guild_rating: Described<u32>,
    /// A new guild's ladder position.
    pub new_guild_rank: Described<u32>,
    /// The `0x0133` value of the creation batch, and the counter's start.
    pub revision_on_create: Described<u32>,
    /// How far the counter moves per later change.
    pub revision_step: Described<u32>,
}

impl Guilds {
    /// Parse the file's text.
    pub fn from_json(text: &str) -> Result<Guilds, String> {
        serde_json::from_str(text).map_err(|err| err.to_string())
    }
}

/// The guild settings in force, read once per process. An error when the file is missing or broken.
pub fn guilds() -> Result<&'static Guilds, String> {
    static IN_FORCE: OnceLock<Result<Guilds, String>> = OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let path = match std::env::var(GUILDS_VARIABLE) {
                Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
                _ => GUILDS_FILE.to_string(),
            };
            let path = resolve(&path).display().to_string();
            match std::fs::read_to_string(&path) {
                Ok(text) => Guilds::from_json(&text)
                    .map_err(|err| format!("{path} is not a valid guilds file: {err}"))
                    .map(|guilds| {
                        eprintln!("asura: guild settings from {path}");
                        guilds
                    }),
                Err(err) => Err(format!("{path} is not readable: {err}")),
            }
        })
        .as_ref()
        .map_err(|err| err.clone())
}

/// The file that holds the title definitions.
pub const TITLES_FILE: &str = "config/Server/titles.json";

/// The variable that names a different title file, for an experiment.
pub const TITLES_VARIABLE: &str = "ASURA_TITLES_CONFIG";

/// The title definitions in force, as the rules take them.
///
/// Experimental (`docs/40-titles-system-research.md`): a missing, unreadable, malformed or invalid file is
/// the feature switched off - logged, never fatal - because a world that sent a title definition the client
/// cannot draw would kill the client at render time, and a world that sends none is what ran before titles
/// existed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Titles {
    enabled: bool,
    titles: Vec<asura_rules::titles::Title>,
}

impl Titles {
    /// Parse and check the file's text. Any problem `asura_rules::titles::validate` finds refuses the whole
    /// file: one rank id under two records reaches the client's single rank table either way.
    pub fn from_json(text: &str) -> Result<Titles, String> {
        let file: TitlesFile = serde_json::from_str(text).map_err(|err| err.to_string())?;
        let titles = file
            .titles
            .into_iter()
            .map(TitleEntry::into_rule)
            .collect::<Result<Vec<_>, String>>()?;
        let problems = asura_rules::titles::validate(&titles);
        if !problems.is_empty() {
            return Err(problems
                .iter()
                .map(|problem| format!("title {}: {}", problem.title_id, problem.what))
                .collect::<Vec<_>>()
                .join("; "));
        }
        Ok(Titles {
            enabled: file.enabled.value,
            titles,
        })
    }

    /// Whether a load sends titles and display requests are answered.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The definitions, empty when the feature is off.
    pub fn definitions(&self) -> &[asura_rules::titles::Title] {
        if self.enabled {
            &self.titles
        } else {
            &[]
        }
    }
}

/// `config/Server/titles.json` as written.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TitlesFile {
    /// What the file is; for the reader.
    #[serde(default)]
    #[allow(dead_code)]
    doc: Option<String>,
    /// The switch.
    enabled: Described<bool>,
    /// One sentence per key; for the reader.
    #[serde(default)]
    #[allow(dead_code)]
    fields: Option<serde_json::Value>,
    /// The definitions.
    titles: Vec<TitleEntry>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum ScopeEntry {
    Character,
    Account,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TitleEntry {
    title_id: u32,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    scope: ScopeEntry,
    flags: u32,
    mode_flags: u32,
    base_rank_id: u32,
    progress_template: String,
    description_template: String,
    display_from_rank: Option<u32>,
    ranks: Vec<RankEntry>,
    maxed: Option<RankEntry>,
    #[serde(default)]
    earned_by: Option<EarningEntry>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum EarningEntry {
    PreSearingExploration,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RankEntry {
    rank_id: u32,
    flags: u32,
    value: u32,
    #[serde(default)]
    minimum: u32,
    name: String,
}

/// A coded string as the file writes it: space-separated hex UTF-16 units, `0x` optional.
fn coded_units(text: &str) -> Result<Vec<u16>, String> {
    text.split_whitespace()
        .map(|unit| {
            let digits = unit.trim_start_matches("0x").trim_start_matches("0X");
            u16::from_str_radix(digits, 16).map_err(|err| format!("{unit:?}: {err}"))
        })
        .collect()
}

impl RankEntry {
    fn into_rule(self) -> Result<asura_rules::titles::Rank, String> {
        Ok(asura_rules::titles::Rank {
            rank_id: self.rank_id,
            flags: self.flags,
            value: self.value,
            minimum: self.minimum,
            name: coded_units(&self.name)
                .map_err(|err| format!("rank {} name: {err}", self.rank_id))?,
        })
    }
}

impl TitleEntry {
    fn into_rule(self) -> Result<asura_rules::titles::Title, String> {
        let title_id = self.title_id;
        let field = |name: &str, text: &str| {
            coded_units(text).map_err(|err| format!("title {title_id} {name}: {err}"))
        };
        Ok(asura_rules::titles::Title {
            title_id,
            names: self.name.into_iter().chain(self.aliases).collect(),
            scope: match self.scope {
                ScopeEntry::Character => asura_rules::titles::Scope::Character,
                ScopeEntry::Account => asura_rules::titles::Scope::Account,
            },
            flags: self.flags,
            mode_flags: self.mode_flags,
            base_rank_id: self.base_rank_id,
            progress_template: field("progress_template", &self.progress_template)?,
            description_template: field("description_template", &self.description_template)?,
            ranks: self
                .ranks
                .into_iter()
                .map(RankEntry::into_rule)
                .collect::<Result<Vec<_>, String>>()?,
            maxed: self.maxed.map(RankEntry::into_rule).transpose()?,
            display_from_rank: self.display_from_rank,
            earned_by: self.earned_by.map(|earning| match earning {
                EarningEntry::PreSearingExploration => {
                    asura_rules::titles::Earning::PreSearingExploration
                }
            }),
        })
    }
}

/// The title definitions in force, read once per process. Every failure is the feature off, with the
/// reason on stderr.
pub fn titles() -> &'static Titles {
    static IN_FORCE: OnceLock<Titles> = OnceLock::new();
    IN_FORCE.get_or_init(|| {
        let path = match std::env::var(TITLES_VARIABLE) {
            Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => TITLES_FILE.to_string(),
        };
        let path = resolve(&path).display().to_string();
        match std::fs::read_to_string(&path) {
            Ok(text) => match Titles::from_json(&text) {
                Ok(titles) => {
                    eprintln!(
                        "asura: titles from {path}: {} definition(s), {}",
                        titles.titles.len(),
                        if titles.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    );
                    titles
                }
                Err(err) => {
                    eprintln!("asura: {path} is not a valid title file, titles are off: {err}");
                    Titles::default()
                }
            },
            Err(err) => {
                eprintln!("asura: title file {path} is not readable, titles are off: {err}");
                Titles::default()
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLES_COMMITTED: &str = include_str!("../../../config/Server/titles.json");

    #[test]
    fn the_committed_title_file_parses_and_validates() {
        let titles = Titles::from_json(TITLES_COMMITTED).expect("the committed file is valid");
        assert!(titles.enabled());
        let ldoa = asura_rules::titles::find(titles.definitions(), 21).expect("title 21");
        assert_eq!(ldoa.ranks[1].rank_id, 177);
        assert!(ldoa.names.iter().any(|name| name == "ldoa"));
        assert_eq!(
            ldoa.ranks[1].name,
            vec![0x8101, 0x5A05, 0xA50B, 0x8B97, 0x2B8E]
        );
        assert_eq!(
            ldoa.progress_template,
            vec![0x8101, 0x5A03, 0x9239, 0xB2FE, 0x4878]
        );
    }

    #[test]
    fn a_title_file_with_a_crash_rule_broken_is_refused_whole() {
        let broken = TITLES_COMMITTED.replace("\"title_id\": 21", "\"title_id\": 48");
        let err = Titles::from_json(&broken).expect_err("title 48 is past the client's table");
        assert!(err.contains("title 48"), "{err}");
    }

    #[test]
    fn a_disabled_title_file_defines_nothing() {
        let off = TITLES_COMMITTED.replacen("\"value\": true", "\"value\": false", 1);
        let titles = Titles::from_json(&off).expect("parses");
        assert!(!titles.enabled());
        assert!(titles.definitions().is_empty());
    }

    /// The committed file, parsed the way a service parses it.
    const COMMITTED: &str = include_str!("../../../config/Server/intervals.json");

    /// The committed account-settings file, parsed the way the login path parses it.
    const ACCOUNT_SETTINGS_COMMITTED: &str =
        include_str!("../../../config/Server/account_settings.json");

    #[test]
    fn the_committed_account_settings_is_the_blob_a_login_is_answered_with() {
        let settings = AccountSettings::from_json(ACCOUNT_SETTINGS_COMMITTED)
            .expect("the committed file parses");
        let bytes = settings.bytes().expect("hex");
        assert_eq!(bytes.len(), 591, "the blob the reference sends");
        assert_eq!(&bytes[..2], &[0x08, 0x00], "the blob's own first field");
    }

    /// The committed file-check file, parsed the way the socket layer parses it.
    const FILE_CHECK_COMMITTED: &str = include_str!("../../../config/Server/file_check.json");

    #[test]
    fn the_committed_file_check_is_the_two_strings_the_client_expects() {
        let check = FileCheck::from_json(FILE_CHECK_COMMITTED).expect("the committed file parses");
        let request = check.request_bytes().expect("hex");
        let response = check.response_bytes().expect("hex");
        assert_eq!(request.len(), 21, "the question the client compares whole");
        assert_eq!(response.len(), 32, "the answer it parses");
        assert_eq!(
            &response[..4],
            &[0xF1, 0x02, 0x20, 0x00],
            "header 0x02F1, length 0x2000"
        );
        assert!(check.request.doc.is_some() && check.response.doc.is_some());
    }

    #[test]
    fn a_byte_string_that_is_not_hex_is_an_error() {
        let err = FileCheck::from_json(r#"{"request": {"hex": "zz"}, "response": {"hex": "f1"}}"#)
            .expect("parses")
            .request_bytes()
            .expect_err("refused");
        assert!(err.contains("zz"), "said: {err}");
    }

    /// The committed defaults file, parsed the way the services parse it.
    const DEFAULTS_COMMITTED: &str = include_str!("../../../config/Server/defaults.json");

    /// The committed movement file, parsed the way `asura-world` parses it.
    const MOVEMENT_COMMITTED: &str = include_str!("../../../config/Server/movement.json");

    #[test]
    fn the_committed_movement_file_states_the_ceiling() {
        // The owner's number lives in the file, so the file is what is asserted. The `doc` sentence is
        // required for the same reason every other server key carries one: JSON has no comments.
        let movement = Movement::from_json(MOVEMENT_COMMITTED).expect("the committed file parses");
        assert_eq!(
            movement.max_allowed_correction(),
            150.0,
            "the ceiling this machine runs at"
        );
        assert!(
            movement
                .max_allowed_correction
                .doc
                .as_deref()
                .is_some_and(|doc| !doc.trim().is_empty()),
            "the file says what the number means"
        );
        assert_ne!(
            movement.max_allowed_correction(),
            Movement::default().max_allowed_correction(),
            "and it is not the reference fallback, or the file would be pointless"
        );
    }

    #[test]
    fn a_movement_key_this_crate_does_not_know_is_refused() {
        let err = Movement::from_json(r#"{"max_allowed_correction_x": {"value": 1}}"#)
            .expect_err("a misspelled key would otherwise run at the fallback");
        assert!(err.contains("unknown field"), "{err}");
    }

    /// The committed spawns file, parsed the way `asura-world` parses it.
    const SPAWNS_COMMITTED: &str = include_str!("../../../config/Server/spawns.json");

    #[test]
    fn the_committed_spawns_file_states_and_documents_the_scatter() {
        // The owner's radius lives in the file, so the file is what is asserted; the `doc` sentence is
        // required for the same reason every other server key carries one.
        let spawns = Spawns::from_json(SPAWNS_COMMITTED).expect("the committed file parses");
        assert_eq!(
            spawns.spawn_scatter(),
            250.0,
            "the radius this machine runs at"
        );
        assert!(
            spawns
                .spawn_scatter
                .doc
                .as_deref()
                .is_some_and(|doc| doc.contains("spawn point")),
            "the file says what the number means"
        );
        assert!(
            !spawns.spawn_scatter.immutable,
            "it is the owner's number, not the client's"
        );
    }

    #[test]
    fn the_committed_file_states_the_defaults() {
        // The file and the defaults cannot drift: a change to either turns this red. The note is free
        // text, so only the numbers are compared.
        let from_file = Intervals::from_json(COMMITTED).expect("the committed file parses");
        let defaults = Intervals::default();
        assert_eq!(from_file.tick_interval_ms, defaults.tick_interval_ms);
        assert_eq!(from_file.ping_interval_secs, defaults.ping_interval_secs);
        assert_eq!(from_file.ping_value, defaults.ping_value);
        assert_eq!(
            from_file.world_probe_timeout_ms,
            defaults.world_probe_timeout_ms
        );
        assert_eq!(from_file.world_max_age_secs, defaults.world_max_age_secs);
        assert!(
            from_file.note.is_some(),
            "the file says where its numbers came from"
        );
    }

    #[test]
    fn the_committed_defaults_file_states_and_documents_the_outpost() {
        let defaults = Defaults::from_json(DEFAULTS_COMMITTED).expect("the committed file parses");
        assert_eq!(
            defaults.default_outpost(),
            148,
            "the base game's starting outpost"
        );
        let doc = defaults
            .default_outpost
            .doc
            .as_deref()
            .expect("the key documents itself");
        assert!(
            doc.contains("creation") && doc.contains("travelling"),
            "the doc says who writes the real value, said: {doc}"
        );
        assert_eq!(defaults.creation_map(), 0, "the creation instance is map 0");
        assert_eq!(
            defaults.creation_screen_unlocked_professions(),
            2,
            "bit 1, Warrior"
        );
        assert_eq!(defaults.name_max_units(), 20);
        assert!(
            defaults
                .creation_map
                .doc
                .as_deref()
                .is_some_and(|doc| doc.contains("creation")),
            "the key documents itself"
        );
    }

    #[test]
    fn a_defaults_file_without_the_outpost_is_an_error_not_a_default() {
        let err = Defaults::from_json("{}").expect_err("refused");
        assert!(err.contains("default_outpost"), "said: {err}");
    }

    #[test]
    fn a_key_may_carry_its_own_documentation() {
        let defaults = Defaults::from_json(
            r#"{"default_outpost": {"value": 163, "doc": "why this number", "immutable": false},
                    "creation_map": {"value": 0, "immutable": true},
                    "creation_screen_unlocked_professions": {"value": 2},
                    "name_max_units": {"value": 20},
                    "map_type_main_town": {"value": 0, "immutable": true},
                    "map_type_main_explorable": {"value": 1, "immutable": true},
                    "map_type_character_creation": {"value": 11, "immutable": true}}"#,
        )
        .expect("parses");
        assert_eq!(defaults.default_outpost(), 163);
        assert_eq!(
            defaults.default_outpost.doc.as_deref(),
            Some("why this number")
        );
    }

    #[test]
    fn a_file_may_state_only_what_this_machine_changes() {
        let intervals = Intervals::from_json(r#"{"tick_interval_ms": 200}"#).expect("parses");
        assert_eq!(intervals.tick_interval_ms, 200);
        assert_eq!(intervals.ping_interval_secs, 5, "the rest are defaults");
        assert_eq!(intervals.ping_value, 20);
    }

    #[test]
    fn a_misspelled_key_is_an_error_rather_than_a_silent_default() {
        let err = Intervals::from_json(r#"{"tick_interval_msec": 200}"#).expect_err("refused");
        assert!(err.contains("tick_interval_msec") || err.contains("unknown field"));
    }

    #[test]
    fn the_durations_are_the_numbers_the_file_states() {
        let intervals = Intervals::from_json(
            r#"{"tick_interval_ms": 250, "ping_interval_secs": 3, "world_probe_timeout_ms": 750}"#,
        )
        .expect("parses");
        assert_eq!(intervals.tick(), std::time::Duration::from_millis(250));
        assert_eq!(intervals.ping(), std::time::Duration::from_secs(3));
        assert_eq!(
            intervals.probe_timeout(),
            std::time::Duration::from_millis(750)
        );
    }
    #[test]
    fn the_shipped_guilds_file_parses() {
        let text = std::fs::read_to_string(super::resolve(super::GUILDS_FILE))
            .expect("config/Server/guilds.json");
        let guilds = super::Guilds::from_json(&text).expect("a valid guilds file");
        assert_eq!(guilds.revision_on_create.value, 0x0200);
        assert_eq!(guilds.revision_step.value, 0x0100);
    }
}
