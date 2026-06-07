//! Runnable example: a real-time sensor-monitoring digital twin.
//!
//! Run with `cargo run --example twin`. Phase 1 runs the twin **live**: a producer
//! thread injects sensor readings, which the twin processes at a wall-clock cadence
//! set by the real-time pacer (parking in between), printing each processed value as
//! it is broadcast. Phase 2 **replays** the recorded journal with the same seed and
//! confirms it reproduces the exact processed-value sequence — with no live thread
//! and no pacing.

use std::rc::Rc;
use std::thread;
use std::time::Duration;

use systemrs::prelude::*;
use systemrs_examples::twin::{Reading, SensorParams, build_sensor, inject, run_replay};

/// A subscriber that prints each processed reading as the twin emits it.
struct Printer;

impl AnalysisWrite<Reading> for Printer {
    fn write(&self, reading: &Reading) {
        println!("  t = {:>7} units   value = {:>4}", reading.0, reading.1);
    }
}

/// Runs the live (paced) phase and then the replay phase.
fn main() {
    let params = SensorParams::default();
    let raw_readings: Vec<u64> = vec![600, 540, 720, 480, 660, 500, 700, 520];

    // ---- Phase 1: live, paced, externally driven ----
    println!("Live: a producer streams readings; the twin paces them to wall clock.");
    let journal = {
        let sim = Sim::new();
        let sample = sim.alloc_event();
        let port = build_sensor(&sim, &params, sample);
        let printer = Rc::new(Printer);
        port.bind(&printer);

        // Pace each 100 us sample advance to ~20 ms of wall clock (scale = 200).
        RealTimePacer::new(200.0, params.sample_period).install(&sim);

        let (recorder, sender, stop, journal) =
            journal_input(params.seed, move |cx, raw| inject(cx, sample, raw));
        attach_external_input(&sim, recorder, stop.clone());

        let readings = raw_readings.clone();
        let producer = thread::spawn(move || {
            for raw in readings {
                thread::sleep(Duration::from_millis(3)); // arrive faster than paced
                let _ = sender.send(raw);
            }
            thread::sleep(Duration::from_millis(30));
            stop.stop();
        });

        sim.run_until(SimTime::INF);
        producer.join().expect("producer");
        journal.borrow().clone()
    };

    // ---- Phase 2: deterministic replay ----
    println!("\nReplay: the journal + seed reproduce the run (no thread, no pacing).");
    let replayed = run_replay(params, journal);
    for r in &replayed {
        println!("  t = {:>7} units   value = {:>4}", r.0, r.1);
    }
    println!("\nReplayed {} readings deterministically.", replayed.len());
}
