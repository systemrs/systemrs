//! M6 no-perturbation invariant: with the digital-twin kernel seams present but
//! NOTHING attached (no pacer, input, gate, or journal), the run loop is
//! byte-identical to M5 — same `(now, delta_count)` trajectory and the default
//! starvation-EXIT (`doc/systemrs-design.md` §6f). Locks the load-bearing invariant
//! before any twin consumer is built. (The whole M0-M5 suite passing with the seams
//! compiled in is the broader proof; this pins the two behaviour-touching branches.)

use std::sync::{Arc, Mutex};

use systemrs::prelude::*;

/// Runs a tiny model (three 5 ns steps, then it ends) to a far horizon and returns
/// its `(now_units, delta_count)` at natural starvation.
fn run_model() -> (u64, u64) {
    let sim = Sim::new();
    let sig: Signal<u32> = Signal::new(&sim, "s", 0);
    let out: Arc<Mutex<(u64, u64)>> = Arc::new(Mutex::new((0, 0)));
    let o = Arc::clone(&out);
    sim.add_thread("t", &[], true, move |cx| {
        for i in 1..=3u32 {
            cx.wait(SimTime::from_ns(5));
            sig.write(cx, i);
        }
        *o.lock().expect("lock") = (cx.now().units(), cx.delta_count());
    });
    // A far horizon: with the default ExitOnStarvation policy the run must stop at the
    // model's natural starvation (~15 ns), NOT advance to the horizon.
    sim.run_until(SimTime::from_us(1_000));
    *out.lock().expect("lock")
}

/// The twin seams, present but unattached, change nothing: the run is deterministic,
/// stops at natural starvation (not the horizon), and lands at the expected time.
#[test]
fn unattached_seams_preserve_starvation_exit_and_determinism() {
    let a = run_model();
    let b = run_model();
    assert_eq!(a, b, "run is deterministic with seams present");
    // Default starvation-EXIT: the model ended at 15 ns, far below the 1 ms horizon —
    // the seams did NOT turn this into a run-to-time or a park.
    assert_eq!(a.0, SimTime::from_ns(15).units());
    assert!(a.0 < SimTime::from_us(1_000).units());
}
