//! The SystemRS discrete-event kernel.
//!
//! This is the centrepiece of `doc/systemrs-design.md` (§6a): a single-threaded,
//! cooperatively non-preemptive scheduler that owns time and runs the strict
//! three-phase delta cycle (EVALUATE → UPDATE → DELTA-NOTIFY) bit-for-bit, along
//! with the immediate > delta > timed notification-collapse rules, the verified
//! `trigger()` subscriber ordering, and the `change_stamp`/`delta_count`
//! accounting. Determinism is the product: tie-breaks SystemC leaves
//! implementation-defined are pinned here (insertion sequence numbers).
//!
//! Everything else in SystemRS — channels, the TLM-2.0 transport surface, the
//! quantum keeper — is a *client* of these primitives.
//!
//! # Example
//!
//! ```
//! use systemrs_kernel::Sim;
//! use systemrs_time::SimTime;
//! use std::sync::Arc;
//! use std::sync::atomic::{AtomicU32, Ordering};
//!
//! let sim = Sim::new();
//! let ticks = Arc::new(AtomicU32::new(0));
//! let t = Arc::clone(&ticks);
//! // A thread that waits 10 ns three times.
//! sim.add_thread("ticker", &[], true, move |cx| {
//!     for _ in 0..3 {
//!         cx.wait(SimTime::from_ns(10));
//!         t.fetch_add(1, Ordering::Relaxed);
//!     }
//! });
//! sim.run_until(SimTime::from_ns(100));
//! assert_eq!(ticks.load(Ordering::Relaxed), 3);
//! assert_eq!(sim.now(), SimTime::from_ns(30));
//! ```

// Pre-1.0: some faithful-API fields/variants are forward-looking (e.g. process
// `name`/`wake`, `OrTimeout`), exercised by later milestones (§12). Allowed per
// the Rust skill until 1.0.0.
#![allow(dead_code)]

mod channel;
mod ctx;
mod event;
mod ids;
mod inner;
mod phase;
mod process;
mod sim;
mod timed;

pub use channel::UpdatableChannel;
pub use ctx::Ctx;
pub use ids::{ChanId, EventId, ObjectId, ProcId};
pub use phase::{GateOutcome, Phase, Stage, Starvation};
pub use process::WakeReason;
pub use sim::Sim;

#[cfg(test)]
mod tests;
