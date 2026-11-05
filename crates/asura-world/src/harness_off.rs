//! The diagnostic instruments, inert.
//!
//! `capture` records every message the server sends and receives, and `replay` serves the reference's
//! captured stream verbatim so a crash can be bisected. Neither is part of answering a client: they are
//! a measuring instrument and a bisection tool, and a build that serves players should not carry them
//! (`docs/38-layer-decisions.md`, D32).
//!
//! This module is what those two modules answer when the `harness` feature is off. It is deliberately a
//! stub with the same shape rather than a set of `cfg` attributes at every call site: the server code
//! asks for a capture and a replay the same way in both builds, and the only difference is that an
//! instrument build does something with the answer.

/// The server-side packet capture, inert.
pub mod capture {
    /// Refuse to start a capture: this build has no recorder in it.
    ///
    /// An error rather than a silent success, because an operator who asked for a capture and got a
    /// server that recorded nothing has lost the session they were trying to measure.
    pub fn install(_path: &str) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "this build has no capture recorder; rebuild with `--features harness`",
        ))
    }

    /// Whether anything is being recorded: never, in this build.
    pub fn recording() -> bool {
        false
    }

    /// There is nothing buffered to write.
    pub fn flush() {}

    /// Nothing is recorded.
    pub fn record(_direction: &str, _header: u16, _bytes: &[u8]) {}

    /// The first bytes of a message, as hex, for the trace. The same helper the recorder carries, so a
    /// serving build logs a message identically with no capture behind it.
    pub fn hex_preview(bytes: &[u8]) -> String {
        bytes
            .iter()
            .take(24)
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The reference-stream replay, inert.
pub mod replay {
    use asura_protocol::client::Frame;

    /// A replay that replaces nothing. The fields are here so the code that chooses a block reads the
    /// same in both builds.
    #[derive(Debug, Default, Clone)]
    pub struct Replay {
        /// The load block.
        pub load: Vec<Frame>,
        /// The item stream.
        pub items: Vec<Frame>,
        /// The spawn block.
        pub spawn: Vec<Frame>,
        /// The player block.
        pub players: Vec<Frame>,
        /// Which blocks this replay replaces.
        pub pick: Pick,
    }

    impl Replay {
        /// Read a capture. This build cannot, so it says so.
        pub fn from_capture(_path: &std::path::Path) -> Result<Replay, String> {
            Err("this build has no replay parser; rebuild with `--features harness`".to_string())
        }

        /// No replay is ever configured in this build, so a start-up reads as "none asked for".
        pub fn from_environment() -> Result<Option<Replay>, String> {
            Ok(None)
        }
    }

    /// Which blocks a replay replaces.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Pick;

    impl Pick {
        /// Nothing is replaced.
        pub fn wants(&self, _block: BlockName) -> bool {
            false
        }
    }

    /// The four unasked blocks a replay can stand in for.
    #[derive(Debug, Clone, Copy)]
    pub enum BlockName {
        /// The load block.
        Load,
        /// The item stream.
        Items,
        /// The spawn block.
        Spawn,
        /// The player block.
        Players,
    }
}
