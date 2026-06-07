//! Runnable example: an enable-gated counter.
//!
//! Run with `cargo run --example counter`. The counter increments on a clock posedge
//! only while the external `enable` line is high — here a stimulus raises `enable`
//! from 30 ns to 70 ns, so only those edges are counted.

use systemrs::prelude::*;
use systemrs_examples::counter;

/// Builds and runs the gated counter, driving `enable` over a window and printing
/// each observed increment.
fn main() {
    let sim = Sim::new();
    let counter = counter::build(&sim, SimTime::from_ns(10));

    // Stimulus: raise the enable line from 30 ns to 70 ns.
    let enable = counter.enable;
    sim.add_thread("impulse", &[], true, move |cx| {
        cx.wait(SimTime::from_ns(30));
        enable.write(cx, true);
        cx.wait(SimTime::from_ns(40)); // t = 70 ns
        enable.write(cx, false);
    });

    // Monitor: print each new count.
    sim.method("monitor")
        .sensitive_to(counter.count.value_changed_event())
        .dont_initialize()
        .finish(move |cx| {
            println!("[{}] count = {}", cx.now(), counter.count.read(cx));
        });

    println!("Enable-gated counter, 10 ns period, enable high 30–70 ns:");
    sim.run_until(SimTime::from_ns(100));
    println!(
        "Done at {}. Final count = {} (edges at 30,40,50,60,70 ns).",
        sim.now(),
        counter.count.read(&sim.ctx())
    );
}
