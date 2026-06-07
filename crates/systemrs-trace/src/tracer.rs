//! [`Tracer`] — stage-callback sampling driven at `PostUpdate`.
//!
//! Installs a single `PostUpdate` stage hook (fired after the update phase commits,
//! `doc/systemrs-design.md` §6e) that samples each registered signal through its
//! `Copy` handle — never a long-lived borrow into a mutated signal — and emits a
//! [`TraceEvent`] to the sink on change. Sampling is **read-only** with respect to
//! the schedule, so a traced run is byte-identical to an untraced one.

use std::cell::{Cell, RefCell};
use std::fmt::Display;
use std::rc::Rc;

use systemrs_channels::Signal;
use systemrs_kernel::{Ctx, Sim, Stage};
use systemrs_tlm2::GenericPayload;

use crate::record::{TraceEvent, TxnRecord};
use crate::sink::TraceSink;

/// A registered sampler: reads a signal and yields an event on change.
type Sampler = Box<dyn Fn(&Ctx) -> Option<TraceEvent>>;

/// Drives observability sampling from the kernel's stage callbacks.
pub struct Tracer {
    /// The per-signal samplers, run at each `PostUpdate`.
    samplers: RefCell<Vec<Sampler>>,

    /// Where sampled events are delivered.
    sink: Rc<dyn TraceSink>,
}

impl Tracer {
    /// Creates a tracer and installs its `PostUpdate` stage hook.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `sink` - Where sampled events go.
    ///
    /// # Returns
    ///
    /// The shared [`Tracer`] (retain it to register signals).
    pub fn new(sim: &Sim, sink: Rc<dyn TraceSink>) -> Rc<Tracer> {
        let tracer = Rc::new(Tracer {
            samplers: RefCell::new(Vec::new()),
            sink,
        });
        let driver = Rc::clone(&tracer);
        sim.add_stage_hook(move |cx, stage| {
            if stage == Stage::PostUpdate {
                for sampler in driver.samplers.borrow().iter() {
                    if let Some(event) = sampler(cx) {
                        driver.sink.emit(event);
                    }
                }
            }
        });
        tracer
    }

    /// Registers a signal to sample at every `PostUpdate`; emits a `Signal` event
    /// whenever its committed value changes.
    ///
    /// # Arguments
    ///
    /// * `signal` - The signal handle to sample (read through a `Copy` snapshot).
    /// * `name` - The signal's trace name.
    pub fn trace_signal<T>(&self, signal: Signal<T>, name: &str)
    where
        T: Copy + PartialEq + Display + 'static,
    {
        let name = name.to_owned();
        let last: Cell<Option<T>> = Cell::new(None);
        self.samplers.borrow_mut().push(Box::new(move |cx| {
            let value = signal.read(cx); // Copy snapshot of the committed value
            if last.get() == Some(value) {
                return None;
            }
            last.set(Some(value));
            Some(TraceEvent::Signal {
                name: name.clone(),
                time: cx.now(),
                delta: cx.delta_count(),
                value: format!("{value}"),
            })
        }));
    }

    /// Records a transaction (LT capture) to the sink. Call from a `b_transport`
    /// after servicing the payload. AT phase accumulation is a deferred follow-up.
    ///
    /// # Arguments
    ///
    /// * `ctx` - A kernel handle.
    /// * `payload` - The serviced transaction payload.
    pub fn record_transaction(&self, ctx: &Ctx, payload: &GenericPayload) {
        let rec = TxnRecord::from_payload(ctx.now(), payload);
        self.sink.emit(TraceEvent::Txn(rec));
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::Mutex;

    use systemrs_channels::Signal;
    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;

    use super::Tracer;
    use crate::record::TraceEvent;
    use crate::sink::MemorySink;

    /// Drives a signal `0,1,2,3` at 10 ns steps and returns the model's
    /// `(final now units, final delta_count)` trajectory. If `tracer` is given, the
    /// `count` signal is traced.
    fn run_counter(trace: bool) -> ((u64, u64), Option<MemorySink>) {
        let sim = Sim::new();
        let count: Signal<u32> = Signal::new(&sim, "count", 0);

        let sink = MemorySink::new();
        let kept = if trace {
            let tracer = Tracer::new(&sim, Rc::new(sink.clone()));
            tracer.trace_signal(count, "count");
            // Keep the tracer alive for the whole run.
            std::mem::forget(tracer);
            Some(sink)
        } else {
            None
        };

        let traj: Arc<Mutex<(u64, u64)>> = Arc::new(Mutex::new((0, 0)));
        let t = Arc::clone(&traj);
        sim.add_thread("driver", &[], true, move |cx| {
            for i in 1..=3u32 {
                cx.wait(SimTime::from_ns(10));
                count.write(cx, i);
            }
            cx.wait(SimTime::from_ns(10));
            *t.lock().expect("lock") = (cx.now().units(), cx.delta_count());
        });

        sim.run_until(SimTime::from_ns(1000));
        let out = *traj.lock().expect("lock");
        (out, kept)
    }

    /// EC4: a model traced by an actively-sampling sink has a byte-identical
    /// `(now, delta_count)` trajectory to the untraced run — telemetry-on == off — and
    /// the sink actually captured the value changes.
    #[test]
    fn active_sink_is_schedule_identical() {
        let (baseline, _) = run_counter(false);
        let (traced, sink) = run_counter(true);

        assert_eq!(baseline, traced, "tracing must not perturb the schedule");

        let sink = sink.expect("traced run kept its sink");
        let values: Vec<String> = sink
            .events()
            .into_iter()
            .filter_map(|e| match e {
                TraceEvent::Signal { value, .. } => Some(value),
                TraceEvent::Txn(_) => None,
            })
            .collect();
        // The initial 0 plus the three writes (1,2,3) are all captured, in order.
        assert_eq!(values, vec!["0", "1", "2", "3"]);
    }
}
