//! Example 1: an enable-gated counter.
//!
//! A [`systemrs::Clock`] drives an `SC_METHOD` sensitive to the clock's posedge. On
//! each rising edge the counter samples an external `enable` line and increments its
//! count **only when `enable` is high** — it counts an external impulse, not bare
//! clock cycles. This is the smallest example of synchronous, edge-triggered,
//! externally-gated logic: two channels (a clock and an `enable` signal), an
//! `SC_METHOD`, and the evaluate/update determinism discipline (a level driven
//! *between* edges is sampled cleanly *at* the next edge).
//!
//! Here `enable` is driven by an in-sim stimulus (the unit tests below, or any
//! model); for a genuinely external impulse — a line toggled from outside the
//! simulation — drive `enable` from a `systemrs::ExternalInput` (see the real-time
//! twin example).

use systemrs::prelude::*;

/// Handles to a built gated counter: the clock, the gating `enable` line, and the
/// observable count.
#[derive(Clone, Copy)]
pub struct GatedCounter {
    /// The driving clock.
    pub clock: Clock,

    /// The active-high enable line, sampled at each posedge. Drive it externally.
    pub enable: Signal<bool>,

    /// The current count, written one delta after each *enabled* posedge.
    pub count: Signal<u32>,
}

/// Builds an enable-gated counter into `sim`: a clock of `period`, an `enable` line,
/// and a posedge-sensitive method that increments only when `enable` is high.
///
/// The returned [`GatedCounter::enable`] starts low; a testbench or model drives it.
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
/// * `period` - The clock period.
///
/// # Returns
///
/// [`GatedCounter`] handles for driving `enable` and inspecting the count.
// ANCHOR: build
pub fn build(sim: &Sim, period: SimTime) -> GatedCounter {
    let clock = Clock::new(sim, "clk", period);
    let enable = Signal::<bool>::new(sim, "enable", false);
    let count = Signal::<u32>::new(sim, "count", 0);

    let mut n: u32 = 0;
    sim.method("counter")
        .sensitive_to(clock.posedge_event())
        .dont_initialize()
        .finish(move |cx| {
            // Sample the enable line at the rising edge; count only the impulse.
            if enable.read(cx) {
                n += 1;
                count.write(cx, n);
            }
        });

    GatedCounter {
        clock,
        enable,
        count,
    }
}
// ANCHOR_END: build

#[cfg(test)]
mod tests {
    use super::{GatedCounter, build};
    use systemrs::prelude::*;

    /// Drives `enable` high over a window and verifies only the enabled posedges are
    /// counted.
    #[test]
    fn counts_only_enabled_edges() {
        let sim = Sim::new();
        let GatedCounter { enable, count, .. } = build(&sim, SimTime::from_ns(10));

        // Raise enable between the 0 ns and 10 ns edges, lower it after the 20 ns edge.
        sim.add_thread("impulse", &[], true, move |cx| {
            cx.wait(SimTime::from_ns(2));
            enable.write(cx, true);
            cx.wait(SimTime::from_ns(20)); // t = 22 ns
            enable.write(cx, false);
        });

        // Posedges: 0(off), 10(on→1), 20(on→2), 30(off), 40(off).
        sim.run_until(SimTime::from_ns(45));
        assert_eq!(count.read(&sim.ctx()), 2);
    }

    /// With `enable` never driven, the counter stays at zero across many edges.
    #[test]
    fn stays_zero_when_disabled() {
        let sim = Sim::new();
        let counter = build(&sim, SimTime::from_ns(10));
        sim.run_until(SimTime::from_ns(95));
        assert_eq!(counter.count.read(&sim.ctx()), 0);
    }
}
