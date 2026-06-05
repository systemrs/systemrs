//! Conformance tests for the primitive channels' evaluate/update discipline.

use crate::{Clock, Fifo, Signal};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use systemrs_core::Build;
use systemrs_kernel::Sim;
use systemrs_time::SimTime;

/// Verifies a signal write is only visible after the update phase: a same-delta
/// read returns the old value; the next delta returns the new value.
#[test]
fn signal_write_visible_next_delta() {
    let sim = Sim::new();
    let sig = Signal::<u32>::new(&sim, "sig", 0);
    let seen: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let s = Arc::clone(&seen);

    sim.add_thread("writer", &[], true, move |cx| {
        sig.write(cx, 10);
        s.lock().expect("lock").push(sig.read(cx)); // same delta: old value (0)
        cx.wait(SimTime::ZERO); // one delta later
        s.lock().expect("lock").push(sig.read(cx)); // committed: 10
    });

    sim.run_until(SimTime::from_ns(10));
    assert_eq!(*seen.lock().expect("lock"), vec![0, 10]);
}

/// Verifies a clock drives a posedge-sensitive counter the expected number of
/// times (posedges at 0, 10, 20, … ns).
#[test]
fn clock_drives_counter() {
    let sim = Sim::new();
    let clk = Clock::new(&sim, "clk", SimTime::from_ns(10));

    let count = Rc::new(Cell::new(0u32));
    let c = Rc::clone(&count);
    sim.method("counter")
        .sensitive_to(clk.posedge_event())
        .dont_initialize()
        .finish(move |_cx| c.set(c.get() + 1));

    sim.run_until(SimTime::from_ns(55));
    // Posedges at 0,10,20,30,40,50 → 6 increments.
    assert_eq!(count.get(), 6);
}

/// Verifies the FIFO visibility rule: a value put in delta N is not readable until
/// N+1.
#[test]
fn fifo_value_readable_next_delta() {
    let sim = Sim::new();
    let fifo = Fifo::<i32>::new(&sim, "f", 4);
    let log: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(Vec::new()));
    let l = Arc::clone(&log);

    sim.add_thread("t", &[], true, move |cx| {
        fifo.try_put(cx, 42).expect("space available");
        l.lock().expect("lock").push(fifo.num_available(cx) as i64); // 0 (same delta)
        cx.wait(SimTime::from_ns(1)); // advance a delta
        l.lock().expect("lock").push(fifo.num_available(cx) as i64); // 1 (next delta)
        l.lock()
            .expect("lock")
            .push(fifo.try_get(cx).map_or(-1, i64::from)); // 42
    });

    sim.run_until(SimTime::from_ns(10));
    assert_eq!(*log.lock().expect("lock"), vec![0, 1, 42]);
}

/// Verifies blocking producer/consumer threads transfer all items in order through
/// a bounded FIFO.
#[test]
fn fifo_producer_consumer_in_order() {
    let sim = Sim::new();
    let fifo = Fifo::<i32>::new(&sim, "f", 2);
    let out: Arc<Mutex<Vec<i32>>> = Arc::new(Mutex::new(Vec::new()));
    let o = Arc::clone(&out);

    sim.add_thread("producer", &[], true, move |cx| {
        for i in 0..5 {
            fifo.put(cx, i);
            cx.wait(SimTime::from_ns(1));
        }
    });
    sim.add_thread("consumer", &[], true, move |cx| {
        for _ in 0..5 {
            let v = fifo.get(cx);
            o.lock().expect("lock").push(v);
        }
    });

    sim.run_until(SimTime::from_ns(100));
    assert_eq!(*out.lock().expect("lock"), vec![0, 1, 2, 3, 4]);
}
