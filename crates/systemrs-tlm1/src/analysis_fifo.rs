//! [`AnalysisFifo`] — an **unbounded** analysis sink (`tlm_analysis_fifo`).
//!
//! The ergonomic "stream" face of the analysis layer: a subscriber that buffers
//! every broadcast value so telemetry never stalls the model
//! (`doc/systemrs-design.md` §3.7, §6e). `write()` always succeeds (unbounded — no
//! back-pressure); a consumer drains the values one delta later, honouring the
//! evaluate/update visibility rule (put in delta N → readable in N+1).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::rc::Rc;

use systemrs_kernel::{ChanId, Ctx, EventId, Sim, UpdatableChannel};

use crate::analysis_port::AnalysisWrite;

/// Kernel-held state for an [`AnalysisFifo`].
struct AnalysisFifoState<T> {
    /// The unbounded buffer of values.
    buf: RefCell<VecDeque<T>>,

    /// Items readable now (committed at update; the N+1 visibility count).
    num_readable: Cell<usize>,

    /// Items written this delta, not yet readable (committed at the next update).
    num_written: Cell<usize>,

    /// Fired (next delta) when newly-written items become readable.
    data_written: EventId,
}

impl<T: 'static> UpdatableChannel for AnalysisFifoState<T> {
    fn update(&self, ctx: &Ctx) {
        let written = self.num_written.replace(0);
        if written > 0 {
            self.num_readable.set(self.num_readable.get() + written);
            ctx.notify(self.data_written); // readable next delta
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// An unbounded analysis FIFO. A `Copy` handle; the buffer lives in the arena.
pub struct AnalysisFifo<T> {
    /// The channel id.
    id: ChanId,

    /// The data-written event id.
    data_written: EventId,

    /// Carries `T` without owning it.
    _t: PhantomData<T>,
}

// Manual `Copy`/`Clone`/`Debug`: the handle is plain ids, so they must not require
// `T: Copy`/`Clone`/`Debug` (the derive would wrongly add those bounds via `T`).
impl<T> Clone for AnalysisFifo<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for AnalysisFifo<T> {}

impl<T> core::fmt::Debug for AnalysisFifo<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnalysisFifo")
            .field("id", &self.id)
            .field("data_written", &self.data_written)
            .finish()
    }
}

impl<T: Clone + 'static> AnalysisFifo<T> {
    /// Creates an empty unbounded analysis FIFO.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name (reserved).
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new FIFO.
    pub fn new(sim: &Sim, name: &str) -> Self {
        let _ = name;
        let data_written = sim.alloc_event();
        let state = Rc::new(AnalysisFifoState::<T> {
            buf: RefCell::new(VecDeque::new()),
            num_readable: Cell::new(0),
            num_written: Cell::new(0),
            data_written,
        });
        let id = sim.register_channel(state);
        AnalysisFifo {
            id,
            data_written,
            _t: PhantomData,
        }
    }

    /// Returns the event fired (next delta) when items become readable.
    pub fn data_written_event(&self) -> EventId {
        self.data_written
    }

    /// Returns the number of currently-readable items.
    ///
    /// # Arguments
    ///
    /// * `ctx` - A kernel handle.
    pub fn num_available(&self, ctx: &Ctx) -> usize {
        with_state::<T, _, _>(ctx, self.id, |st| st.num_readable.get())
    }

    /// Removes and returns the next readable item, if any.
    ///
    /// # Arguments
    ///
    /// * `ctx` - A kernel handle.
    ///
    /// # Returns
    ///
    /// The next item, or `None` if none is readable yet.
    pub fn try_get(&self, ctx: &Ctx) -> Option<T> {
        with_state::<T, _, _>(ctx, self.id, |st| {
            if st.num_readable.get() == 0 {
                return None;
            }
            st.num_readable.set(st.num_readable.get() - 1);
            st.buf.borrow_mut().pop_front()
        })
    }

    /// Drains **all** currently-readable items in order (the stream-face
    /// drain-all-per-wake semantics).
    ///
    /// # Arguments
    ///
    /// * `ctx` - A kernel handle.
    ///
    /// # Returns
    ///
    /// All readable items, in FIFO order.
    pub fn drain(&self, ctx: &Ctx) -> Vec<T> {
        let mut out = Vec::new();
        while let Some(v) = self.try_get(ctx) {
            out.push(v);
        }
        out
    }
}

impl<T: Clone + 'static> AnalysisWrite<T> for AnalysisFifo<T> {
    /// Buffers `txn`; always succeeds (unbounded — no back-pressure). The value
    /// becomes readable on the next delta. Reaches the kernel via [`Ctx::current`]
    /// (analysis writes happen inside a running process).
    fn write(&self, txn: &T) {
        let ctx = Ctx::current();
        with_state::<T, _, _>(&ctx, self.id, |st| {
            st.buf.borrow_mut().push_back(txn.clone());
            st.num_written.set(st.num_written.get() + 1);
        });
        ctx.request_update(self.id);
    }
}

/// Downcasts the kernel-held channel state for `id` and runs `f` against it.
fn with_state<T, F, R>(ctx: &Ctx, id: ChanId, f: F) -> R
where
    T: 'static,
    F: FnOnce(&AnalysisFifoState<T>) -> R,
{
    let rc = ctx
        .channel(id)
        .expect("analysis fifo channel is registered");
    let st = rc
        .as_any()
        .downcast_ref::<AnalysisFifoState<T>>()
        .expect("channel id refers to an AnalysisFifo of this value type");
    f(st)
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::Mutex;

    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;

    use super::AnalysisFifo;
    use crate::analysis_port::{AnalysisPort, AnalysisWrite};

    /// EC2: a flood of writes in one delta never back-pressures, and all are readable
    /// (in order) on the next delta.
    #[test]
    fn unbounded_no_back_pressure() {
        let sim = Sim::new();
        let fifo: AnalysisFifo<u32> = AnalysisFifo::new(&sim, "f");
        let event = fifo.data_written_event();
        let drained: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
        let d = Arc::clone(&drained);

        sim.add_thread("io", &[], true, move |cx| {
            // Write 10_000 items in a single delta — none can stall the model.
            for i in 0..10_000u32 {
                fifo.write(&i);
            }
            // Not readable in the same delta.
            assert_eq!(fifo.num_available(cx), 0);
            // Next delta: all readable, drained in order.
            cx.wait_event(event);
            *d.lock().expect("lock") = fifo.drain(cx);
        });

        sim.run_until(SimTime::from_ns(10));
        let got = drained.lock().expect("lock");
        assert_eq!(got.len(), 10_000);
        assert_eq!(got[0], 0);
        assert_eq!(got[9_999], 9_999);
    }

    /// An `AnalysisPort` fanning out to an `AnalysisFifo` buffers every value.
    #[test]
    fn port_fans_out_to_fifo() {
        let sim = Sim::new();
        let port = Rc::new(AnalysisPort::<u32>::new());
        let fifo = Rc::new(AnalysisFifo::<u32>::new(&sim, "f"));
        port.bind(&fifo);
        let event = fifo.data_written_event();
        let f = *fifo;
        let out: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
        let o = Arc::clone(&out);
        let p = Rc::clone(&port);

        sim.add_method("producer", &[], true, move |_cx| {
            p.write(&1);
            p.write(&2);
            p.write(&3);
        });
        sim.add_thread("consumer", &[], true, move |cx| {
            cx.wait_event(event);
            *o.lock().expect("lock") = f.drain(cx);
        });

        sim.run_until(SimTime::from_ns(10));
        assert_eq!(*out.lock().expect("lock"), vec![1, 2, 3]);
    }
}
