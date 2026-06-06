//! Report actions and verbosity (`doc/systemrs-design.md` §3.12).
//!
//! SystemC resolves each report to a set of *actions* (display, log, cache, throw,
//! abort, …) by a strict precedence, and gates `INFO` by a verbosity level. This
//! module models the actions as a plain bool struct (no `bitflags` dependency), the
//! verbosity ladder, and the **golden default-action table** — the per-severity
//! defaults that [`crate::ReportHandler`] resolves against.

use crate::severity::Severity;

/// The set of actions taken for a report (a plain-bool action mask).
///
/// Mirrors SystemC's `sc_actions` bits relevant to SystemRS. `STOP`/`INTERRUPT` are
/// represented but not acted on in M5 (faithful bit-modelling, not load-bearing).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActionFlags {
    /// Print the report to standard error (`SC_DISPLAY`).
    pub display: bool,

    /// Append the report to the log (`SC_LOG`); a no-op sink in M5.
    pub log: bool,

    /// Retain the report as the last cached report (`SC_CACHE_REPORT`).
    pub cache_report: bool,

    /// Surface the report as a recoverable `Result` (`SC_THROW`).
    pub throw: bool,

    /// Abort the process (`SC_ABORT`).
    pub abort: bool,

    /// Request the simulation stop (`SC_STOP`); modelled, not acted on in M5.
    pub stop: bool,
}

impl ActionFlags {
    /// No actions (`SC_DO_NOTHING`).
    pub const NONE: ActionFlags = ActionFlags {
        display: false,
        log: false,
        cache_report: false,
        throw: false,
        abort: false,
        stop: false,
    };

    /// Returns the **default** actions for `severity` (the golden table).
    ///
    /// Matches SystemC's default action set per severity
    /// (`sc_report_handler.cpp`): INFO and FATAL display, ERROR does **not** display
    /// (it only throws), FATAL aborts. This preserves SystemRS's existing free-fn
    /// behaviour exactly.
    ///
    /// # Arguments
    ///
    /// * `severity` - The report severity.
    ///
    /// # Returns
    ///
    /// The default [`ActionFlags`].
    pub fn default_for(severity: Severity) -> ActionFlags {
        match severity {
            Severity::Info => ActionFlags {
                display: true,
                log: true,
                ..ActionFlags::NONE
            },
            Severity::Warning => ActionFlags {
                display: true,
                log: true,
                cache_report: true,
                ..ActionFlags::NONE
            },
            Severity::Error => ActionFlags {
                // No DISPLAY: ERROR surfaces as a Result, it does not print.
                log: true,
                cache_report: true,
                throw: true,
                ..ActionFlags::NONE
            },
            Severity::Fatal => ActionFlags {
                display: true,
                log: true,
                cache_report: true,
                abort: true,
                ..ActionFlags::NONE
            },
        }
    }
}

/// A report's verbosity level (`INFO` is gated by it). Higher = more verbose.
///
/// Mirrors SystemC's `SC_NONE`/`SC_LOW`/`SC_MEDIUM`/`SC_HIGH`/`SC_FULL`/`SC_DEBUG`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Verbosity {
    /// `SC_NONE` (0).
    None,

    /// `SC_LOW` (100).
    Low,

    /// `SC_MEDIUM` (200) — the default.
    #[default]
    Medium,

    /// `SC_HIGH` (300).
    High,

    /// `SC_FULL` (400).
    Full,

    /// `SC_DEBUG` (500).
    Debug,
}

impl Verbosity {
    /// Returns the numeric level (matching SystemC's constants).
    pub fn level(self) -> u32 {
        match self {
            Verbosity::None => 0,
            Verbosity::Low => 100,
            Verbosity::Medium => 200,
            Verbosity::High => 300,
            Verbosity::Full => 400,
            Verbosity::Debug => 500,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ActionFlags, Verbosity};
    use crate::severity::Severity;

    /// The golden default-action table (EC3 baseline).
    #[test]
    fn golden_default_action_table() {
        let info = ActionFlags::default_for(Severity::Info);
        assert!(info.display && info.log && !info.throw && !info.abort);

        let warn = ActionFlags::default_for(Severity::Warning);
        assert!(warn.display && warn.cache_report && !warn.throw);

        let err = ActionFlags::default_for(Severity::Error);
        // ERROR throws but does NOT display (preserves `error()`'s no-print behaviour).
        assert!(err.throw && err.cache_report && !err.display && !err.abort);

        let fatal = ActionFlags::default_for(Severity::Fatal);
        assert!(fatal.display && fatal.abort && fatal.cache_report);
    }

    /// Verbosity levels are ordered (INFO gating compares them).
    #[test]
    fn verbosity_levels_ordered() {
        assert!(Verbosity::Debug.level() > Verbosity::Medium.level());
        assert!(Verbosity::None.level() < Verbosity::Low.level());
        assert_eq!(Verbosity::default(), Verbosity::Medium);
    }
}
