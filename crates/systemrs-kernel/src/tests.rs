//! Kernel-level conformance tests for the delta cycle and notification semantics.

use crate::Sim;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use systemrs_time::SimTime;

/// A `Send` ordered log used to assert observable orderings across methods and
/// (Send-required) threads in the same simulation.
type Log = Arc<Mutex<Vec<&'static str>>>;

/// Verifies a pure-time `wait` thread advances time monotonically and runs the
/// expected number of times.
#[test]
fn thread_time_waits_advance_monotonically() {
    let sim = Sim::new();
    let count = Arc::new(AtomicU64::new(0));
    let c = Arc::clone(&count);
    sim.add_thread("t", &[], true, move |cx| {
        for _ in 0..5 {
            cx.wait(SimTime::from_ns(2));
            c.fetch_add(1, Ordering::Relaxed);
        }
    });

    sim.run_until(SimTime::from_ns(100));
    assert_eq!(count.load(Ordering::Relaxed), 5);
    assert_eq!(sim.now(), SimTime::from_ns(10));
}

/// Verifies evaluate-phase ordering: methods run before threads within a delta.
#[test]
fn methods_run_before_threads_in_a_delta() {
    let sim = Sim::new();
    let log: Log = Arc::new(Mutex::new(Vec::new()));

    let lm = Arc::clone(&log);
    sim.add_method("m", &[], true, move |_cx| {
        lm.lock().expect("lock").push("method");
    });
    let lt = Arc::clone(&log);
    sim.add_thread("t", &[], true, move |_cx| {
        lt.lock().expect("lock").push("thread");
    });

    sim.run_until(SimTime::from_ns(1));
    assert_eq!(*log.lock().expect("lock"), vec!["method", "thread"]);
}

/// Verifies notification collapse: a delta notification overrides a pending timed
/// one, so the collapsed event fires exactly once.
#[test]
fn notify_collapse_delta_beats_timed() {
    let sim = Sim::new();
    let ev = sim.alloc_event();

    let runs = Arc::new(AtomicU64::new(0));
    let r = Arc::clone(&runs);
    sim.add_method("waiter", &[ev], false, move |_cx| {
        r.fetch_add(1, Ordering::Relaxed);
    });

    sim.add_thread("driver", &[], true, move |cx| {
        cx.notify_after(ev, SimTime::from_ns(50)); // timed
        cx.notify(ev); // delta overrides the pending timed
        cx.wait(SimTime::from_ns(100));
    });

    sim.run_until(SimTime::from_ns(200));
    assert_eq!(runs.load(Ordering::Relaxed), 1);
}

/// Verifies the immediate self-notification guard: a method immediately notifying
/// an event it is statically sensitive to does not re-run itself in the same
/// evaluation.
#[test]
fn immediate_self_notification_guard() {
    let sim = Sim::new();
    let ev = sim.alloc_event();

    let runs = Arc::new(AtomicU64::new(0));
    let r = Arc::clone(&runs);
    sim.add_method("selfnotify", &[ev], true, move |cx| {
        r.fetch_add(1, Ordering::Relaxed);
        cx.notify_now(ev); // immediate self-notify: must not re-queue this process
    });

    sim.run_until(SimTime::from_ns(10));
    assert_eq!(runs.load(Ordering::Relaxed), 1);
}

/// Verifies `triggered()` is false for a never-fired event, even during the
/// initial evaluate phase when `change_stamp == 0` (the trigger-stamp is seeded to
/// `u64::MAX`, matching SystemC, so it cannot alias the initial change stamp).
#[test]
fn triggered_is_false_for_never_fired_event() {
    let sim = Sim::new();
    let ev = sim.alloc_event();
    let seen: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
    let s = Arc::clone(&seen);
    sim.add_method("checker", &[], true, move |cx| {
        *s.lock().expect("lock") = Some(cx.triggered(ev));
    });
    sim.run_until(SimTime::from_ns(1));
    assert_eq!(*seen.lock().expect("lock"), Some(false));
}

/// Verifies a dynamic event wait: a waiter thread wakes when another process
/// delta-notifies the event it waits on.
#[test]
fn dynamic_event_wait_wakes_on_notify() {
    let sim = Sim::new();
    let ev = sim.alloc_event();
    let woke = Arc::new(AtomicU64::new(0));

    let w = Arc::clone(&woke);
    sim.add_thread("waiter", &[], true, move |cx| {
        cx.wait_event(ev);
        w.fetch_add(1, Ordering::Relaxed);
    });
    sim.add_thread("notifier", &[], true, move |cx| {
        cx.wait(SimTime::from_ns(5));
        cx.notify(ev);
        cx.wait(SimTime::from_ns(5));
    });

    sim.run_until(SimTime::from_ns(100));
    assert_eq!(woke.load(Ordering::Relaxed), 1);
    assert_eq!(sim.now(), SimTime::from_ns(10));
}

/// Verifies a thread spawned mid-run via `Ctx::spawn_thread` runs in the current
/// simulation, can `wait`, and that spawn order maps to run order (FIFO).
#[test]
fn spawn_thread_runs_in_current_run() {
    let sim = Sim::new();
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let l = Arc::clone(&log);
    let now_at_child: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
    let n = Arc::clone(&now_at_child);

    sim.add_thread("parent", &[], true, move |cx| {
        // The parent runs at t=5ns, then spawns two children.
        cx.wait(SimTime::from_ns(5));
        let la = Arc::clone(&l);
        cx.spawn_thread("child_a", move |c2| {
            la.lock().expect("lock").push("a-start");
            c2.wait(SimTime::from_ns(1));
            la.lock().expect("lock").push("a-end");
        });
        let lb = Arc::clone(&l);
        let n2 = Arc::clone(&n);
        cx.spawn_thread("child_b", move |c2| {
            lb.lock().expect("lock").push("b-start");
            n2.store(c2.now().units(), Ordering::Relaxed);
        });
    });

    sim.run_until(SimTime::from_ns(100));
    // Both children started this run; FIFO spawn order; the child observed now==5ns.
    assert_eq!(
        *log.lock().expect("lock"),
        vec!["a-start", "b-start", "a-end"]
    );
    assert_eq!(
        now_at_child.load(Ordering::Relaxed),
        SimTime::from_ns(5).units()
    );
}

/// Verifies a spawned body — which must be `Send` and so cannot capture an `Rc` —
/// reaches per-spawn state via a registered service keyed by a `Copy` id (the
/// construction the AT→LT adapter relies on).
#[test]
fn spawn_thread_body_reaches_service_state() {
    use std::cell::RefCell;
    use std::rc::Rc;

    let sim = Sim::new();
    let svc = Rc::new(RefCell::new(0u32));
    sim.register_service(Rc::clone(&svc));

    sim.add_thread("parent", &[], true, |cx| {
        let id = 7u32; // Copy; the only thing captured into the Send body
        cx.spawn_thread("child", move |c2| {
            // Reach the service by type; the !Send Rc is never captured.
            let state = c2.service::<RefCell<u32>>();
            *state.borrow_mut() = id;
        });
    });

    sim.run_until(SimTime::from_ns(1));
    assert_eq!(*svc.borrow(), 7);
}
