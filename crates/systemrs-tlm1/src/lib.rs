//! TLM-1.0 for SystemRS: the analysis sublayer (`doc/systemrs-design.md` §3.7, §6e).
//!
//! The non-intrusive telemetry backbone a digital twin observes through: a
//! synchronous fan-out [`AnalysisPort`], an unbounded [`AnalysisFifo`] stream sink
//! that never back-pressures the model, and a timestamped [`AnalysisTriple`].
//!
//! The general TLM-1 message-passing channels (`tlm_fifo` put/get/peek) are a
//! deferred follow-up — the M5 observability deliverable is the analysis sublayer,
//! and bounded blocking FIFOs are already provided by `systemrs_channels::Fifo`.

mod analysis_fifo;
mod analysis_port;
mod analysis_triple;

pub use analysis_fifo::AnalysisFifo;
pub use analysis_port::{AnalysisPort, AnalysisWrite};
pub use analysis_triple::AnalysisTriple;
