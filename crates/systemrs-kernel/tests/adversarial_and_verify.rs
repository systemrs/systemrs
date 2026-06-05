//! Regression test for the AND-wait stale-subscription bug.
//!
//! A timed-out `wait_event_timeout(e1, …)` leaves a stale `e1` subscription in
//! `e1`'s dynamic list (lazy cleanup). A subsequent `wait_all([e1, e2])` then has
//! `e1` subscribed twice. With a bare `remaining` counter, firing `e1` alone would
//! decrement twice and wrongly complete the AND. The fix tracks the remaining
//! *set* of events, so the decrement is idempotent and the AND only completes once
//! every member has genuinely fired.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use systemrs_kernel::Sim;
use systemrs_time::SimTime;

/// An AND-wait must not complete when only one of its members fires, even in the
/// presence of a stale subscription left by an earlier timed-out wait.
#[test]
fn and_does_not_complete_on_stale_subscription() {
    let sim = Sim::new();
    let e1 = sim.alloc_event();
    let e2 = sim.alloc_event();
    let woke = Arc::new(AtomicU64::new(0));

    let w = Arc::clone(&woke);
    sim.add_thread("p", &[], true, move |cx| {
        // Times out at 5 ns; e1 never fires here, so the e1 subscription leaks.
        cx.wait_event_timeout(e1, SimTime::from_ns(5));
        // Now wait for BOTH e1 and e2.
        cx.wait_all(&[e1, e2]);
        w.fetch_add(1, Ordering::Relaxed);
    });

    sim.add_thread("driver", &[], true, move |cx| {
        // Fire ONLY e1 at 10 ns; e2 is NEVER fired.
        cx.wait(SimTime::from_ns(10));
        cx.notify(e1);
        cx.wait(SimTime::from_ns(10));
    });

    sim.run_until(SimTime::from_ns(100));

    // Correct (SystemC) behaviour: the AND must NOT complete since e2 never fired.
    assert_eq!(
        woke.load(Ordering::Relaxed),
        0,
        "AND wrongly woke on e1 alone (stale-subscription double-decrement)"
    );
}

/// A well-formed AND-wait completes exactly once, only after all members fire.
#[test]
fn and_completes_once_all_members_fire() {
    let sim = Sim::new();
    let e1 = sim.alloc_event();
    let e2 = sim.alloc_event();
    let woke = Arc::new(AtomicU64::new(0));

    let w = Arc::clone(&woke);
    sim.add_thread("p", &[], true, move |cx| {
        cx.wait_all(&[e1, e2]);
        w.fetch_add(1, Ordering::Relaxed);
    });
    sim.add_thread("driver", &[], true, move |cx| {
        cx.wait(SimTime::from_ns(10));
        cx.notify(e1);
        cx.wait(SimTime::from_ns(10));
        cx.notify(e2); // now both have fired
        cx.wait(SimTime::from_ns(10));
    });

    sim.run_until(SimTime::from_ns(100));
    assert_eq!(woke.load(Ordering::Relaxed), 1);
}
