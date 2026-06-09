//! Bounded snapshot/restore of the kernel scheduler state (M7 slice 2, design §6f).
//!
//! A [`KernelSnapshot`] captures the **kernel-visible** scheduler state at a *quiescent
//! timestep boundary* — the determinism counters, the timed wheel, each event's pending
//! notification + dynamic subscriber lists, and each process's wait state — **not** the
//! process bodies. Coroutine (`SC_THREAD`) stack frames cannot be serialized
//! (`doc/systemrs-design.md` §6f: transparent native-stack capture is research-grade), so
//! restore re-enters a process at its wait continuation, not at a resumed raw stack.
//!
//! ## What restores byte-identically
//!
//! Restore applies a snapshot to a **freshly rebuilt model** — one constructed with the
//! exact same sequence of `alloc_event` / `add_method` / `add_thread` calls, so the
//! generational ids line up. Because the bodies are fresh, byte-identical continuation is
//! guaranteed for **run-to-completion `SC_METHOD`s whose mutable state lives in channels
//! or services** (not in closure captures): a method re-runs from scratch on each trigger,
//! so a fresh closure plus restored component state continues the original timeline
//! exactly. An `SC_THREAD` that holds live locals on its native stack across a `wait`
//! cannot be resumed mid-body and is therefore **out of scope** for this first cut
//! (documented in §6f as the bound).
//!
//! Model state held in channels/services (the design's "arena columns") is the model
//! author's to save and restore around the snapshot — the kernel checkpoints the
//! scheduler; you checkpoint your serializable component state. Automatic channel
//! serialization (a `Snapshot` trait on every channel) and on-disk persistence are
//! additive follow-ons.

use std::collections::BinaryHeap;

use systemrs_diag::ReportError;
use systemrs_time::SimTime;

use crate::event::Pending;
use crate::ids::{EventId, ProcId};
use crate::inner::Inner;
use crate::phase::Phase;
use crate::process::WaitState;
use crate::timed::TimedEntry;

/// An event's restorable state (its pending notification, fire stamp, and the *ordered*
/// dynamic subscriber lists — order is load-bearing for `trigger` determinism).
#[derive(Clone)]
struct EventSnap {
    id: EventId,
    pending: Pending,
    trigger_stamp: u64,
    dynamic_methods: Vec<ProcId>,
    dynamic_threads: Vec<ProcId>,
}

/// A process's restorable wait state (its dynamic sensitivity, the timeout generation,
/// and whether it has terminated). The body is not captured.
#[derive(Clone)]
struct ProcSnap {
    id: ProcId,
    wait: WaitState,
    wait_gen: u64,
    dead: bool,
}

/// A bounded snapshot of the kernel scheduler at a quiescent timestep boundary.
///
/// Produced by [`Sim::snapshot`](crate::Sim::snapshot) and applied by
/// [`Sim::restore`](crate::Sim::restore) to a freshly rebuilt model. Opaque: hold it,
/// then restore it. See the module docs for what restores byte-identically.
#[derive(Clone)]
pub struct KernelSnapshot {
    now: SimTime,
    delta_count: u64,
    change_stamp: u64,
    delta_count_baseline_at_now: u64,
    seq: u64,
    phase: Phase,
    timed: Vec<TimedEntry>,
    events: Vec<EventSnap>,
    procs: Vec<ProcSnap>,
}

impl Inner {
    /// Returns `true` at a quiescent timestep boundary: nothing running, no runnable
    /// process, and no pending update/delta work. The only point a snapshot is legal.
    pub(crate) fn is_quiescent(&self) -> bool {
        self.running.is_none()
            && self.runnable_empty()
            && self.update_queue.is_empty()
            && self.delta_events.is_empty()
            && self.delta_wakes.is_empty()
    }

    /// Captures the kernel-visible scheduler state into a [`KernelSnapshot`].
    pub(crate) fn capture(&self) -> KernelSnapshot {
        let events = self
            .events
            .iter()
            .map(|(id, e)| EventSnap {
                id,
                pending: e.pending,
                trigger_stamp: e.trigger_stamp,
                dynamic_methods: e.dynamic_methods.clone(),
                dynamic_threads: e.dynamic_threads.clone(),
            })
            .collect();
        let procs = self
            .procs
            .iter()
            .map(|(id, p)| ProcSnap {
                id,
                wait: p.wait.clone(),
                wait_gen: p.wait_gen,
                dead: p.dead,
            })
            .collect();
        KernelSnapshot {
            now: self.now,
            delta_count: self.delta_count,
            change_stamp: self.change_stamp,
            delta_count_baseline_at_now: self.delta_count_baseline_at_now,
            seq: self.seq,
            phase: self.phase,
            timed: self.timed.iter().copied().collect(),
            events,
            procs,
        }
    }

    /// Applies a snapshot to this (freshly rebuilt) kernel.
    ///
    /// # Errors
    ///
    /// `SYSTEMRS/SNAPSHOT` if the rebuilt model does not match the snapshot (a different
    /// number of processes/events, or an id the snapshot names that no longer exists) —
    /// i.e. the model was not reconstructed with the same sequence of constructor calls.
    pub(crate) fn apply(&mut self, snap: &KernelSnapshot) -> Result<(), ReportError> {
        if snap.procs.len() != self.procs.len() || snap.events.len() != self.events.len() {
            return Err(systemrs_diag::error(
                "SYSTEMRS/SNAPSHOT",
                "snapshot does not match the rebuilt model (process/event count differs); \
                 restore requires reconstructing the model with the same constructor calls",
            ));
        }

        self.now = snap.now;
        self.delta_count = snap.delta_count;
        self.change_stamp = snap.change_stamp;
        self.delta_count_baseline_at_now = snap.delta_count_baseline_at_now;
        self.seq = snap.seq;
        self.phase = snap.phase;
        self.timed = snap.timed.iter().copied().collect::<BinaryHeap<_>>();

        for es in &snap.events {
            let e = self.events.get_mut(es.id).ok_or_else(|| {
                systemrs_diag::error(
                    "SYSTEMRS/SNAPSHOT",
                    "snapshot event id absent in the rebuilt model",
                )
            })?;
            e.pending = es.pending;
            e.trigger_stamp = es.trigger_stamp;
            e.dynamic_methods.clone_from(&es.dynamic_methods);
            e.dynamic_threads.clone_from(&es.dynamic_threads);
        }
        for ps in &snap.procs {
            let p = self.procs.get_mut(ps.id).ok_or_else(|| {
                systemrs_diag::error(
                    "SYSTEMRS/SNAPSHOT",
                    "snapshot process id absent in the rebuilt model",
                )
            })?;
            p.wait.clone_from(&ps.wait);
            p.wait_gen = ps.wait_gen;
            p.dead = ps.dead;
            p.queued = false;
        }

        // Resume as a started sim parked at the restored boundary: do not re-run the
        // initialize pass, and clear all transient runnable/update/delta work (a
        // quiescent snapshot has none, and the rebuild's own initial procs must not run).
        self.started = true;
        self.running = None;
        self.method_pop.clear();
        self.method_push.clear();
        self.thread_pop.clear();
        self.thread_push.clear();
        self.update_queue.clear();
        self.update_pending.clear();
        self.delta_events.clear();
        self.delta_wakes.clear();
        self.initial_procs.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use systemrs_time::SimTime;

    use crate::Sim;

    /// A self-clocking counter method (a run-to-completion `SC_METHOD` whose only state is
    /// a shared `Cell` — the model's serializable component state). Each tick increments
    /// the cell, logs `(now, count)`, and re-arms via `next_trigger`. Returns the sim, the
    /// state cell (saved/restored around a snapshot), and the trajectory log.
    #[allow(clippy::type_complexity)]
    fn build(period: SimTime) -> (Sim, Rc<Cell<u32>>, Rc<RefCell<Vec<(u64, u32)>>>) {
        let sim = Sim::new();
        let state = Rc::new(Cell::new(0u32));
        let log = Rc::new(RefCell::new(Vec::new()));
        let s = Rc::clone(&state);
        let l = Rc::clone(&log);
        sim.add_method("ticker", &[], true, move |cx| {
            let n = s.get() + 1;
            s.set(n);
            l.borrow_mut().push((cx.now().units(), n));
            cx.next_trigger(period);
        });
        (sim, state, log)
    }

    /// A method-based model snapshotted mid-run and restored onto a fresh rebuild
    /// continues to a byte-identical trajectory (the M7 slice-2 exit criterion).
    #[test]
    fn snapshot_restore_continues_byte_identical() {
        let period = SimTime::from_ns(10);
        let split = SimTime::from_ns(45);
        let end = SimTime::from_ns(100);

        // Reference: a straight run.
        let (sim_ref, _s, log_ref) = build(period);
        sim_ref.run_until(end);
        let reference = log_ref.borrow().clone();

        // Snapshot run: run to the split, snapshot + save model state, restore to a fresh
        // sim, restore model state, continue.
        let (sim1, state1, _log1) = build(period);
        sim1.run_until(split);
        let snap = sim1.snapshot().expect("snapshot at a quiescent boundary");
        let saved = state1.get();

        let (sim2, state2, log2) = build(period);
        sim2.restore(&snap)
            .expect("restore onto a matching rebuild");
        state2.set(saved);
        sim2.run_until(end);

        // The restored run's log covers exactly the reference's tail past the split.
        let tail: Vec<_> = reference
            .iter()
            .copied()
            .filter(|&(t, _)| t > split.units())
            .collect();
        assert!(!tail.is_empty(), "there must be ticks after the split");
        assert_eq!(
            log2.borrow().clone(),
            tail,
            "restored run must continue identically"
        );
    }

    /// Snapshotting requires a quiescent boundary; the gate is in place. (A freshly built,
    /// un-run sim with an `initialize` method has a queued initial process — not yet
    /// runnable until started — so this checks the happy path after a run settles.)
    #[test]
    fn snapshot_is_quiescent_after_a_settled_run() {
        let (sim, _s, _l) = build(SimTime::from_ns(10));
        sim.run_until(SimTime::from_ns(25));
        assert!(
            sim.snapshot().is_ok(),
            "a settled run is a quiescent boundary"
        );
    }
}
