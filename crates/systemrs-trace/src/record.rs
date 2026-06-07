//! Trace records: owned, `Send` events for the telemetry plane.
//!
//! Records carry **owned** plain data (`String`, `u64`, local enums) — never a
//! borrow into a live signal, and never a kernel `ObjectId` or tlm2 enum — so they
//! cross the off-thread writer's `Send` boundary freely and are formatted to text
//! without a serde dependency (`doc/systemrs-design.md` §6e).

use systemrs_time::SimTime;
use systemrs_tlm2::{Command, GenericPayload, ResponseStatus};

/// A transaction command, mirrored locally so a record carries no tlm2 type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceCommand {
    /// A read transaction.
    Read,
    /// A write transaction.
    Write,
    /// An ignore/no-op transaction.
    Ignore,
}

impl From<Command> for TraceCommand {
    fn from(c: Command) -> Self {
        match c {
            Command::Read => TraceCommand::Read,
            Command::Write => TraceCommand::Write,
            Command::Ignore => TraceCommand::Ignore,
        }
    }
}

/// A transaction response status, mirrored locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceResponse {
    /// `TLM_INCOMPLETE_RESPONSE`.
    Incomplete,
    /// `TLM_OK_RESPONSE`.
    Ok,
    /// Any error response (collapsed; the discriminant is negative).
    Error,
}

impl From<ResponseStatus> for TraceResponse {
    fn from(r: ResponseStatus) -> Self {
        if r.is_ok() {
            TraceResponse::Ok
        } else if r.is_error() {
            TraceResponse::Error
        } else {
            TraceResponse::Incomplete
        }
    }
}

/// A transaction-centric record (`doc/systemrs-design.md` §6e). LT capture leaves
/// `phases` empty; AT phase accumulation is a deferred follow-up.
///
/// Transactions are timed (not delta-tagged): the record carries the simulation
/// time, never the internal `delta_count` — so a replayed run, whose injections
/// arrive via a different scheduler path, still produces a byte-identical trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxnRecord {
    /// The time the transaction was recorded.
    pub time: SimTime,

    /// The command.
    pub command: TraceCommand,

    /// The address.
    pub address: u64,

    /// The data length in bytes.
    pub length: u32,

    /// The response status.
    pub response: TraceResponse,
}

impl TxnRecord {
    /// Builds a record from a payload at the current time.
    ///
    /// # Arguments
    ///
    /// * `time` - The current simulation time.
    /// * `payload` - The transaction payload to snapshot.
    ///
    /// # Returns
    ///
    /// The [`TxnRecord`].
    pub fn from_payload(time: SimTime, payload: &GenericPayload) -> Self {
        TxnRecord {
            time,
            command: payload.command().into(),
            address: payload.address(),
            length: u32::try_from(payload.len()).unwrap_or(u32::MAX),
            response: payload.response_status().into(),
        }
    }
}

/// A telemetry event: a signal sample or a transaction record. Owned + `Send`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent {
    /// A signal value sampled after the update phase committed.
    Signal {
        /// The signal's name.
        name: String,
        /// The sample time.
        time: SimTime,
        /// The delta count at the sample.
        delta: u64,
        /// The committed value, formatted.
        value: String,
    },

    /// A captured transaction.
    Txn(TxnRecord),
}

impl core::fmt::Display for TraceEvent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TraceEvent::Signal {
                name,
                time,
                delta,
                value,
            } => write!(f, "@{}d{} {name}={value}", time.units(), delta),
            TraceEvent::Txn(r) => write!(
                f,
                "@{} {:?} addr={:#x} len={} {:?}",
                r.time.units(),
                r.command,
                r.address,
                r.length,
                r.response
            ),
        }
    }
}
