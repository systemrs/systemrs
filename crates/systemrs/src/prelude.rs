//! The SystemRS prelude: the common types and traits for authoring models.
//!
//! ```
//! use systemrs::prelude::*;
//!
//! let sim = Sim::new();
//! let clk = Clock::new(&sim, "clk", SimTime::from_ns(10));
//! sim.method("tick")
//!     .sensitive_to(clk.posedge_event())
//!     .dont_initialize()
//!     .finish(|_cx| { /* one tick */ });
//! sim.run_until(SimTime::from_ns(100));
//! ```

pub use systemrs_channels::{Buffer, Clock, Fifo, Signal};
pub use systemrs_core::{Build, Elaborate};
pub use systemrs_kernel::{ChanId, Ctx, EventId, ProcId, Sim};
pub use systemrs_time::{Resolution, SimTime};
pub use systemrs_tlm2::{
    ByteEnable, Command, GenericPayload, InitiatorSocket, Memory, Phase, ResponseStatus,
    TargetSocket, TlmSync, Txn, TxnPool,
};
