//! Diagnostics and reporting for SystemRS.
//!
//! SystemC's `sc_report` is simultaneously a *record* and a *thrown exception*
//! (`doc/systemrs-design.md` §3.12, §7). SystemRS splits these by phase: INFO and
//! WARNING are emitted as records, ERROR surfaces as a typed [`ReportError`]
//! (recoverable — testbenches inject errors), and FATAL aborts the process with a
//! diagnostic. This crate is the L0 leaf every other crate reports through.

mod action;
mod handler;
mod report;
mod severity;

pub use action::{ActionFlags, Verbosity};
pub use handler::ReportHandler;
pub use report::{Report, ReportError};
pub use severity::Severity;

/// Emits an informational report to standard error.
///
/// # Arguments
///
/// * `msg_type` - The message-type tag (mirrors SystemC's message-type string).
/// * `message` - The human-readable report body.
pub fn report_info(msg_type: &str, message: &str) {
    eprintln!("Info: {msg_type}: {message}");
}

/// Emits a warning report to standard error.
///
/// # Arguments
///
/// * `msg_type` - The message-type tag.
/// * `message` - The human-readable report body.
pub fn report_warning(msg_type: &str, message: &str) {
    eprintln!("Warning: {msg_type}: {message}");
}

/// Constructs a recoverable [`ReportError`] for an `ERROR`-severity condition.
///
/// Unlike SystemC, ERROR does not throw across the stack; the caller propagates
/// the returned error with `?` (`doc/systemrs-design.md` §7).
///
/// # Arguments
///
/// * `msg_type` - The message-type tag.
/// * `message` - The human-readable report body.
///
/// # Returns
///
/// A [`ReportError`] wrapping the constructed [`Report`].
pub fn error(msg_type: &str, message: &str) -> ReportError {
    ReportError(Report::new(Severity::Error, msg_type, message))
}

/// Reports a `FATAL` condition and aborts.
///
/// FATAL is unrecoverable invariant corruption; per the design this aborts with a
/// diagnostic (`doc/systemrs-design.md` §7). Implemented as a panic so destructors
/// of live frames still run on the unwinding build.
///
/// # Arguments
///
/// * `msg_type` - The message-type tag.
/// * `message` - The human-readable report body.
///
/// # Panics
///
/// Always panics with the formatted report.
pub fn report_fatal(msg_type: &str, message: &str) -> ! {
    panic!("Fatal: {msg_type}: {message}");
}
