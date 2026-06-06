//! Trace sinks: where [`TraceEvent`]s go.
//!
//! - [`MemorySink`] collects events in-process (for tests/inspection).
//! - [`WriterSink`] pushes events to an **off-thread writer** over a `Send` channel
//!   so telemetry I/O never sits on the simulation hot path
//!   (`doc/systemrs-design.md` §6e). It is flushed and joined deterministically at
//!   end-of-simulation via [`WriterSink::attach`].

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use systemrs_kernel::Sim;

use crate::record::TraceEvent;

/// Where trace events are delivered.
pub trait TraceSink {
    /// Records one event (must not block the simulation).
    ///
    /// # Arguments
    ///
    /// * `event` - The event to record.
    fn emit(&self, event: TraceEvent);
}

/// An in-process sink that collects events in registration order.
#[derive(Clone, Default)]
pub struct MemorySink {
    /// The collected events.
    events: Rc<RefCell<Vec<TraceEvent>>>,
}

impl MemorySink {
    /// Creates an empty memory sink.
    ///
    /// # Returns
    ///
    /// A new [`MemorySink`] (cloneable; clones share the buffer).
    pub fn new() -> Self {
        MemorySink::default()
    }

    /// Returns a snapshot of the collected events.
    pub fn events(&self) -> Vec<TraceEvent> {
        self.events.borrow().clone()
    }

    /// Returns the number of collected events.
    pub fn len(&self) -> usize {
        self.events.borrow().len()
    }

    /// Returns whether no events have been collected.
    pub fn is_empty(&self) -> bool {
        self.events.borrow().is_empty()
    }
}

impl TraceSink for MemorySink {
    fn emit(&self, event: TraceEvent) {
        self.events.borrow_mut().push(event);
    }
}

/// Shared state for an [`WriterSink`]'s off-thread writer.
struct WriterShared {
    /// The send side; dropped to signal the writer thread to finish.
    tx: RefCell<Option<Sender<TraceEvent>>>,

    /// The writer thread handle, joined at flush.
    handle: RefCell<Option<JoinHandle<()>>>,

    /// The formatted output lines (the writer's destination).
    out: Arc<Mutex<Vec<String>>>,
}

/// A sink that hands events to a background OS thread (the one real `Send`
/// boundary). Events are owned + `Send`; the writer formats them to text off the
/// simulation thread.
#[derive(Clone)]
pub struct WriterSink {
    /// The shared writer state.
    shared: Rc<WriterShared>,
}

impl WriterSink {
    /// Spawns the off-thread writer and returns a handle to feed it.
    ///
    /// # Returns
    ///
    /// A new [`WriterSink`].
    pub fn new() -> Self {
        let out = Arc::new(Mutex::new(Vec::new()));
        let out_writer = Arc::clone(&out);
        let (tx, rx) = mpsc::channel::<TraceEvent>();
        // The writer thread drains until the sender is dropped.
        let handle = std::thread::spawn(move || {
            for event in rx {
                if let Ok(mut lines) = out_writer.lock() {
                    lines.push(format!("{event}"));
                }
            }
        });
        WriterSink {
            shared: Rc::new(WriterShared {
                tx: RefCell::new(Some(tx)),
                handle: RefCell::new(Some(handle)),
                out,
            }),
        }
    }

    /// Returns the shared output buffer (valid to read after [`WriterSink::flush`]).
    pub fn output(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.shared.out)
    }

    /// Flushes and joins the writer thread (idempotent). After this returns, every
    /// emitted event is present in [`WriterSink::output`].
    pub fn flush(&self) {
        // Drop the sender so the writer's `for event in rx` loop ends, then join.
        self.shared.tx.borrow_mut().take();
        if let Some(handle) = self.shared.handle.borrow_mut().take() {
            let _ = handle.join();
        }
    }

    /// Registers an end-of-simulation hook that flushes and joins the writer, so the
    /// telemetry is durable and the thread is reaped deterministically.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation to attach to.
    pub fn attach(&self, sim: &Sim) {
        let me = self.clone();
        sim.add_end_of_sim_hook(move |_ctx| me.flush());
    }
}

impl Default for WriterSink {
    fn default() -> Self {
        WriterSink::new()
    }
}

impl TraceSink for WriterSink {
    fn emit(&self, event: TraceEvent) {
        if let Some(tx) = self.shared.tx.borrow().as_ref() {
            let _ = tx.send(event); // never blocks the sim (unbounded channel)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use systemrs_channels::Signal;
    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;

    use super::{MemorySink, WriterSink};
    use crate::tracer::Tracer;

    /// The off-thread `WriterSink` produces, after the end-of-sim flush/join, exactly
    /// the same formatted lines a `MemorySink` collects — proving the `Send` boundary
    /// neither drops nor reorders events.
    #[test]
    fn writer_matches_memorysink_after_flush() {
        // Reference: in-process memory sink.
        let expected = {
            let sim = Sim::new();
            let sig: Signal<u32> = Signal::new(&sim, "x", 0);
            let mem = MemorySink::new();
            let tracer = Tracer::new(&sim, Rc::new(mem.clone()));
            tracer.trace_signal(sig, "x");
            std::mem::forget(tracer);
            run_writes(&sim, sig);
            mem.events()
                .iter()
                .map(|e| format!("{e}"))
                .collect::<Vec<_>>()
        };

        // Off-thread writer sink, flushed at end-of-sim.
        let sim = Sim::new();
        let sig: Signal<u32> = Signal::new(&sim, "x", 0);
        let writer = WriterSink::new();
        writer.attach(&sim); // flush+join wired into end-of-sim
        let out = writer.output();
        let tracer = Tracer::new(&sim, Rc::new(writer));
        tracer.trace_signal(sig, "x");
        std::mem::forget(tracer);
        run_writes(&sim, sig);
        sim.end_of_sim(); // deterministic flush + join

        let lines = out.lock().expect("lock").clone();
        assert_eq!(lines, expected);
        assert!(!lines.is_empty());
    }

    /// Drives `1,2` into `sig` at 10 ns steps.
    fn run_writes(sim: &Sim, sig: Signal<u32>) {
        sim.add_thread("d", &[], true, move |cx| {
            for i in 1..=2u32 {
                cx.wait(SimTime::from_ns(10));
                sig.write(cx, i);
            }
            cx.wait(SimTime::from_ns(10));
        });
        sim.run_until(SimTime::from_ns(1000));
    }
}
