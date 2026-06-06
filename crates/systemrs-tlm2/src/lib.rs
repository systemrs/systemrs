//! TLM-2.0 for SystemRS.
//!
//! Keeps SystemC's TLM-2.0 *contracts* bit-for-bit while modernizing the
//! *mechanisms* (`doc/systemrs-design.md` §3.8–3.10, §6d): the generic payload with
//! an **owned** data buffer, sum-type command/response/sync enums replacing
//! signed-int conventions, `TypeId`-keyed extensions replacing RTTI, an `Rc`+pool
//! memory manager, and a kernel-owned socket registry of `Copy` ids dissolving the
//! initiator/target bind cycle. Convenience sockets register boxed closures,
//! replacing the `void*` trampoline.
//!
//! The transport surface is **synchronous** because SystemRS processes are stackful
//! coroutines — `wait()` is an ordinary call inside `b_transport`, not an `async`
//! colour spreading across the forward path.

// Pre-1.0: the faithful `FwTransport`/`BwTransport` traits and the AT/DMI
// scaffolding are part of the public contract but not all exercised by the LT
// examples yet (§12, M4). Allowed per the Rust skill until 1.0.0.
#![allow(dead_code)]

mod extension;
mod gp;
mod memory;
mod mm;
mod phase;
mod protocol;
mod socket;

pub use extension::{Extension, ExtensionMap};
pub use gp::{ByteEnable, Command, GenericPayload, ResponseStatus};
pub use memory::Memory;
pub use mm::{Txn, TxnPool};
pub use phase::{Phase, PhaseId, TlmSync};
pub use protocol::{
    BaseProtocol, BwBaseProtocol, BwTransport, Dmi, DmiAccess, FwTransport, Protocol,
};
pub use socket::{InitiatorSocket, TargetSocket};

#[cfg(test)]
mod tests;
