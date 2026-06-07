//! Reusable models for the SystemRS reference examples.
//!
//! The runnable binaries live in `examples/`; the model logic lives here so it can
//! be exercised by both the examples and the integration tests in `tests/`.
//!
//! - [`counter`] — an enable-gated counter: an `SC_METHOD` on a [`systemrs::Clock`]
//!   posedge that increments only while an external `enable` [`systemrs::Signal`] is
//!   high (counting an impulse, not bare clock cycles).
//! - [`rv32i`] — a basic RV32I CPU hart: an `SC_THREAD` that fetches, decodes, and
//!   executes the RV32I base integer instruction set, with all memory access going
//!   through `b_transport` over an initiator socket to a memory target.
//! - [`reverb`] — a fixed-point electric-guitar reverb pedal: blocks of `qfixed`
//!   `Q2.14` samples streamed over `b_transport`, a comb+allpass reverb with a
//!   complex-`CQ` NCO tremolo, and per-block level telemetry on an `AnalysisPort`.
//! - [`dma`] — a register-programmed DMA engine: a CPU programs it over LT
//!   (`b_transport`), and it copies a block over the AT four-phase handshake
//!   (`nb_transport_fw`/`bw` + PEQ) to an `AtMemory`, raising a completion interrupt.

pub mod counter;
pub mod dma;
pub mod platform;
pub mod reverb;
pub mod rv32i;
