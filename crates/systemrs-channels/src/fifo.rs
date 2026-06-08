//! `Fifo<T>`: a bounded blocking FIFO honouring the evaluate/update visibility rule.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::rc::Rc;

use systemrs_kernel::{ChanId, Ctx, EventId, Sim, UpdatableChannel};

/// Kernel-held state for a [`Fifo`].
///
/// Models `sc_fifo`'s counters (`doc/systemrs-design.md` §3.7): a value put in
/// delta N is not gettable until N+1, because `num_readable` (which `read` consults
/// via `num_readable - num_read`) is only refreshed at the update phase.
pub(crate) struct FifoState<T> {
    /// All buffered elements (committed plus written-this-delta).
    buf: RefCell<VecDeque<T>>,

    /// Maximum number of buffered elements.
    cap: usize,

    /// Elements readable in the current delta (refreshed at update).
    num_readable: Cell<usize>,

    /// Reads performed in the current delta.
    num_read: Cell<usize>,

    /// Writes performed in the current delta.
    num_written: Cell<usize>,

    /// Fired (next delta) when at least one element was written this delta.
    data_written: EventId,

    /// Fired (next delta) when at least one element was read this delta.
    data_read: EventId,
}

impl<T: 'static> UpdatableChannel for FifoState<T> {
    fn update(&self, ctx: &Ctx) {
        if self.num_read.get() > 0 {
            ctx.notify(self.data_read);
        }
        if self.num_written.get() > 0 {
            ctx.notify(self.data_written);
        }
        // Refresh the readable count to the whole (post-read) buffer, so anything
        // written this delta becomes readable starting next delta.
        self.num_readable.set(self.buf.borrow().len());
        self.num_read.set(0);
        self.num_written.set(0);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl<T: 'static> FifoState<T> {
    /// Elements available to read right now (`num_readable - num_read`).
    fn available(&self) -> usize {
        self.num_readable.get() - self.num_read.get()
    }

    /// Attempts a non-blocking read.
    fn try_get(&self, ctx: &Ctx, id: ChanId) -> Option<T> {
        if self.available() == 0 {
            return None;
        }
        let value = self.buf.borrow_mut().pop_front();
        if value.is_some() {
            self.num_read.set(self.num_read.get() + 1);
            ctx.request_update(id);
        }
        value
    }

    /// Attempts a non-blocking write, returning the value back on overflow.
    fn try_put(&self, ctx: &Ctx, id: ChanId, value: T) -> Result<(), T> {
        if self.buf.borrow().len() >= self.cap {
            return Err(value);
        }
        self.buf.borrow_mut().push_back(value);
        self.num_written.set(self.num_written.get() + 1);
        ctx.request_update(id);
        Ok(())
    }
}

/// A bounded blocking FIFO (`sc_fifo`).
///
/// `put`/`get` block (yield the calling thread) until space/data is available;
/// `try_put`/`try_get` are the non-blocking forms. Written values become readable
/// only in the following delta.
///
/// # Examples
///
/// A producer outruns a capacity-2 FIFO; `put` blocks until the consumer drains it,
/// and order is preserved:
///
/// ```
/// use systemrs_channels::Fifo;
/// use systemrs_kernel::Sim;
/// use systemrs_time::SimTime;
/// use std::sync::{Arc, Mutex};
///
/// let sim = Sim::new();
/// let fifo: Fifo<u32> = Fifo::new(&sim, "f", 2);
/// let got = Arc::new(Mutex::new(Vec::new()));
///
/// let p = fifo;
/// sim.add_thread("producer", &[], true, move |cx| {
///     for i in 0..3 {
///         p.put(cx, i); // blocks when full, until the consumer makes room
///     }
/// });
/// let g = Arc::clone(&got);
/// sim.add_thread("consumer", &[], true, move |cx| {
///     for _ in 0..3 {
///         g.lock().unwrap().push(fifo.get(cx));
///     }
/// });
///
/// sim.run_until(SimTime::from_ns(10));
/// assert_eq!(*got.lock().unwrap(), vec![0, 1, 2]);
/// ```
#[derive(Clone, Copy)]
pub struct Fifo<T> {
    /// The channel id.
    id: ChanId,

    /// Capacity (for `is_full` and diagnostics).
    cap: usize,

    /// The data-written event.
    data_written: EventId,

    /// The data-read event.
    data_read: EventId,

    /// Carries `T` without owning it.
    _t: PhantomData<T>,
}

impl<T: 'static> Fifo<T> {
    /// Creates a bounded FIFO of capacity `cap`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name (reserved for a future name registry).
    /// * `cap` - The maximum number of buffered elements.
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new FIFO.
    pub fn new(sim: &Sim, name: &str, cap: usize) -> Self {
        let _ = name;
        let data_written = sim.alloc_event();
        let data_read = sim.alloc_event();
        let state = Rc::new(FifoState {
            buf: RefCell::new(VecDeque::<T>::new()),
            cap,
            num_readable: Cell::new(0),
            num_read: Cell::new(0),
            num_written: Cell::new(0),
            data_written,
            data_read,
        });
        let id = sim.register_channel(state);
        Fifo {
            id,
            cap,
            data_written,
            data_read,
            _t: PhantomData,
        }
    }

    /// Returns the number of elements readable right now.
    pub fn num_available(&self, ctx: &Ctx) -> usize {
        self.with_state(ctx, FifoState::available)
    }

    /// Returns the FIFO's capacity.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Non-blocking write; returns `Err(value)` if the FIFO is full.
    ///
    /// # Arguments
    ///
    /// * `ctx` - A kernel handle.
    /// * `value` - The value to enqueue.
    ///
    /// # Errors
    ///
    /// Returns the value back as `Err(value)` if the buffer is full (no ownership
    /// loss).
    pub fn try_put(&self, ctx: &Ctx, value: T) -> Result<(), T> {
        let rc = ctx.channel(self.id).expect("fifo channel is registered");
        let st = rc
            .as_any()
            .downcast_ref::<FifoState<T>>()
            .expect("channel id refers to a Fifo of this element type");
        st.try_put(ctx, self.id, value)
    }

    /// Non-blocking read; returns `None` if no element is readable this delta.
    pub fn try_get(&self, ctx: &Ctx) -> Option<T> {
        let rc = ctx.channel(self.id).expect("fifo channel is registered");
        let st = rc
            .as_any()
            .downcast_ref::<FifoState<T>>()
            .expect("channel id refers to a Fifo of this element type");
        st.try_get(ctx, self.id)
    }

    /// Blocking write: yields the calling thread until space is available.
    ///
    /// # Arguments
    ///
    /// * `ctx` - A kernel handle (must be a thread context — it may `wait`).
    /// * `value` - The value to enqueue.
    pub fn put(&self, ctx: &Ctx, value: T) {
        let mut pending = value;
        loop {
            match self.try_put(ctx, pending) {
                Ok(()) => return,
                Err(returned) => {
                    pending = returned;
                    ctx.wait_event(self.data_read);
                }
            }
        }
    }

    /// Blocking read: yields the calling thread until an element is available.
    ///
    /// # Arguments
    ///
    /// * `ctx` - A kernel handle (must be a thread context — it may `wait`).
    ///
    /// # Returns
    ///
    /// The dequeued element.
    pub fn get(&self, ctx: &Ctx) -> T {
        loop {
            if let Some(value) = self.try_get(ctx) {
                return value;
            }
            ctx.wait_event(self.data_written);
        }
    }

    /// Downcasts the kernel-held state and runs `f`.
    fn with_state<F, R>(&self, ctx: &Ctx, f: F) -> R
    where
        F: FnOnce(&FifoState<T>) -> R,
    {
        let rc = ctx.channel(self.id).expect("fifo channel is registered");
        let st = rc
            .as_any()
            .downcast_ref::<FifoState<T>>()
            .expect("channel id refers to a Fifo of this element type");
        f(st)
    }
}
