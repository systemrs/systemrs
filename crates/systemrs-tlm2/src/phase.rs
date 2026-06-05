//! TLM-2.0 phases and the non-blocking sync enum.

/// An interned id for an extended (user-defined) phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhaseId(pub u32);

/// A TLM-2.0 transaction phase.
///
/// The four base phases form the strict ordering BEGIN_REQ → END_REQ →
/// BEGIN_RESP → END_RESP (`doc/systemrs-design.md` §3.9); user protocols add
/// [`Phase::Extended`] phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The initial, pre-request phase.
    Uninitialized,

    /// The request begins.
    BeginReq,

    /// The request ends.
    EndReq,

    /// The response begins.
    BeginResp,

    /// The response ends.
    EndResp,

    /// A protocol-specific extended phase.
    Extended(PhaseId),
}

/// The return value of a non-blocking transport call (`tlm_sync_enum`).
///
/// Folding the advanced phase into [`TlmSync::Updated`] makes "the phase is only
/// meaningful when the call advanced it" structural rather than conventional
/// (`doc/systemrs-design.md` §6d).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlmSync {
    /// The callee is unchanged; await the opposite path (`TLM_ACCEPTED`).
    Accepted,

    /// The phase advanced synchronously to the carried phase (`TLM_UPDATED`).
    Updated(Phase),

    /// The transaction completed early (`TLM_COMPLETED`).
    Completed,
}
