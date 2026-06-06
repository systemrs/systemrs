//! Convenience sockets (`tlm_utils::simple_initiator_socket` /
//! `simple_target_socket`).
//!
//! Thin wrappers that register **boxed closures** (replacing SystemC's `void*`
//! trampoline + pointer-to-member, `doc/systemrs-design.md` §6d) and synthesize the
//! missing transport direction: a [`SimpleTargetSocket`] given only a `b_transport`
//! (LT) implementation still answers an AT initiator's `nb_transport_fw` by spawning
//! a per-transaction coroutine that calls the LT body (`nb→b`), reusing the same
//! Send-safe service+`TxnId` construction as [`crate::AtToLtAdapter`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use systemrs_kernel::{Ctx, ObjectId, Sim};
use systemrs_time::SimTime;
use systemrs_tlm2::{GenericPayload, InitiatorSocket, Phase, TargetSocket, TlmSync, Txn};

use crate::adapter_lt_at::TxnId;

/// A boxed `b_transport` body.
type BTransportFn = Rc<dyn Fn(&Ctx, &mut GenericPayload, &mut SimTime)>;

/// Per-`SimpleTargetSocket` synthesis state (keyed by the target's object id, so
/// multiple convenience sockets share one service without collision).
struct SimpleTargetState {
    /// The user's LT body, called by the synthesized AT path.
    b_transport: BTransportFn,

    /// In-flight transactions for the spawned `nb→b` workers, by id.
    txns: HashMap<TxnId, Txn>,

    /// Monotonic id counter.
    next_id: u64,
}

/// The convenience-socket synthesis registry (a `Sim` service).
#[derive(Default)]
struct SimpleRegistry {
    /// Target object id → its synthesis state.
    targets: HashMap<ObjectId, SimpleTargetState>,
}

/// Returns the simulation's convenience-socket registry, creating it on first use.
fn registry(sim: &Sim) -> Rc<RefCell<SimpleRegistry>> {
    let ctx = sim.ctx();
    if let Some(existing) = ctx.try_service::<RefCell<SimpleRegistry>>() {
        return existing;
    }
    let reg = Rc::new(RefCell::new(SimpleRegistry::default()));
    sim.register_service(Rc::clone(&reg));
    reg
}

/// A convenience initiator socket: a thin wrapper over [`InitiatorSocket`].
#[derive(Debug, Clone, Copy)]
pub struct SimpleInitiatorSocket {
    /// The wrapped socket.
    inner: InitiatorSocket,
}

impl SimpleInitiatorSocket {
    /// Creates a convenience initiator socket.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name.
    ///
    /// # Returns
    ///
    /// A `Copy` handle.
    pub fn new(sim: &Sim, name: &str) -> Self {
        SimpleInitiatorSocket {
            inner: InitiatorSocket::new(sim, name),
        }
    }

    /// Returns the wrapped [`InitiatorSocket`].
    pub fn inner(&self) -> InitiatorSocket {
        self.inner
    }

    /// Binds to a convenience target socket.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `target` - The convenience target to bind to.
    pub fn bind(&self, sim: &Sim, target: &SimpleTargetSocket) {
        self.inner.bind(sim, &target.inner);
    }

    /// Registers the backward (AT response) callback.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `callback` - The `nb_transport_bw` implementation.
    pub fn register_nb_transport_bw<F>(&self, sim: &Sim, callback: F)
    where
        F: Fn(&Ctx, &Txn, Phase, &mut SimTime) -> TlmSync + 'static,
    {
        self.inner.register_nb_transport_bw(sim, callback);
    }
}

/// A convenience target socket that synthesizes the missing transport direction.
#[derive(Debug, Clone, Copy)]
pub struct SimpleTargetSocket {
    /// The wrapped socket.
    inner: TargetSocket,
}

impl SimpleTargetSocket {
    /// Creates a convenience target socket.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name.
    ///
    /// # Returns
    ///
    /// A `Copy` handle.
    pub fn new(sim: &Sim, name: &str) -> Self {
        SimpleTargetSocket {
            inner: TargetSocket::new(sim, name),
        }
    }

    /// Returns the wrapped [`TargetSocket`].
    pub fn inner(&self) -> TargetSocket {
        self.inner
    }

    /// Registers the forward AT (`nb_transport_fw`) callback directly.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `callback` - The `nb_transport_fw` implementation.
    pub fn register_nb_transport_fw<F>(&self, sim: &Sim, callback: F)
    where
        F: Fn(&Ctx, &Txn, Phase, &mut SimTime) -> TlmSync + 'static,
    {
        self.inner.register_nb_transport_fw(sim, callback);
    }

    /// Registers an LT `b_transport` body and **synthesizes** the AT path: an AT
    /// initiator's `nb_transport_fw` spawns a per-transaction coroutine that calls
    /// this body (`nb→b`), then drives the backward response.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `callback` - The `b_transport` implementation (it may `wait`).
    pub fn register_b_transport<F>(&self, sim: &Sim, callback: F)
    where
        F: Fn(&Ctx, &mut GenericPayload, &mut SimTime) + 'static,
    {
        let boxed: BTransportFn = Rc::new(callback);
        let target = self.inner;

        // Direct LT path.
        let direct = Rc::clone(&boxed);
        self.inner
            .register_b_transport(sim, move |cx, payload, delay| direct(cx, payload, delay));

        // Stash the body for the synthesized AT workers.
        registry(sim).borrow_mut().targets.insert(
            target.id(),
            SimpleTargetState {
                b_transport: boxed,
                txns: HashMap::new(),
                next_id: 0,
            },
        );

        // Synthesized AT path: nb→b via a spawned coroutine.
        self.inner
            .register_nb_transport_fw(sim, move |cx, txn, phase, _t| {
                if phase != Phase::BeginReq {
                    return TlmSync::Accepted;
                }
                let id = {
                    let reg = registry_from_ctx(cx);
                    let mut reg = reg.borrow_mut();
                    let st = reg
                        .targets
                        .get_mut(&target.id())
                        .expect("simple target state registered");
                    let id = TxnId::new(st.next_id);
                    st.next_id += 1;
                    st.txns.insert(id, Rc::clone(txn));
                    id
                };
                // Capture ONLY Copy data; reach the !Send body + Txn via the service.
                cx.spawn_thread("simple_nb2b", move |worker| {
                    let reg = worker.service::<RefCell<SimpleRegistry>>();
                    let fetched = {
                        let r = reg.borrow();
                        r.targets.get(&target.id()).and_then(|st| {
                            st.txns
                                .get(&id)
                                .map(|t| (Rc::clone(&st.b_transport), Rc::clone(t)))
                        })
                    };
                    if let Some((b_transport, txn)) = fetched {
                        let mut delay = SimTime::ZERO;
                        {
                            let mut payload = txn.borrow_mut();
                            b_transport(worker, &mut payload, &mut delay);
                        }
                        let mut t = SimTime::ZERO;
                        target.nb_transport_bw(worker, &txn, Phase::BeginResp, &mut t);
                        if let Some(st) = reg.borrow_mut().targets.get_mut(&target.id()) {
                            st.txns.remove(&id);
                        }
                    }
                });
                TlmSync::Accepted
            });
    }
}

/// Returns the convenience-socket registry from a runtime [`Ctx`].
fn registry_from_ctx(ctx: &Ctx) -> Rc<RefCell<SimpleRegistry>> {
    ctx.service::<RefCell<SimpleRegistry>>()
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::Mutex;

    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;
    use systemrs_tlm2::{Command, GenericPayload, Phase, ResponseStatus, TlmSync, TxnPool};

    use super::{SimpleInitiatorSocket, SimpleTargetSocket};
    use crate::PhaseQueue;

    /// A `SimpleTargetSocket` with only a `b_transport` registered answers an AT
    /// (`nb_transport_fw`) initiator via the synthesized `nb→b` path.
    #[test]
    fn simple_target_synthesizes_nb_from_b() {
        let sim = Sim::new();
        let target = SimpleTargetSocket::new(&sim, "mem");
        // Only an LT body is provided.
        target.register_b_transport(&sim, |cx, payload, _delay| {
            cx.wait(SimTime::from_ns(2)); // modelled latency
            if matches!(payload.command(), Command::Read) {
                for b in payload.data_mut() {
                    *b = 0x77;
                }
            }
            payload.set_response_status(ResponseStatus::Ok);
        });

        let isock = SimpleInitiatorSocket::new(&sim, "cpu");
        isock.bind(&sim, &target);

        // AT initiator: complete the handshake and capture the response.
        let inner = isock.inner();
        let end_pq = Rc::new(PhaseQueue::new(&sim, move |cx, txn, phase| {
            let mut t = SimTime::ZERO;
            inner.nb_transport_fw(cx, txn, phase, &mut t);
        }));
        let epq = Rc::clone(&end_pq);
        let got: Arc<Mutex<Option<u8>>> = Arc::new(Mutex::new(None));
        let g = Arc::clone(&got);
        isock.register_nb_transport_bw(&sim, move |cx, txn, phase, _t| {
            if phase == Phase::BeginResp {
                *g.lock().expect("lock") = Some(txn.borrow().data()[0]);
                epq.notify(cx, Rc::clone(txn), Phase::EndResp, SimTime::ZERO);
            }
            TlmSync::Accepted
        });

        let pool = TxnPool::new();
        let rd = pool.acquire();
        *rd.borrow_mut() = GenericPayload::read(0, 1);
        let driver = isock.inner();
        sim.add_method("driver", &[], true, move |cx| {
            let mut t = SimTime::ZERO;
            driver.nb_transport_fw(cx, &rd, Phase::BeginReq, &mut t);
        });

        sim.run_until(SimTime::from_ns(100));
        assert_eq!(*got.lock().expect("lock"), Some(0x77)); // LT body serviced the AT read
        assert_eq!(sim.now(), SimTime::from_ns(2)); // the LT body's latency advanced time
    }
}
