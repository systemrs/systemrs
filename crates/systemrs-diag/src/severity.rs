//! Report severity levels.

/// Severity of a diagnostic report, ordered from least to most serious.
///
/// Mirrors SystemC's `sc_severity` (`SC_INFO`/`SC_WARNING`/`SC_ERROR`/`SC_FATAL`),
/// with the default-action mapping described in `doc/systemrs-design.md` §3.12:
/// ERROR becomes a recoverable `Result` and FATAL aborts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Informational; emitted as a record, verbosity-gated in SystemC.
    Info,

    /// An unexpected but handled condition.
    Warning,

    /// A recoverable error surfaced as a typed `Result`.
    Error,

    /// Unrecoverable invariant corruption; aborts.
    Fatal,
}

impl Severity {
    /// Returns the SystemC-style label for this severity.
    ///
    /// # Returns
    ///
    /// A static string such as `"Info"` or `"Fatal"`.
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "Info",
            Severity::Warning => "Warning",
            Severity::Error => "Error",
            Severity::Fatal => "Fatal",
        }
    }
}
