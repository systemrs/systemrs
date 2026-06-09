//! A checkpointable model: a self-clocking accumulator — a run-to-completion
//! `SC_METHOD` whose only mutable state is a shared `Cell` (the model's serializable
//! component state) — snapshotted mid-run and restored onto a fresh rebuild, continuing
//! **byte-identically**. The worked example for M7 slice 2 bounded snapshot/restore
//! (`doc/systemrs-design.md` §6f).
//!
//! The kernel checkpoints the *scheduler* (the timeline, the timed wheel, each process's
//! wait state) via [`Sim::snapshot`](systemrs::Sim::snapshot); the *model* checkpoints its
//! own serializable state (here, the accumulator `Cell`) by reading it before the snapshot
//! and writing it after the restore. Together they resume the run exactly.
//!
//! Run it with `cargo run --example checkpoint`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use systemrs::{Sim, SimTime};

/// A recorded sample: `(time_units, accumulator)`.
pub type Sample = (u64, u64);

/// A built accumulator: its serializable state cell and its trajectory log.
pub struct Accumulator {
    /// The accumulator value — the model's serializable state, saved/restored around a
    /// snapshot.
    pub state: Rc<Cell<u64>>,
    /// The recorded `(now, total)` trajectory.
    pub log: Rc<RefCell<Vec<Sample>>>,
}

/// Builds a self-clocking accumulator: every `period`, add `step` and record `(now,
/// total)`. Its mutable state lives entirely in the returned `Cell` — the condition for
/// byte-identical snapshot/restore.
#[must_use]
pub fn build(period: SimTime, step: u64) -> (Sim, Accumulator) {
    let sim = Sim::new();
    let state = Rc::new(Cell::new(0u64));
    let log = Rc::new(RefCell::new(Vec::new()));
    let s = Rc::clone(&state);
    let l = Rc::clone(&log);
    // ANCHOR: model
    // A run-to-completion method whose only mutable state is the shared `state` cell:
    // the condition for byte-identical snapshot/restore.
    sim.add_method("accumulator", &[], true, move |cx| {
        let total = s.get() + step;
        s.set(total);
        l.borrow_mut().push((cx.now().units(), total));
        cx.next_trigger(period);
    });
    // ANCHOR_END: model
    (sim, Accumulator { state, log })
}

/// Runs straight through to `end`, returning the trajectory (the golden reference).
#[must_use]
pub fn run_straight(period: SimTime, step: u64, end: SimTime) -> Vec<Sample> {
    let (sim, acc) = build(period, step);
    sim.run_until(end);
    acc.log.borrow().clone()
}

/// Runs to `split`, snapshots, restores onto a fresh rebuild, and continues to `end`.
///
/// Returns `(pre-split log, post-split log)`; their concatenation must equal the straight
/// run (the snapshot/restore preserved the timeline exactly).
///
/// # Panics
///
/// Panics if the snapshot is not taken at a quiescent boundary or the rebuild does not
/// match — neither happens here (the split is a settled boundary; the rebuild is identical).
#[must_use]
pub fn run_with_checkpoint(
    period: SimTime,
    step: u64,
    split: SimTime,
    end: SimTime,
) -> (Vec<Sample>, Vec<Sample>) {
    // ANCHOR: checkpoint
    // Run to the split, then checkpoint: the kernel scheduler + the model's state cell.
    let (sim1, acc1) = build(period, step);
    sim1.run_until(split);
    let snapshot = sim1.snapshot().expect("snapshot at a quiescent boundary");
    let saved_state = acc1.state.get();
    let before = acc1.log.borrow().clone();

    // Restore onto a fresh rebuild (same constructor calls => same generational ids),
    // restore the model's state, and continue.
    let (sim2, acc2) = build(period, step);
    sim2.restore(&snapshot)
        .expect("restore onto a matching rebuild");
    acc2.state.set(saved_state);
    sim2.run_until(end);
    let after = acc2.log.borrow().clone();
    // ANCHOR_END: checkpoint

    (before, after)
}
