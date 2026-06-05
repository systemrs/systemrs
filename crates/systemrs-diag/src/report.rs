//! The [`Report`] record and the recoverable [`ReportError`] wrapper.

use crate::severity::Severity;
use thiserror::Error;

/// A diagnostic record: a severity, a message-type tag, and a message body.
///
/// This is the *record* half of SystemC's `sc_report`; the *exception* half is
/// modelled by [`ReportError`] (for ERROR) or an abort (for FATAL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The severity of the condition.
    severity: Severity,

    /// The message-type tag (mirrors SystemC's message-type string).
    msg_type: String,

    /// The human-readable report body.
    message: String,
}

impl Report {
    /// Constructs a new report.
    ///
    /// # Arguments
    ///
    /// * `severity` - The severity of the condition.
    /// * `msg_type` - The message-type tag.
    /// * `message` - The human-readable report body.
    ///
    /// # Returns
    ///
    /// The constructed [`Report`].
    pub fn new(severity: Severity, msg_type: &str, message: &str) -> Self {
        Self {
            severity,
            msg_type: msg_type.to_owned(),
            message: message.to_owned(),
        }
    }

    /// Returns the severity of this report.
    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// Returns the message-type tag of this report.
    pub fn msg_type(&self) -> &str {
        &self.msg_type
    }

    /// Returns the message body of this report.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl core::fmt::Display for Report {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}: {}: {}",
            self.severity.label(),
            self.msg_type,
            self.message
        )
    }
}

/// A recoverable error carrying an ERROR-severity [`Report`].
///
/// Returned by [`crate::error`] and propagated with `?`, replacing SystemC's
/// ERROR-as-exception control flow (`doc/systemrs-design.md` §7).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct ReportError(pub Report);
