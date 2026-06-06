//! Reusable models for the SystemRS reference examples.
//!
//! The runnable binaries live in `examples/`; the model logic lives here so it can
//! be exercised by both the examples and the integration tests in `tests/`.
//!
//! - [`counter`] — an incrementing counter driven by a [`systemrs::Clock`] and an
//!   `SC_METHOD` sensitive to its posedge, writing to a [`systemrs::Signal`].
//! - [`rv32i`] — a basic RV32I CPU hart: an `SC_THREAD` that fetches, decodes, and
//!   executes the RV32I base integer instruction set, with all memory access going
//!   through `b_transport` over an initiator socket to a memory target.

pub mod counter;
pub mod platform;
pub mod rv32i;
