//! M6 exit criterion 1: a twin paces to wall clock within tolerance and emits slip
//! telemetry (`doc/systemrs-design.md` §6f). Pacing derives from femtoseconds, so it
//! works at the default (sub-nanosecond) resolution — the floor assertion would fail
//! under a per-unit-integer rounding bug that never sleeps at fine resolution.

use std::time::Instant;

use systemrs::prelude::*;

/// A model advancing 10 µs of sim time (ten 1 µs steps) with a pacer at scale 200
/// (wall-ns per sim-ns) takes ≈ 2 ms of wall clock — it actually sleeps — and reports
/// non-trivial pacing stats.
#[test]
fn pacer_paces_to_wall_clock_and_reports_slip() {
    let sim = Sim::new();
    // scale 200: 10 µs sim → ~2 ms wall. Tolerance 1 µs sim keeps it pacing each step.
    let pacer = RealTimePacer::new(200.0, SimTime::from_us(1));
    pacer.install(&sim);

    sim.add_thread("clock", &[], true, move |cx| {
        for _ in 0..10 {
            cx.wait(SimTime::from_us(1));
        }
    });

    let start = Instant::now();
    sim.run_until(SimTime::from_us(100));
    let wall = start.elapsed();

    // It actually paced (slept): an unpaced run of ten waits finishes in microseconds.
    // A per-unit-integer rounding bug at this resolution would never sleep and fail here.
    assert!(wall.as_micros() >= 800, "pacer slept (wall = {wall:?})");
    // …and stayed within a sane band for CI (target ≈ 2 ms, generous ceiling).
    assert!(
        wall.as_millis() < 500,
        "paced within tolerance (wall = {wall:?})"
    );

    let stats = pacer.stats();
    assert!(stats.advances >= 10, "one advance per timestep");
    assert!(stats.corrections > 0, "slept to re-align at least once");
}
