//! M6 exit criterion 3: a recorded journal + seed replays to a **byte-identical
//! transaction trace** (`doc/systemrs-design.md` §6f, §8), with the false-positive
//! guards the critique demanded — the model is *explicitly* instrumented to record
//! transactions, the seed is shown to be load-bearing, and an uninstrumented model
//! yields an empty trace (so the equality cannot pass vacuously).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use systemrs::prelude::*;
use systemrs::{Journal, TraceEvent, TxnRecord};

const SEED: u64 = 0x5EED_1234;

/// Builds the sensor model: a memory + a sensor thread that, on each injected value,
/// draws an address from the RNG (making the seed load-bearing), writes the value
/// there over `b_transport`, and — when `instrumented` — records the transaction to a
/// `MemorySink` service. Returns the sensor event (for the injector) and the sink.
fn build_sensor(sim: &Sim, instrumented: bool) -> (EventId, MemorySink) {
    let mem = Memory::new(16, SimTime::from_ns(1));
    let target = TargetSocket::new(sim, "mem");
    mem.connect(sim, &target);
    let isock = InitiatorSocket::new(sim, "cpu");
    isock.bind(sim, &target);

    // Injected values queue + the trace sink, both reached by the (Send) sensor body
    // via services (it captures only Copy data).
    let queue: Rc<RefCell<VecDeque<u64>>> = Rc::new(RefCell::new(VecDeque::new()));
    sim.register_service(Rc::clone(&queue));
    let sink = MemorySink::new();
    sim.register_service(Rc::new(sink.clone()));

    let sensor_ev = sim.alloc_event();
    sim.add_thread("sensor", &[], true, move |cx| {
        loop {
            cx.wait_event(sensor_ev);
            let queue = cx.service::<RefCell<VecDeque<u64>>>();
            let rng = Rng::from_ctx(cx);
            loop {
                let value = queue.borrow_mut().pop_front();
                let Some(value) = value else { break };
                let addr = rng.gen_range(0, 16); // RNG-driven → seed is load-bearing
                let mut payload =
                    GenericPayload::write(addr, vec![u8::try_from(value & 0xFF).unwrap_or(0)]);
                let mut delay = SimTime::ZERO;
                isock.b_transport(cx, &mut payload, &mut delay);
                if instrumented {
                    let rec = TxnRecord::from_payload(cx.now(), &payload);
                    cx.service::<MemorySink>().emit(TraceEvent::Txn(rec));
                }
            }
        }
    });
    (sensor_ev, sink)
}

/// Records the live run, returning `(transaction trace, journal)`.
fn record_live() -> (Vec<TraceEvent>, Journal) {
    let sim = Sim::new();
    Rng::install(&sim, SEED);
    let (sensor_ev, sink) = build_sensor(&sim, true);

    let (recorder, sender, stop, journal) = journal_input(SEED, move |cx, v| {
        cx.service::<RefCell<VecDeque<u64>>>()
            .borrow_mut()
            .push_back(v);
        cx.notify(sensor_ev);
    });
    attach_external_input(&sim, recorder, stop.clone());

    let producer = thread::spawn(move || {
        for v in [11u64, 22, 33, 44] {
            thread::sleep(Duration::from_millis(15));
            let _ = sender.send(v);
        }
        thread::sleep(Duration::from_millis(15));
        stop.stop();
    });
    sim.run_until(SimTime::INF);
    producer.join().expect("producer");

    let recorded = journal.borrow().clone();
    (sink.events(), recorded)
}

/// Replays `journal` with `seed`, returning the transaction trace (no live thread).
fn replay(journal: Journal, seed: u64) -> Vec<TraceEvent> {
    let sim = Sim::new();
    Rng::install(&sim, seed);
    let (sensor_ev, sink) = build_sensor(&sim, true);
    JournalReplayer::new(journal, move |cx, v| {
        cx.service::<RefCell<VecDeque<u64>>>()
            .borrow_mut()
            .push_back(v);
        cx.notify(sensor_ev);
    })
    .install(&sim);
    sim.run_until(SimTime::INF);
    sink.events()
}

/// EC3: replay reproduces the live transaction trace byte-for-byte.
#[test]
fn journal_and_seed_replay_byte_identically() {
    let (trace_live, journal) = record_live();
    assert!(!trace_live.is_empty(), "live run recorded transactions");
    assert_eq!(journal.records.len(), 4, "all four injections journaled");

    let trace_replay = replay(journal.clone(), journal.seed);
    assert_eq!(
        trace_live, trace_replay,
        "replay is byte-identical to the live run"
    );
}

/// The seed is load-bearing: the same journal with a different seed diverges.
#[test]
fn seed_is_load_bearing() {
    let (trace_live, journal) = record_live();
    let trace_other_seed = replay(journal.clone(), journal.seed ^ 0xFFFF);
    assert_ne!(
        trace_live, trace_other_seed,
        "a different seed yields a different (RNG-driven) trace"
    );
}

/// Negative guard: an uninstrumented model records nothing, so the equality above
/// cannot pass vacuously.
#[test]
fn uninstrumented_model_yields_empty_trace() {
    let sim = Sim::new();
    Rng::install(&sim, SEED);
    let (sensor_ev, sink) = build_sensor(&sim, false); // NOT instrumented
    let (input, sender, stop) = channel_input::<u64, _>(move |cx, v| {
        cx.service::<RefCell<VecDeque<u64>>>()
            .borrow_mut()
            .push_back(v);
        cx.notify(sensor_ev);
    });
    attach_external_input(&sim, input, stop.clone());
    let producer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(15));
        let _ = sender.send(1u64);
        thread::sleep(Duration::from_millis(15));
        stop.stop();
    });
    sim.run_until(SimTime::INF);
    producer.join().expect("producer");
    assert!(
        sink.is_empty(),
        "uninstrumented model records no transactions"
    );
}
