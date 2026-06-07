//! [`RealTimePacer`] — paces wall clock to simulation time (`doc/systemrs-design.md`
//! §6f).
//!
//! Installed on the kernel's time-advance hook, so **only time advance is paced**;
//! delta cycles stay instantaneous. On each advance it computes the target wall-clock
//! offset from the simulation time elapsed since the pacing epoch (in femtoseconds,
//! so sub-nanosecond resolutions do not round to zero) scaled by a factor, and sleeps
//! if the sim has run ahead of wall clock beyond a tolerance. Slip is exposed as a
//! plain [`PacerStats`] getter — never a `TraceEvent` — so the pacer needs no
//! dependency on `systemrs-trace` (which would form a cycle with the kernel).

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use systemrs_kernel::{Ctx, Sim};
use systemrs_time::SimTime;

/// Pacing statistics (a plain `Copy` snapshot read via [`RealTimePacer::stats`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PacerStats {
    /// Number of time advances paced.
    pub advances: u64,

    /// Signed slip at the last advance, nanoseconds (`+` = behind wall clock, i.e.
    /// the sim is slower than wall; `-` = ahead, the sim ran faster and was slept).
    pub last_slip_ns: i64,

    /// Largest absolute slip observed, nanoseconds.
    pub max_abs_slip_ns: u64,

    /// Number of advances where the pacer slept to re-align (sim ahead of wall).
    pub corrections: u64,
}

/// Paces wall clock to simulation time on the kernel's time-advance hook.
pub struct RealTimePacer {
    /// Wall nanoseconds per simulation nanosecond. `1.0` = real time; `< 1.0` runs
    /// faster than real time; `> 1.0` slower.
    scale: f64,

    /// Slip tolerance: the sim may run this far ahead of wall clock before the pacer
    /// sleeps to re-align.
    tolerance: SimTime,

    /// Wall-clock instant of the first paced advance (the pacing epoch).
    start: Cell<Option<Instant>>,

    /// Simulation time at the pacing epoch.
    sim_epoch: Cell<SimTime>,

    /// Femtoseconds per time unit, resolved from the kernel at the first advance.
    fs_per_unit: Cell<u64>,

    /// Accumulated statistics.
    stats: Cell<PacerStats>,
}

impl RealTimePacer {
    /// Creates a pacer with the given wall-per-sim `scale` and slip `tolerance`.
    ///
    /// # Arguments
    ///
    /// * `scale` - Wall nanoseconds per simulation nanosecond (`1.0` = real time).
    /// * `tolerance` - How far ahead of wall clock the sim may run before sleeping.
    ///
    /// # Returns
    ///
    /// A shared [`RealTimePacer`] (install it with [`RealTimePacer::install`]).
    pub fn new(scale: f64, tolerance: SimTime) -> Rc<RealTimePacer> {
        Rc::new(RealTimePacer {
            scale,
            tolerance,
            start: Cell::new(None),
            sim_epoch: Cell::new(SimTime::ZERO),
            fs_per_unit: Cell::new(1),
            stats: Cell::new(PacerStats::default()),
        })
    }

    /// Installs this pacer on the simulation's time-advance hook.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    pub fn install(self: &Rc<Self>, sim: &Sim) {
        let me = Rc::clone(self);
        sim.set_time_advance_hook(move |cx, from, to| me.on_advance(cx, from, to));
    }

    /// Returns a snapshot of the pacing statistics.
    pub fn stats(&self) -> PacerStats {
        self.stats.get()
    }

    /// Converts a span of `units` to wall nanoseconds at `fs_per_unit`, scaled.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn wall_ns(&self, units: u64, fs_per_unit: u64) -> u128 {
        let sim_fs = u128::from(units) * u128::from(fs_per_unit);
        // f64 is used here only to fold in `scale`; the fs base is integer so
        // sub-ns resolutions never round to zero. Pacing affects wall timing only,
        // never simulation results, so this lossy step cannot perturb determinism.
        let scaled_fs = (sim_fs as f64) * self.scale;
        (scaled_fs as u128) / 1_000_000 // fs → ns
    }

    /// Paces a single time advance (`from` → `to`).
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    fn on_advance(&self, cx: &Ctx, from: SimTime, to: SimTime) {
        if self.start.get().is_none() {
            self.fs_per_unit.set(cx.resolution().fs_per_unit());
            self.sim_epoch.set(from);
            self.start.set(Some(Instant::now()));
        }
        let Some(start) = self.start.get() else {
            return;
        };
        let fs_per_unit = self.fs_per_unit.get();
        let epoch = self.sim_epoch.get();

        let target_ns = self.wall_ns(to.units().saturating_sub(epoch.units()), fs_per_unit);
        let elapsed_ns = start.elapsed().as_nanos();
        let slip = elapsed_ns as i128 - target_ns as i128; // + behind, - ahead

        let tol_ns = self.wall_ns(self.tolerance.units(), fs_per_unit) as i128;
        let mut corrected = false;
        if slip < -tol_ns {
            // The sim ran ahead of wall clock beyond tolerance — sleep to re-align.
            let sleep_ns = (-slip) as u64;
            std::thread::sleep(Duration::from_nanos(sleep_ns));
            corrected = true;
        }

        let mut s = self.stats.get();
        s.advances += 1;
        s.last_slip_ns = slip.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
        let abs = slip.unsigned_abs().min(u128::from(u64::MAX)) as u64;
        s.max_abs_slip_ns = s.max_abs_slip_ns.max(abs);
        if corrected {
            s.corrections += 1;
        }
        self.stats.set(s);
    }
}
