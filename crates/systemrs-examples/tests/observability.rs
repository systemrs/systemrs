//! M5 capstone: the observability stack exercised end-to-end through the facade —
//! synchronous in-order analysis fan-out (a scoreboard), the unbounded analysis
//! stream, and telemetry-on == telemetry-off identity (`doc/systemrs-design.md` §6e).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use systemrs::prelude::*;
use systemrs::{TraceCommand, TxnRecord};

/// A scoreboard subscriber: counts transactions by command, recording arrival order.
#[derive(Default)]
struct Scoreboard {
    reads: Cell<u32>,
    writes: Cell<u32>,
    order: RefCell<Vec<TraceCommand>>,
}

impl AnalysisWrite<TxnRecord> for Scoreboard {
    fn write(&self, rec: &TxnRecord) {
        match rec.command {
            TraceCommand::Read => self.reads.set(self.reads.get() + 1),
            TraceCommand::Write => self.writes.set(self.writes.get() + 1),
            TraceCommand::Ignore => {}
        }
        self.order.borrow_mut().push(rec.command);
    }
}

/// EC1 (capstone): one `write()` reaches a scoreboard *and* an analysis FIFO
/// synchronously and in order; the FIFO buffers every record for next-delta drain.
#[test]
fn scoreboard_and_stream_fan_out() {
    let sim = Sim::new();
    let port = Rc::new(AnalysisPort::<TxnRecord>::new());
    let board = Rc::new(Scoreboard::default());
    let fifo = Rc::new(AnalysisFifo::<TxnRecord>::new(&sim, "telemetry"));
    port.bind(&board);
    port.bind(&fifo);

    let event = fifo.data_written_event();
    let stream = *fifo;
    let drained: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
    let d = Arc::clone(&drained);

    let p = Rc::clone(&port);
    sim.add_method("producer", &[], true, move |cx| {
        for (cmd_write, addr) in [(true, 0u64), (false, 4), (true, 8)] {
            let gp = if cmd_write {
                GenericPayload::write(addr, vec![0xAB])
            } else {
                GenericPayload::read(addr, 1)
            };
            // Synchronous broadcast to every subscriber in registration order.
            p.write(&TxnRecord::from_payload(cx.now(), &gp));
        }
    });
    sim.add_thread("consumer", &[], true, move |cx| {
        cx.wait_event(event);
        *d.lock().expect("lock") = stream.drain(cx).len();
    });

    sim.run_until(SimTime::from_ns(10));

    assert_eq!(board.writes.get(), 2);
    assert_eq!(board.reads.get(), 1);
    assert_eq!(
        *board.order.borrow(),
        vec![TraceCommand::Write, TraceCommand::Read, TraceCommand::Write]
    );
    assert_eq!(*drained.lock().expect("lock"), 3); // every record buffered, no loss
}

/// Runs a `0,1,2` counter signal at 10 ns steps and returns `(now_units, delta)`;
/// optionally traces `count` into a memory sink (returned for inspection).
fn run_counter(trace: bool) -> ((u64, u64), Option<MemorySink>) {
    let sim = Sim::new();
    let count: Signal<u32> = Signal::new(&sim, "count", 0);
    let kept = trace.then(|| {
        let sink = MemorySink::new();
        let tracer = Tracer::new(&sim, Rc::new(sink.clone()));
        tracer.trace_signal(count, "count");
        std::mem::forget(tracer); // keep alive for the run
        sink
    });

    let traj: Arc<Mutex<(u64, u64)>> = Arc::new(Mutex::new((0, 0)));
    let t = Arc::clone(&traj);
    sim.add_thread("driver", &[], true, move |cx| {
        for i in 1..=2u32 {
            cx.wait(SimTime::from_ns(10));
            count.write(cx, i);
        }
        cx.wait(SimTime::from_ns(10));
        *t.lock().expect("lock") = (cx.now().units(), cx.delta_count());
    });
    sim.run_until(SimTime::from_ns(1000));
    (*traj.lock().expect("lock"), kept)
}

/// EC4 (capstone, through the facade): a traced run is byte-identical to an untraced
/// one, and the tracer captured the value changes.
#[test]
fn telemetry_on_off_identical() {
    let (baseline, _) = run_counter(false);
    let (traced, sink) = run_counter(true);
    assert_eq!(baseline, traced, "telemetry must not perturb the schedule");
    assert!(
        sink.expect("sink kept").len() >= 3,
        "captured initial + writes"
    );
}
