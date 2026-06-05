//! Primitive channels for SystemRS.
//!
//! These reproduce SystemC's evaluate-then-update determinism
//! (`doc/systemrs-design.md` §3.6, §6c): a `write`/`put` only *stages* a value and
//! calls `request_update`; the committed value becomes visible only after the
//! update phase, so process execution order within a delta cannot affect read
//! values. Value-changed events fire one delta later.
//!
//! Channel *handles* are `Copy`/`Send` (an id + phantom type); the state lives in
//! the kernel arena. This lets the same handle be used from both `SC_METHOD`
//! bodies and (Send-required) `SC_THREAD` bodies, refer-by-id per the design (§6a).

mod clock;
mod fifo;
mod signal;

pub use clock::Clock;
pub use fifo::Fifo;
pub use signal::{Buffer, Signal};

#[cfg(test)]
mod tests;
