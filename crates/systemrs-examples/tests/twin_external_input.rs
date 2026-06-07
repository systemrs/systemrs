//! M6 exit criterion 2: an externally-driven model **parks** (does not exit) when
//! idle and **resumes** on injection (`doc/systemrs-design.md` §6f). Also pins the
//! finite-`run_until` interaction (a bounded run exits on starvation rather than
//! deadlocking on a never-arriving input).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use systemrs::prelude::*;

/// EC2 + the gate delta-wake regression: a thread parks on an event; a producer on
/// another OS thread injects three DELTA notifications (the path that the empty-delta
/// guard would otherwise drop) with gaps so the sim parks between each. The model
/// wakes exactly three times, never advances time, and the run terminates on stop().
#[test]
fn external_input_parks_then_resumes_on_each_injection() {
    let sim = Sim::new();
    let sensor = sim.alloc_event();

    let wakes = Arc::new(AtomicU32::new(0));
    let w = Arc::clone(&wakes);
    sim.add_thread("sensor-consumer", &[], true, move |cx| {
        loop {
            cx.wait_event(sensor);
            w.fetch_add(1, Ordering::SeqCst);
        }
    });

    // A DELTA injection (cx.notify), deliberately exercising the Resume→commit path.
    let (input, sender, stop) = channel_input::<u32, _>(move |cx, _v| cx.notify(sensor));
    attach_external_input(&sim, input, stop.clone());

    // Producer on another OS thread: park-prove with gaps, inject 3, then stop.
    let producer = thread::spawn(move || {
        for v in 0..3u32 {
            thread::sleep(Duration::from_millis(20)); // sim should be PARKED here
            sender.send(v).expect("sim receiver alive");
        }
        thread::sleep(Duration::from_millis(20));
        stop.stop();
    });

    // Long-lived twin mode: run forever until stopped.
    sim.run_until(SimTime::INF);
    producer.join().expect("producer thread");

    // Resumed on each of the three injections (the gate's delta notify woke the
    // process across the Resume boundary — the headline regression).
    assert_eq!(wakes.load(Ordering::SeqCst), 3);
    // Pure delta injections: time never advanced (it parked at starvation, did not
    // run to a timed end).
    assert_eq!(sim.now(), SimTime::ZERO);
}

/// A finite `run_until(end)` with an input attached but no injection/stop must RETURN
/// (exit on starvation) rather than park forever — the bounded-run interaction.
#[test]
fn finite_run_until_with_input_attached_returns() {
    let sim = Sim::new();
    let sensor = sim.alloc_event();
    sim.add_thread("c", &[], true, move |cx| {
        cx.wait_event(sensor); // waits forever; the model itself never ends
    });

    let (input, sender, stop) = channel_input::<u32, _>(move |cx, _v| cx.notify(sensor));
    attach_external_input(&sim, input, stop);
    // Hold the sender so the channel stays open; never inject, never stop.
    let _keep = sender;

    // A FINITE end: this must return at starvation, not hang.
    sim.run_until(SimTime::from_us(10));
    assert!(sim.now() <= SimTime::from_us(10));
}
