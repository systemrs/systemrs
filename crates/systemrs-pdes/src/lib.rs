//! Tier-1 conservative, barrier-synchronous **parallel discrete-event simulation
//! (PDES)** for SystemRS.
//!
//! The unit of parallelism is a **region**: a disjoint subgraph running its own
//! single-threaded [`Sim`](systemrs_kernel::Sim) kernel up to a **quantum boundary**.
//! An [`Orchestrator`] drives all regions through a per-quantum, three-phase
//! barrier-synchronous loop — run-to-boundary, deterministic cross-region exchange,
//! commit — synchronizing at quantum boundaries coarser than a delta cycle, with the
//! TLM quantum as the conservative-PDES lookahead (`doc/systemrs-design.md` §8a).
//!
//! **Determinism is the product.** A Tier-1 run produces a result bit-identical to the
//! serial Tier-0 run of the same model with the same quantum + partition, independent of
//! thread count or timing. The cross-region exchange sorts by the canonical key
//! `(deliver_at, dst_region, dst_link, src_seq)` — no address, hash order, or completion
//! order on the path — and all time arithmetic is integer ([`SimTime`](systemrs_time::SimTime)
//! is `u64`-backed). Ship `--verify-determinism` from day one via [`assert_traces_match`]:
//! build a model both as a [`LocalHost`] (Tier-0 golden reference) and a partitioned
//! [`Orchestrator`] (Tier-1), and compare their traces.
//!
//! # Example: the integer quantum grid
//!
//! ```
//! use systemrs_pdes::global_quantum_boundary;
//! use systemrs_time::SimTime;
//!
//! let q = SimTime::from_ns(10);
//! assert_eq!(global_quantum_boundary(0, q), SimTime::from_ns(10)); // end of quantum 0
//! assert_eq!(global_quantum_boundary(2, q), SimTime::from_ns(30)); // end of quantum 2
//! ```
//!
//! # Safety
//!
//! This crate contains exactly one `unsafe` item — `unsafe impl Send for Region` (in
//! `handle.rs`), compiled **only under the `rayon` feature** — the single audited trust
//! boundary of the parallel tier (a region is moved to exactly one worker, used
//! exclusively, never aliased, and shares no `Rc` with another region). See its
//! `// SAFETY:` note. The default (sequential) build is `unsafe`-free, yet fully
//! deterministic, so the determinism tests run on the `unsafe`-free build.

#![cfg_attr(feature = "rayon", allow(unsafe_code))]

mod error;
mod ids;
mod io;
mod link;
mod local;
mod message;
mod orchestrator;
mod region;
mod verify;

#[cfg(feature = "rayon")]
mod handle;

pub use error::PdesError;
pub use ids::{LinkId, RegionId};
pub use link::{BoundaryLink, LinkReceiver, LinkSender};
pub use local::{LocalHost, LocalLink};
pub use orchestrator::{Orchestrator, OrchestratorBuilder, global_quantum_boundary};
pub use region::Region;
pub use verify::assert_traces_match;
