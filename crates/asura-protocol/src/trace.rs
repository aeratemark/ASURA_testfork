//! The per-message trace: a service's own record of what crossed the wire.
//!
//! The game client reports nothing but an error number (`007`), so a trace is the only way to see what
//! actually differed from the reference (`docs/03-protocol.md`, section 6). GWPS has the same switch
//! (`GWPS_TRACE`), and this project's services read `ASURA_TRACE`.
//!
//! Shared between the auth service and a world server because both need the same rule and the same
//! output shape: a wrong answer to "is the trace on?" is a debugging session that produces nothing.

/// Whether to print the per-message trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trace {
    Off,
    On,
}

impl Trace {
    /// Read `ASURA_TRACE` from the environment: `1`, `true`, or `yes` turns it on. Anything else,
    /// including an unset variable, is off.
    pub fn from_environment() -> Self {
        match std::env::var("ASURA_TRACE") {
            Ok(value) if Trace::is_on_value(&value) => Trace::On,
            _ => Trace::Off,
        }
    }

    /// Whether the trace is on.
    pub fn on(self) -> bool {
        matches!(self, Trace::On)
    }

    /// Whether a value means "on". Split out from [`Trace::from_environment`] so the rule can be
    /// tested without touching the process environment.
    pub fn is_on_value(value: &str) -> bool {
        let value = value.trim().to_ascii_lowercase();
        value == "1" || value == "true" || value == "yes"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_on_values_are_the_documented_ones() {
        for value in ["1", "true", "TRUE", " yes ", "Yes"] {
            assert!(Trace::is_on_value(value), "{value:?} means on");
        }
    }

    #[test]
    fn anything_else_is_off() {
        for value in ["", "0", "false", "no", "on", "2"] {
            assert!(!Trace::is_on_value(value), "{value:?} means off");
        }
    }
}
