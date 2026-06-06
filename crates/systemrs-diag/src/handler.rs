//! [`ReportHandler`] — configurable report action resolution (`sc_report_handler`).
//!
//! Resolves each report to its [`ActionFlags`] by the strict SystemC precedence
//! (`doc/systemrs-design.md` §3.12): a per-(message-type, severity) override beats a
//! per-severity override beats the [golden default table](ActionFlags::default_for).
//! Resolution is a **pure** function (lookups only — no `HashMap` iteration feeds the
//! result, so it is deterministic). `emit` then applies the resolved actions.

use std::collections::HashMap;

use crate::action::{ActionFlags, Verbosity};
use crate::report::{Report, ReportError};
use crate::severity::Severity;

/// A per-runtime report handler (not a global singleton).
///
/// Holds the action overrides, the verbosity ceiling, and the last cached report.
#[derive(Default)]
pub struct ReportHandler {
    /// Per-severity action overrides.
    severity_actions: HashMap<Severity, ActionFlags>,

    /// Per-(message-type, severity) action overrides (highest precedence).
    type_severity_actions: HashMap<(String, Severity), ActionFlags>,

    /// The verbosity ceiling; an `INFO` above it is suppressed.
    verbosity: Verbosity,

    /// The last report with the `cache_report` action (`SC_CACHE_REPORT`).
    cached: Option<Report>,
}

impl ReportHandler {
    /// Creates a handler with the default golden actions and `Medium` verbosity.
    ///
    /// # Returns
    ///
    /// A fresh [`ReportHandler`].
    pub fn new() -> Self {
        ReportHandler::default()
    }

    /// Overrides the actions for every report of `severity`.
    ///
    /// # Arguments
    ///
    /// * `severity` - The severity to override.
    /// * `actions` - The actions to take.
    pub fn set_severity_actions(&mut self, severity: Severity, actions: ActionFlags) {
        self.severity_actions.insert(severity, actions);
    }

    /// Overrides the actions for reports of `msg_type` at `severity` (highest
    /// precedence).
    ///
    /// # Arguments
    ///
    /// * `msg_type` - The message-type tag.
    /// * `severity` - The severity.
    /// * `actions` - The actions to take.
    pub fn set_type_severity_actions(
        &mut self,
        msg_type: &str,
        severity: Severity,
        actions: ActionFlags,
    ) {
        self.type_severity_actions
            .insert((msg_type.to_owned(), severity), actions);
    }

    /// Sets the verbosity ceiling.
    ///
    /// # Arguments
    ///
    /// * `verbosity` - The maximum verbosity that will be emitted for `INFO`.
    pub fn set_verbosity(&mut self, verbosity: Verbosity) {
        self.verbosity = verbosity;
    }

    /// Returns the last cached report, if any (`SC_CACHE_REPORT`).
    pub fn cached_report(&self) -> Option<&Report> {
        self.cached.as_ref()
    }

    /// Resolves the actions for `msg_type` at `severity` by the strict precedence.
    ///
    /// Pure: per-(type, severity) override > per-severity override > golden default.
    ///
    /// # Arguments
    ///
    /// * `msg_type` - The message-type tag.
    /// * `severity` - The severity.
    ///
    /// # Returns
    ///
    /// The resolved [`ActionFlags`].
    pub fn resolve(&self, msg_type: &str, severity: Severity) -> ActionFlags {
        if let Some(&a) = self
            .type_severity_actions
            .get(&(msg_type.to_owned(), severity))
        {
            return a;
        }
        if let Some(&a) = self.severity_actions.get(&severity) {
            return a;
        }
        ActionFlags::default_for(severity)
    }

    /// Emits `report` at `verbosity`, applying the resolved actions.
    ///
    /// `INFO` above the verbosity ceiling is suppressed. `display` prints to stderr;
    /// `cache_report` retains the report; `abort` aborts; `throw` returns the report
    /// as a recoverable [`ReportError`].
    ///
    /// # Arguments
    ///
    /// * `report` - The report to emit.
    /// * `verbosity` - The report's verbosity (only meaningful for `INFO`).
    ///
    /// # Returns
    ///
    /// `Some(ReportError)` if the resolved actions include `throw`, else `None`.
    ///
    /// # Panics
    ///
    /// Panics (aborts) if the resolved actions include `abort` (FATAL by default).
    pub fn emit(&mut self, report: Report, verbosity: Verbosity) -> Option<ReportError> {
        // Verbosity gates INFO only.
        if report.severity() == Severity::Info && verbosity.level() > self.verbosity.level() {
            return None;
        }
        let actions = self.resolve(report.msg_type(), report.severity());
        if actions.cache_report {
            self.cached = Some(report.clone());
        }
        if actions.display {
            eprintln!("{report}");
        }
        assert!(!actions.abort, "{report}");
        if actions.throw {
            return Some(ReportError(report));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::ReportHandler;
    use crate::action::{ActionFlags, Verbosity};
    use crate::report::Report;
    use crate::severity::Severity;

    /// EC3: action precedence — type+severity beats severity beats the golden default.
    #[test]
    fn resolution_precedence() {
        let mut h = ReportHandler::new();

        // Default (golden) when nothing is overridden.
        assert_eq!(
            h.resolve("ANY", Severity::Warning),
            ActionFlags::default_for(Severity::Warning)
        );

        // Per-severity override beats the default.
        let sev = ActionFlags {
            display: false,
            ..ActionFlags::default_for(Severity::Warning)
        };
        h.set_severity_actions(Severity::Warning, sev);
        assert_eq!(h.resolve("ANY", Severity::Warning), sev);

        // Per-(type, severity) override beats the per-severity override.
        let ts = ActionFlags {
            abort: true,
            ..ActionFlags::NONE
        };
        h.set_type_severity_actions("HOT", Severity::Warning, ts);
        assert_eq!(h.resolve("HOT", Severity::Warning), ts);
        // A different type still uses the per-severity override.
        assert_eq!(h.resolve("COLD", Severity::Warning), sev);
    }

    /// `INFO` above the verbosity ceiling is suppressed (no throw, no panic).
    #[test]
    fn verbosity_gates_info() {
        let mut h = ReportHandler::new(); // ceiling = Medium
        let r = Report::new(Severity::Info, "DBG", "noisy");
        // Debug-verbosity INFO is suppressed at the Medium ceiling.
        assert!(h.emit(r.clone(), Verbosity::Debug).is_none());
        assert!(h.cached_report().is_none());
        // Low-verbosity INFO passes (displayed; INFO does not throw).
        assert!(h.emit(r, Verbosity::Low).is_none());
    }

    /// ERROR throws (returns a `ReportError`) and is cached; FATAL aborts.
    #[test]
    fn error_throws_and_caches() {
        let mut h = ReportHandler::new();
        let err = h.emit(Report::new(Severity::Error, "E", "boom"), Verbosity::Medium);
        assert!(err.is_some());
        assert!(h.cached_report().is_some());
    }
}
