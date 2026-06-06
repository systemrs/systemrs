//! Tracing & telemetry for SystemRS (`doc/systemrs-design.md` §3.12, §6e).
//!
//! [`Tracer`] samples signals through `Copy` handles at the kernel's `PostUpdate`
//! stage callback (after the update phase commits), emitting owned, `Send`
//! [`TraceEvent`]s to a [`TraceSink`]. [`MemorySink`] collects them in-process;
//! [`WriterSink`] hands them to an **off-thread** writer over a `Send` channel so
//! telemetry I/O never sits on the simulation hot path — and because sampling is
//! read-only, a traced run is byte-identical to an untraced one.
//!
//! Transaction capture ([`Tracer::record_transaction`]) is LT-path for M5; AT
//! phase-accumulation and VCD/FST backends are deferred follow-ups.

mod record;
mod sink;
mod tracer;

pub use record::{TraceCommand, TraceEvent, TraceResponse, TxnRecord};
pub use sink::{MemorySink, TraceSink, WriterSink};
pub use tracer::Tracer;
