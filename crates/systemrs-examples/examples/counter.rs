//! Runnable example: an incrementing counter.
//!
//! Run with `cargo run --example counter`.

use systemrs::prelude::*;
use systemrs_examples::counter;

/// Builds and runs the counter model, printing each increment as it is observed.
fn main() {
    let sim = Sim::new();
    let period = SimTime::from_ns(10);
    let counter = counter::build(&sim, period);

    // A monitor process: sensitive to the count signal, prints each new value.
    sim.method("monitor")
        .sensitive_to(counter.count.value_changed_event())
        .dont_initialize()
        .finish(move |cx| {
            println!("[{}] count = {}", cx.now(), counter.count.read(cx));
        });

    println!("Incrementing counter, 10 ns period, running to 100 ns:");
    sim.run_until(SimTime::from_ns(100));
    println!(
        "Done at {}. Final count = {}.",
        sim.now(),
        counter.count.read(&sim.ctx())
    );
}
