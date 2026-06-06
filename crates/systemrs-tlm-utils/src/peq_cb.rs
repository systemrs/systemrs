//! [`PhaseQueue`] — the phase-aware callback PEQ (`tlm_utils::peq_with_cb_and_phase`).
//!
//! Wraps [`PeqWithGet`] to carry `(Txn, Phase)` pairs and invokes a callback as each
//! becomes due, in delta/FIFO order (`doc/systemrs-design.md` §3.11). The draining
//! process is an `SC_METHOD` registered at *elaboration* — it needs no runtime spawn
//! — and (being a method, not a thread) may capture the `Rc`-based queue and
//! callback. The delta-parity "one per delta" shape is inherited from [`PeqWithGet`].

use std::cell::RefCell;
use std::rc::Rc;

use systemrs_kernel::{Ctx, Sim};
use systemrs_time::SimTime;
// The tlm2 transaction `Phase` (BeginReq/…); distinct from the kernel scheduler
// phase, which is not imported here — so there is no naming collision.
use systemrs_tlm2::{Phase, Txn};

use crate::peq_get::PeqWithGet;

/// A phase-release callback invoked as `(cx, &txn, phase)`.
type PhaseCb = Rc<dyn Fn(&Ctx, &Txn, Phase)>;

/// A phase-aware callback PEQ: queues `(Txn, Phase)` pairs and invokes a callback as
/// each is released, in delta/FIFO order.
pub struct PhaseQueue {
    /// The shared pull engine carrying `(Txn, Phase)` entries.
    inner: Rc<RefCell<PeqWithGet<(Txn, Phase)>>>,
}

impl PhaseQueue {
    /// Creates a phase queue whose `callback` fires for each released `(Txn, Phase)`.
    ///
    /// Registers an internal draining `SC_METHOD` sensitive to the queue's event; it
    /// releases one entry per delta and invokes `callback` with the queue borrow
    /// already released (so the callback may itself `notify`).
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `callback` - Invoked as `callback(cx, &txn, phase)` for each released entry.
    ///
    /// # Returns
    ///
    /// The [`PhaseQueue`].
    pub fn new<F>(sim: &Sim, callback: F) -> Self
    where
        F: Fn(&Ctx, &Txn, Phase) + 'static,
    {
        let inner = Rc::new(RefCell::new(PeqWithGet::new(sim)));
        let event = inner.borrow().event();

        let drain = Rc::clone(&inner);
        let cb: PhaseCb = Rc::new(callback);
        sim.add_method("peq_drain", &[event], false, move |cx| {
            // Release one due entry; the borrow is dropped before the callback runs,
            // so the callback may re-`notify` the same queue.
            let entry = drain.borrow_mut().get_next(cx);
            if let Some((txn, phase)) = entry {
                cb(cx, &txn, phase);
            }
        });

        PhaseQueue { inner }
    }

    /// Queues `(txn, phase)` for release `after` the current time.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    /// * `txn` - The transaction handle.
    /// * `phase` - The phase conveyed with the transaction.
    /// * `after` - The relative release delay (`ZERO` → next delta).
    pub fn notify(&self, cx: &Ctx, txn: Txn, phase: Phase, after: SimTime) {
        self.inner.borrow_mut().notify(cx, (txn, phase), after);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;
    use systemrs_tlm2::{Phase, TxnPool};

    use super::PhaseQueue;

    /// Two same-time entries fire one delta apart, in insertion (FIFO) order, each
    /// carrying its own phase.
    #[test]
    fn phase_queue_invokes_callback_in_delta_order() {
        let sim = Sim::new();
        let log: Rc<RefCell<Vec<(Phase, u64, u64)>>> = Rc::new(RefCell::new(Vec::new()));
        let l = Rc::clone(&log);

        let pq = Rc::new(PhaseQueue::new(&sim, move |cx, _txn, phase| {
            l.borrow_mut()
                .push((phase, cx.now().units(), cx.delta_count()));
        }));

        let pool = TxnPool::new();
        let a = pool.acquire();
        let b = pool.acquire();
        let driver = Rc::clone(&pq);
        sim.add_method("driver", &[], true, move |cx| {
            driver.notify(cx, Rc::clone(&a), Phase::BeginReq, SimTime::ZERO);
            driver.notify(cx, Rc::clone(&b), Phase::EndResp, SimTime::ZERO);
        });

        sim.run_until(SimTime::from_ns(10));

        let recs = log.borrow();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].0, Phase::BeginReq); // FIFO
        assert_eq!(recs[1].0, Phase::EndResp);
        assert_eq!(recs[0].1, recs[1].1); // same sim time
        assert_eq!(recs[1].2, recs[0].2 + 1); // one delta apart
    }
}
