//! Digital-twin layer for SystemRS (`doc/systemrs-design.md` §6f).
//!
//! The subsystems a long-lived, observable, wall-clock-coupled twin needs that a
//! batch simulator lacks — layered on the deterministic core as the **L6 twin
//! crate** (depends on the kernel + time only):
//!
//! - [`RealTimePacer`] — paces wall clock to sim time at the kernel's time-advance
//!   hook (only time advance is paced; deltas stay instantaneous), exposing slip as a
//!   plain [`PacerStats`] (no `systemrs-trace` dependency, so no dependency cycle).
//! - [`ExternalInput`] + [`attach_external_input`] — an mpsc inbox so an externally
//!   driven model **parks** (does not exit) when idle and resumes on injection
//!   (suspend-on-starvation, the critical twin feature).
//! - [`Rng`] — a seeded, deterministic PRNG service (no ambient `thread_rng`).
//! - [`Journal`] / [`JournalRecorder`] / [`JournalReplayer`] — record injections +
//!   seed so a recorded run replays to a byte-identical transaction trace.
//!
//! Snapshot/restore and structural hot-swap are deferred to M7 (§12).

mod input;
mod journal;
mod pacer;
mod replay;
mod rng;
mod twin;

pub use input::{
    ChannelInput, ChannelInputSender, ExternalInput, StopSignal, attach_external_input,
    channel_input,
};
pub use journal::{InjectionKind, InjectionRecord, Journal, JournalRecorder, journal_input};
pub use pacer::{PacerStats, RealTimePacer};
pub use replay::JournalReplayer;
pub use rng::Rng;
pub use twin::TwinBuilder;
