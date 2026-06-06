//! LT↔AT protocol adapters (`doc/systemrs-design.md` §6d).
//!
//! - [`LtToAtAdapter`] presents `b_transport` to an LT initiator and drives an AT
//!   target: it pushes the transaction onto the AT request path, then **blocks** the
//!   LT coroutine on a per-transaction event until the four-phase exchange completes.
//! - [`AtToLtAdapter`] presents `nb_transport_fw` to an AT initiator and drives an LT
//!   target: on `BEGIN_REQ` it returns `Accepted`, then **spawns** a per-transaction
//!   coroutine that calls the LT target's blocking `b_transport` (which may `wait`),
//!   then drives the response backward.
//!
//! Transactions are matched to their pending state by a `Copy`+`Send` [`TxnId`]
//! carried in a payload extension — never a raw pointer (so it crosses the spawn
//! `Send` boundary). The spawned `AtToLtAdapter` body captures only that id and
//! re-fetches the (`!Send`) `Txn` through a [`Sim`] service.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use systemrs_kernel::{EventId, Sim};
use systemrs_time::SimTime;
use systemrs_tlm2::{Extension, InitiatorSocket, Phase, TargetSocket, TlmSync, Txn, TxnPool};

/// A per-`Sim` monotonic transaction identity (`Copy`+`Send`, not a raw pointer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TxnId(u64);

impl TxnId {
    /// Creates a transaction id from a monotonic counter value.
    ///
    /// # Arguments
    ///
    /// * `value` - The counter value.
    ///
    /// # Returns
    ///
    /// The [`TxnId`].
    pub fn new(value: u64) -> Self {
        TxnId(value)
    }
}

/// A payload extension carrying the adapter's [`TxnId`], so a response transaction
/// can be matched back to its pending state.
struct TxnIdExt(TxnId);

impl Extension for TxnIdExt {
    fn clone_ext(&self) -> Option<Box<dyn Extension>> {
        Some(Box::new(TxnIdExt(self.0)))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Reads a transaction's [`TxnId`] extension, if present.
fn txn_id(txn: &Txn) -> Option<TxnId> {
    txn.borrow().extensions().get::<TxnIdExt>().map(|e| e.0)
}

// ---------------------------------------------------------------- LT → AT

/// Pending state of the [`LtToAtAdapter`]: each blocked LT transaction's completion
/// event, keyed by [`TxnId`].
struct LtToAtState {
    /// `TxnId` → the event the blocked LT coroutine waits on.
    pending: HashMap<TxnId, EventId>,

    /// Monotonic id counter.
    next_id: u64,
}

/// Adapts an LT initiator (`b_transport`) to an AT target (`nb_transport`).
pub struct LtToAtAdapter {
    /// The LT-facing target socket (an LT initiator binds here).
    lt_target: TargetSocket,

    /// The AT-facing initiator socket (binds to the downstream AT target).
    at_initiator: InitiatorSocket,
}

impl LtToAtAdapter {
    /// Creates the adapter and registers its bridging callbacks.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name.
    ///
    /// # Returns
    ///
    /// The [`LtToAtAdapter`].
    pub fn new(sim: &Sim, name: &str) -> Self {
        let lt_target = TargetSocket::new(sim, &format!("{name}.lt"));
        let at_initiator = InitiatorSocket::new(sim, &format!("{name}.at"));
        let state = Rc::new(RefCell::new(LtToAtState {
            pending: HashMap::new(),
            next_id: 0,
        }));
        let pool = Rc::new(TxnPool::new());

        // LT face: block the calling coroutine until the AT exchange completes.
        let bt_state = Rc::clone(&state);
        let bt_pool = Rc::clone(&pool);
        lt_target.register_b_transport(sim, move |cx, payload, delay| {
            let txn = bt_pool.acquire();
            *txn.borrow_mut() = payload.clone();
            let id = {
                let mut st = bt_state.borrow_mut();
                let id = TxnId(st.next_id);
                st.next_id += 1;
                st.pending.insert(id, cx.alloc_event());
                id
            };
            txn.borrow_mut().extensions_mut().set(TxnIdExt(id));

            let mut t = *delay;
            at_initiator.nb_transport_fw(cx, &txn, Phase::BeginReq, &mut t);

            let done = bt_state.borrow().pending[&id];
            cx.wait_event(done); // blocks the LT coroutine (stackful)

            *payload = txn.borrow().clone(); // response back to the caller
            bt_state.borrow_mut().pending.remove(&id);
            bt_pool.recycle(txn);
        });

        // AT face: on the backward response, finish the handshake and unblock.
        let bw_state = Rc::clone(&state);
        at_initiator.register_nb_transport_bw(sim, move |cx, txn, phase, _t| {
            if phase == Phase::BeginResp {
                let mut t = SimTime::ZERO;
                at_initiator.nb_transport_fw(cx, txn, Phase::EndResp, &mut t);
                if let Some(id) = txn_id(txn)
                    && let Some(&done) = bw_state.borrow().pending.get(&id)
                {
                    cx.notify(done);
                }
            }
            TlmSync::Accepted
        });

        LtToAtAdapter {
            lt_target,
            at_initiator,
        }
    }

    /// Returns the LT-facing target socket (an LT initiator binds to this).
    pub fn lt_target(&self) -> TargetSocket {
        self.lt_target
    }

    /// Returns the AT-facing initiator socket (bind this to the downstream AT target).
    pub fn at_initiator(&self) -> InitiatorSocket {
        self.at_initiator
    }
}

// ---------------------------------------------------------------- AT → LT

/// Pending state of the [`AtToLtAdapter`], registered as a `Sim` service so the
/// spawned (`Send`) per-transaction body can reach the (`!Send`) `Txn` by [`TxnId`].
struct AtToLtState {
    /// `TxnId` → the in-flight transaction handle.
    txns: HashMap<TxnId, Txn>,

    /// Monotonic id counter.
    next_id: u64,
}

/// Adapts an AT initiator (`nb_transport_fw`) to an LT target (`b_transport`).
pub struct AtToLtAdapter {
    /// The AT-facing target socket (an AT initiator binds here).
    at_target: TargetSocket,

    /// The LT-facing initiator socket (binds to the downstream LT target).
    lt_initiator: InitiatorSocket,
}

impl AtToLtAdapter {
    /// Creates the adapter and registers its bridging callback.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name.
    ///
    /// # Returns
    ///
    /// The [`AtToLtAdapter`].
    pub fn new(sim: &Sim, name: &str) -> Self {
        let at_target = TargetSocket::new(sim, &format!("{name}.at"));
        let lt_initiator = InitiatorSocket::new(sim, &format!("{name}.lt"));
        let state = Rc::new(RefCell::new(AtToLtState {
            txns: HashMap::new(),
            next_id: 0,
        }));
        sim.register_service(Rc::clone(&state));

        let fw_state = Rc::clone(&state);
        at_target.register_nb_transport_fw(sim, move |cx, txn, phase, _t| {
            if phase == Phase::BeginReq {
                let id = {
                    let mut st = fw_state.borrow_mut();
                    let id = TxnId(st.next_id);
                    st.next_id += 1;
                    st.txns.insert(id, Rc::clone(txn));
                    id
                };
                // Spawn the per-transaction worker. It captures ONLY `Copy`+`Send`
                // data (the two sockets and the id); the `!Send` `Txn` is reached via
                // the service, keyed by the id.
                cx.spawn_thread("at2lt_worker", move |worker| {
                    let st = worker.service::<RefCell<AtToLtState>>();
                    let txn = st.borrow().txns.get(&id).cloned();
                    if let Some(txn) = txn {
                        let mut delay = SimTime::ZERO;
                        {
                            // The sanctioned borrow-across-wait: a single in-flight,
                            // uncontended Txn while the LT target waits for latency.
                            let mut payload = txn.borrow_mut();
                            lt_initiator.b_transport(worker, &mut payload, &mut delay);
                        }
                        let mut t = SimTime::ZERO;
                        at_target.nb_transport_bw(worker, &txn, Phase::BeginResp, &mut t);
                        st.borrow_mut().txns.remove(&id);
                    }
                });
                TlmSync::Accepted
            } else {
                TlmSync::Accepted
            }
        });

        AtToLtAdapter {
            at_target,
            lt_initiator,
        }
    }

    /// Returns the AT-facing target socket (an AT initiator binds to this).
    pub fn at_target(&self) -> TargetSocket {
        self.at_target
    }

    /// Returns the LT-facing initiator socket (bind this to the downstream LT target).
    pub fn lt_initiator(&self) -> InitiatorSocket {
        self.lt_initiator
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::Mutex;

    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;
    use systemrs_tlm2::{
        GenericPayload, InitiatorSocket, Memory, Phase, ResponseStatus, TargetSocket, TlmSync,
        TxnPool,
    };

    use super::{AtToLtAdapter, LtToAtAdapter};
    use crate::PhaseQueue;

    /// Builds a fixture AT target servicing a read (fills bytes with `fill`) over the
    /// committed four-phase handshake, registered on a fresh target socket.
    fn at_target_fixture(sim: &Sim, name: &str, fill: u8) -> TargetSocket {
        let target = TargetSocket::new(sim, name);
        let resp_pq = Rc::new(PhaseQueue::new(sim, move |cx, txn, phase| {
            let mut t = SimTime::ZERO;
            target.nb_transport_bw(cx, txn, phase, &mut t);
        }));
        let rpq = Rc::clone(&resp_pq);
        target.register_nb_transport_fw(sim, move |cx, txn, phase, _t| match phase {
            Phase::BeginReq => {
                {
                    let mut p = txn.borrow_mut();
                    for b in p.data_mut() {
                        *b = fill;
                    }
                    p.set_response_status(ResponseStatus::Ok);
                }
                rpq.notify(cx, Rc::clone(txn), Phase::BeginResp, SimTime::from_ns(2));
                TlmSync::Updated(Phase::EndReq)
            }
            Phase::EndResp => TlmSync::Completed,
            _ => TlmSync::Accepted,
        });
        target
    }

    /// E3: an LT initiator (`b_transport`) drives an AT target through the adapter and
    /// blocks until the four-phase exchange completes, getting the right data back.
    #[test]
    fn lt_initiator_to_at_target() {
        let sim = Sim::new();
        let adapter = LtToAtAdapter::new(&sim, "br");
        let at_target = at_target_fixture(&sim, "mem", 0xEE);
        adapter.at_initiator().bind(&sim, &at_target);

        let lt_init = InitiatorSocket::new(&sim, "cpu");
        lt_init.bind(&sim, &adapter.lt_target());

        let got = Arc::new(Mutex::new(0u8));
        let g = Arc::clone(&got);
        sim.add_thread("lt_init", &[], true, move |cx| {
            let mut payload = GenericPayload::read(0, 1);
            let mut delay = SimTime::ZERO;
            lt_init.b_transport(cx, &mut payload, &mut delay);
            *g.lock().expect("lock") = payload.data()[0];
        });

        sim.run_until(SimTime::from_ns(100));
        assert_eq!(*got.lock().expect("lock"), 0xEE);
        // The blocking b_transport returned only after the modelled AT latency.
        assert_eq!(sim.now(), SimTime::from_ns(2));
    }

    /// E4: an AT initiator (`nb_transport_fw`) drives an LT memory target through the
    /// adapter, which spawns a per-transaction coroutine that calls the LT target's
    /// (waiting) `b_transport` and returns the response on the backward path.
    #[test]
    fn at_initiator_to_lt_target() {
        let sim = Sim::new();
        let adapter = AtToLtAdapter::new(&sim, "br");
        let mem = Memory::new(16, SimTime::from_ns(2));
        let mem_target = TargetSocket::new(&sim, "mem");
        mem.connect(&sim, &mem_target);
        adapter.lt_initiator().bind(&sim, &mem_target);

        let at_init = InitiatorSocket::new(&sim, "cpu");
        at_init.bind(&sim, &adapter.at_target());

        let resp: Rc<RefCell<Option<ResponseStatus>>> = Rc::new(RefCell::new(None));
        let r = Rc::clone(&resp);
        at_init.register_nb_transport_bw(&sim, move |_cx, txn, phase, _t| {
            if phase == Phase::BeginResp {
                *r.borrow_mut() = Some(txn.borrow().response_status());
            }
            TlmSync::Completed
        });

        let pool = TxnPool::new();
        let txn = pool.acquire();
        *txn.borrow_mut() = GenericPayload::write(0, vec![0xCD]);
        sim.add_method("driver", &[], true, move |cx| {
            let mut t = SimTime::ZERO;
            at_init.nb_transport_fw(cx, &txn, Phase::BeginReq, &mut t);
        });

        sim.run_until(SimTime::from_ns(100));
        assert_eq!(mem.read_byte(0), 0xCD); // LT target serviced the write
        assert_eq!(*resp.borrow(), Some(ResponseStatus::Ok)); // response came back via bw
        assert_eq!(sim.now(), SimTime::from_ns(2)); // modelled memory latency advanced time
    }
}
