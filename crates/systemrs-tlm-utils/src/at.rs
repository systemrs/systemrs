//! Approximately-timed (AT) four-phase protocol helpers and obligations.
//!
//! The AT base protocol walks the strict phase order
//! `BEGIN_REQ → END_REQ → BEGIN_RESP → END_RESP` over the socket `nb_transport_fw`
//! (request, forward) and `nb_transport_bw` (response, backward) calls, with the
//! [`TlmSync`](systemrs_tlm2::TlmSync) return value carrying each peer's obligation
//! (`doc/systemrs-design.md` §3.9, §6d). This module provides the phase-order helper
//! and documents the contract; the reference drivers live in the adapters and the
//! conformance tests.
//!
//! ## `TlmSync` obligations
//!
//! - [`Accepted`](systemrs_tlm2::TlmSync::Accepted) — the callee is unchanged; the
//!   caller must await the opposite path (no phase advanced).
//! - [`Updated`](systemrs_tlm2::TlmSync::Updated) — the callee advanced the phase
//!   synchronously (folded into the variant so a `match` is total); the caller drives
//!   from the new phase.
//! - [`Completed`](systemrs_tlm2::TlmSync::Completed) — the transaction ended early;
//!   no further phase calls are owed.
//!
//! The timing annotation `t` is *relative* and a callee may *increase* it; the
//! initiator owes `wait(t)` (or folds it into its quantum keeper).
//!
//! ## `Txn` aliasing discipline
//!
//! The AT transaction is a shared `Rc<RefCell<GenericPayload>>` (`Txn`), aliased
//! across phases and mutated in place. Every `txn.borrow_mut()` taken by an
//! **AT-side phase callback** is short-lived and dropped before any `wait`/`notify`/
//! registry dispatch — a double-borrow is a clean `RefCell` panic, not UB. The one
//! *sanctioned* borrow-across-`wait` is the `b_transport` bridge inside the LT↔AT
//! adapters, where a single in-flight `Txn` (uncontended) is held while the LT callee
//! waits for latency.

use systemrs_tlm2::Phase;

/// Returns the successor of `phase` in the strict AT four-phase order, or `None` at
/// the end (`END_RESP`) or for non-base phases.
///
/// # Arguments
///
/// * `phase` - The current phase.
///
/// # Returns
///
/// The next base phase, or `None`.
pub fn next_phase(phase: Phase) -> Option<Phase> {
    match phase {
        Phase::BeginReq => Some(Phase::EndReq),
        Phase::EndReq => Some(Phase::BeginResp),
        Phase::BeginResp => Some(Phase::EndResp),
        Phase::EndResp | Phase::Uninitialized | Phase::Extended(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;
    use systemrs_tlm2::{
        Command, GenericPayload, InitiatorSocket, Phase, ResponseStatus, TargetSocket, TlmSync,
        TxnPool,
    };

    use super::next_phase;
    use crate::PhaseQueue;

    /// The strict four-phase order, terminating at END_RESP.
    #[test]
    fn next_phase_is_strict_order() {
        assert_eq!(next_phase(Phase::BeginReq), Some(Phase::EndReq));
        assert_eq!(next_phase(Phase::EndReq), Some(Phase::BeginResp));
        assert_eq!(next_phase(Phase::BeginResp), Some(Phase::EndResp));
        assert_eq!(next_phase(Phase::EndResp), None);
    }

    /// E2: a full BEGIN_REQ→END_RESP exchange between a hand-written AT initiator and
    /// AT target exercises all three `TlmSync` paths, with correct data and an `Ok`
    /// response (the committed return-shape fixture).
    ///
    /// Fixture: BeginReq → target returns `Updated(EndReq)` and PEQ-schedules a
    /// backward BeginResp; the initiator returns `Accepted` to BeginResp and
    /// PEQ-schedules a forward EndResp; the target returns `Completed` to EndResp.
    #[test]
    fn full_four_phase_exchange_all_sync_paths() {
        let sim = Sim::new();
        let target = TargetSocket::new(&sim, "mem");
        let isock = InitiatorSocket::new(&sim, "cpu");
        isock.bind(&sim, &target);

        // Every `TlmSync` returned during the handshake, for the all-three assertion.
        let seen: Rc<RefCell<Vec<TlmSync>>> = Rc::new(RefCell::new(Vec::new()));
        // A one-byte backing store the AT target writes to (data-correctness check).
        let backing: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(vec![0u8; 16]));

        // The target's response queue: drive BeginResp backward when due.
        let resp_pq = Rc::new(PhaseQueue::new(&sim, move |cx, txn, phase| {
            let mut t = SimTime::ZERO;
            target.nb_transport_bw(cx, txn, phase, &mut t);
        }));

        // Target forward callback.
        let tseen = Rc::clone(&seen);
        let tback = Rc::clone(&backing);
        let rpq = Rc::clone(&resp_pq);
        target.register_nb_transport_fw(&sim, move |cx, txn, phase, _t| {
            let sync = match phase {
                Phase::BeginReq => {
                    // Short-lived borrow: perform the write, set the response.
                    {
                        let mut p = txn.borrow_mut();
                        if matches!(p.command(), Command::Write) {
                            let addr = p.address() as usize;
                            tback.borrow_mut()[addr] = p.data()[0];
                        }
                        p.set_response_status(ResponseStatus::Ok);
                    }
                    // Schedule the backward response after a modelled latency.
                    rpq.notify(cx, Rc::clone(txn), Phase::BeginResp, SimTime::from_ns(2));
                    TlmSync::Updated(Phase::EndReq) // UPDATED
                }
                Phase::EndResp => TlmSync::Completed, // COMPLETED
                _ => TlmSync::Accepted,
            };
            tseen.borrow_mut().push(sync);
            sync
        });

        // The initiator's EndResp queue: drive EndResp forward when due.
        let end_pq = Rc::new(PhaseQueue::new(&sim, move |cx, txn, phase| {
            let mut t = SimTime::ZERO;
            isock.nb_transport_fw(cx, txn, phase, &mut t);
        }));

        // Initiator backward callback.
        let iseen = Rc::clone(&seen);
        let epq = Rc::clone(&end_pq);
        isock.register_nb_transport_bw(&sim, move |cx, txn, phase, _t| {
            let sync = match phase {
                Phase::BeginResp => {
                    epq.notify(cx, Rc::clone(txn), Phase::EndResp, SimTime::from_ns(1));
                    TlmSync::Accepted // ACCEPTED
                }
                _ => TlmSync::Accepted,
            };
            iseen.borrow_mut().push(sync);
            sync
        });

        // Driver: kick off the handshake with BEGIN_REQ.
        let pool = TxnPool::new();
        let txn = pool.acquire();
        *txn.borrow_mut() = GenericPayload::write(0, vec![0xAB]);
        sim.add_method("driver", &[], true, move |cx| {
            let mut t = SimTime::ZERO;
            isock.nb_transport_fw(cx, &txn, Phase::BeginReq, &mut t);
        });

        sim.run_until(SimTime::from_ns(100));

        let recs = seen.borrow();
        assert!(
            recs.iter().any(|s| matches!(s, TlmSync::Accepted)),
            "ACCEPTED path"
        );
        assert!(
            recs.iter().any(|s| matches!(s, TlmSync::Updated(_))),
            "UPDATED path"
        );
        assert!(
            recs.iter().any(|s| matches!(s, TlmSync::Completed)),
            "COMPLETED path"
        );
        // Data correctness: the AT target wrote the byte.
        assert_eq!(backing.borrow()[0], 0xAB);
    }
}
