//! Integer simulation time for SystemRS.
//!
//! [`SimTime`] is a 64-bit count of resolution units. Per `doc/systemrs-design.md`
//! §6a and principle 5 (§5), every operation that advances or compares simulation
//! time is **integer-only** — `f64` appears solely in one-shot delay derivations
//! ([`SimTime::scaled`]) and never inside a per-step accumulation, so floating
//! point non-associativity can never enter the committed timeline.
//!
//! [`SimTime::INF`] is `u64::MAX`, which is bit-for-bit equal to SystemC's
//! `sc_time::max()` (whose `max_time_tag` constructor is literally `~value_type{}`,
//! `sc_time.h:254-256`) — an exact match, not merely a chosen sentinel.

mod resolution;
mod sim_time;

pub use resolution::Resolution;
pub use sim_time::SimTime;
