//! TLM-2.0 utilities for SystemRS (`doc/systemrs-design.md` §3.11, §6d, §10.1).
//!
//! The L5 utilities layer above `systemrs-tlm2`: temporal decoupling (the
//! [`QuantumKeeper`] and [`GlobalQuantum`]), the payload event queues, the
//! approximately-timed (AT) four-phase `nb_transport` protocol drivers, the LT↔AT
//! adapters, and convenience sockets.
//!
//! These modernise SystemC's `tlm_utils` while keeping its contracts: the PEQ's
//! "fire on next delta" parity is obtained by routing through the kernel's
//! `notify_delta` (no manual even/odd arithmetic), the quantum keeper's
//! `compute_local_quantum` stays integer-only, and the AT shared-mutable transaction
//! is an `Rc<RefCell<GenericPayload>>` (`Txn`) borrowed briefly per phase.

mod adapter_lt_at;
mod at;
mod global_quantum;
mod peq_cb;
mod peq_get;
mod quantum;

pub use adapter_lt_at::{AtToLtAdapter, LtToAtAdapter, TxnId};
pub use at::next_phase;
pub use global_quantum::{GlobalQuantum, set_global_quantum};
pub use peq_cb::PhaseQueue;
pub use peq_get::PeqWithGet;
pub use quantum::QuantumKeeper;
