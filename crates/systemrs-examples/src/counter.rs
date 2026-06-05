//! Example 1: an incrementing counter.
//!
//! A [`systemrs::Clock`] drives an `SC_METHOD` that is statically sensitive to the
//! clock's posedge. On each rising edge the method increments a private count and
//! writes it to an output [`systemrs::Signal`]. This exercises the kernel (delta
//! cycles, timed events), processes (`SC_METHOD`), and channels (signal/clock) with
//! the evaluate/update determinism discipline.

use systemrs::prelude::*;

/// Handles to a built counter: the driving clock and the observable count signal.
#[derive(Clone, Copy)]
pub struct Counter {
    /// The driving clock.
    pub clock: Clock,

    /// The current count, written one delta after each posedge.
    pub count: Signal<u32>,
}

/// Builds a counter into `sim`: a clock of `period`, plus a posedge-sensitive
/// method that increments and publishes the count.
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
/// * `period` - The clock period; the counter increments once per period.
///
/// # Returns
///
/// [`Counter`] handles for inspecting the count and clock.
pub fn build(sim: &Sim, period: SimTime) -> Counter {
    let clock = Clock::new(sim, "clk", period);
    let count = Signal::<u32>::new(sim, "count", 0);

    let mut n: u32 = 0;
    sim.method("counter")
        .sensitive_to(clock.posedge_event())
        .dont_initialize()
        .finish(move |cx| {
            n += 1;
            count.write(cx, n);
        });

    Counter { clock, count }
}

#[cfg(test)]
mod tests {
    use super::build;
    use systemrs::prelude::*;

    /// Verifies the counter reaches the expected value after N clock periods.
    #[test]
    fn counts_one_per_period() {
        let sim = Sim::new();
        let counter = build(&sim, SimTime::from_ns(10));

        // Posedges at 0,10,20,30,40 ns → 5 increments by 45 ns.
        sim.run_until(SimTime::from_ns(45));
        assert_eq!(counter.count.read(&sim.ctx()), 5);
    }
}
