//! The quantum keeper — per-initiator temporal decoupling (`tlm_quantumkeeper`).
//!
//! An LT initiator accumulates *local time* and only `wait`s (synchronises to the
//! kernel) at quantum-grid boundaries, running ahead of simulation time between
//! syncs (`doc/systemrs-design.md` §3.11, §6d). [`QuantumKeeper::sync`] is the only
//! method that yields the coroutine; all arithmetic is integer `SimTime`.

use systemrs_kernel::Ctx;
use systemrs_time::SimTime;

use crate::global_quantum::global_quantum_from_ctx;

/// A per-initiator temporal-decoupling accumulator.
///
/// Holds the unsynchronised `local_time` ahead of `now` and the absolute
/// `next_sync_point` (a quantum-grid boundary). Construct one per LT initiator,
/// `start` it once, then `inc`/`need_sync`/`sync` around modelled work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuantumKeeper {
    /// Local time accumulated ahead of `Ctx::now` since the last sync.
    local_time: SimTime,

    /// The absolute time of the next required synchronisation (a grid boundary).
    next_sync_point: SimTime,
}

impl QuantumKeeper {
    /// Creates a fresh keeper (call [`QuantumKeeper::start`] before use).
    ///
    /// # Returns
    ///
    /// A keeper with zero local time and an unset sync point.
    pub fn new() -> Self {
        QuantumKeeper {
            local_time: SimTime::ZERO,
            next_sync_point: SimTime::ZERO,
        }
    }

    /// Seeds the first sync point from the current time and global quantum.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    pub fn start(&mut self, cx: &Ctx) {
        self.reset(cx);
    }

    /// Adds `t` to the accumulated local time (modelled work that has not yet been
    /// synchronised to the kernel).
    ///
    /// # Arguments
    ///
    /// * `t` - The local time to add.
    pub fn inc(&mut self, t: SimTime) {
        self.local_time += t;
    }

    /// Sets the accumulated local time directly.
    ///
    /// # Arguments
    ///
    /// * `t` - The new local time.
    pub fn set(&mut self, t: SimTime) {
        self.local_time = t;
    }

    /// Returns the accumulated local time.
    pub fn get_local_time(&self) -> SimTime {
        self.local_time
    }

    /// Returns the initiator's effective current time (`now + local_time`).
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    pub fn get_current_time(&self, cx: &Ctx) -> SimTime {
        cx.now() + self.local_time
    }

    /// Returns `true` if the accumulated time has reached the next sync point.
    ///
    /// Uses `>=` (`tlm_quantumkeeper`), so the boundary tick itself triggers a sync.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    pub fn need_sync(&self, cx: &Ctx) -> bool {
        cx.now() + self.local_time >= self.next_sync_point
    }

    /// Synchronises to the kernel: `wait`s out the accumulated local time, then
    /// re-seeds the next sync point. The only method that yields the coroutine.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    pub fn sync(&mut self, cx: &Ctx) {
        cx.wait(self.local_time);
        self.reset(cx);
    }

    /// Resets local time to zero and grid-aligns the next sync point to
    /// `now + compute_local_quantum(now)`.
    ///
    /// If no global quantum is set, decoupling is disabled (the next sync point is
    /// `now`, so every `need_sync` is true).
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    pub fn reset(&mut self, cx: &Ctx) {
        self.local_time = SimTime::ZERO;
        let now = cx.now();
        let local_quantum = global_quantum_from_ctx(cx)
            .map_or(SimTime::ZERO, |gq| gq.borrow().compute_local_quantum(now));
        self.next_sync_point = now + local_quantum;
    }
}

impl Default for QuantumKeeper {
    fn default() -> Self {
        QuantumKeeper::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;

    use super::QuantumKeeper;
    use crate::global_quantum::set_global_quantum;

    /// E1: an LT initiator runs ahead and syncs on grid boundaries; the sync count
    /// matches a hand calculation and time lands on quantum multiples.
    #[test]
    fn lt_initiator_syncs_on_grid_boundaries() {
        let sim = Sim::new();
        set_global_quantum(&sim, SimTime::from_ns(100));

        let syncs = Arc::new(AtomicU64::new(0));
        let final_now = Arc::new(AtomicU64::new(0));
        let s = Arc::clone(&syncs);
        let f = Arc::clone(&final_now);

        sim.add_thread("lt_initiator", &[], true, move |cx| {
            let mut qk = QuantumKeeper::new();
            qk.start(cx);
            // 25 steps of 13 ns of modelled work = 325 ns total.
            for _ in 0..25 {
                qk.inc(SimTime::from_ns(13));
                if qk.need_sync(cx) {
                    qk.sync(cx);
                    s.fetch_add(1, Ordering::Relaxed);
                }
            }
            // Drain remaining local time so final now reflects all work.
            qk.sync(cx);
            f.store(cx.now().units(), Ordering::Relaxed);
        });

        sim.run_until(SimTime::from_us(10));

        // Hand calc: cumulative work 13,26,...,325 ns; need_sync (>=100) first true at
        // 104 ns (step 8) -> sync to 104; next boundary 200 -> true at 208 (cum 208,
        // step 16) -> sync to 208; next 300 -> true at 312 (cum 312? actually relative
        // accounting): syncs land the absolute clock on the accumulated total at each
        // crossing. The crossings of the 100 ns grid over 325 ns total occur 3 times
        // (>=100, >=200, >=300), so 3 in-loop syncs, then a final drain sync.
        assert_eq!(syncs.load(Ordering::Relaxed), 3);
        // Total modelled time is 325 ns; after the final drain sync, now == 325 ns.
        assert_eq!(
            final_now.load(Ordering::Relaxed),
            SimTime::from_ns(325).units()
        );
    }

    /// `need_sync` flips exactly at the `>=` boundary, and a sync exactly on the grid
    /// re-seeds a full quantum ahead.
    #[test]
    fn need_sync_uses_inclusive_boundary() {
        let sim = Sim::new();
        set_global_quantum(&sim, SimTime::from_ns(100));
        let observed = Arc::new(AtomicU64::new(0));
        let o = Arc::clone(&observed);

        sim.add_thread("t", &[], true, move |cx| {
            let mut qk = QuantumKeeper::new();
            qk.start(cx); // next_sync_point = 100 ns
            qk.inc(SimTime::from_ns(99));
            let before = qk.need_sync(cx); // 99 < 100 -> false
            qk.inc(SimTime::from_ns(1));
            let at = qk.need_sync(cx); // 100 >= 100 -> true
            o.store(u64::from(!before) + 2 * u64::from(at), Ordering::Relaxed);
            qk.sync(cx); // wait 100 ns -> now == 100 ns
            // On the grid boundary, the next sync point is a FULL quantum ahead.
            assert_eq!(qk.next_sync_point, SimTime::from_ns(200));
        });

        sim.run_until(SimTime::from_us(1));
        // before=false (-> !before=1), at=true (-> 2): 1 + 2 = 3.
        assert_eq!(observed.load(Ordering::Relaxed), 3);
    }
}
