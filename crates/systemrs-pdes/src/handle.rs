//! The crate's single audited `unsafe`: marking [`Region`] `Send` so the orchestrator
//! can drive a slice of regions with `rayon::par_iter_mut`.
//!
//! Compiled only under the `rayon` feature; the default (sequential) build contains no
//! `unsafe` at all, yet is fully deterministic — which is why the determinism tests run
//! on the `unsafe`-free build.

use crate::region::Region;

// SAFETY: A `Region` is `!Send` only because its `Sim` and its outbox/ingress are
// `Rc<RefCell<..>>`. Those `Rc`s are region-LOCAL: no `Rc` is shared with any other
// region. The partition forbids cross-region channels — cross-region communication is
// exclusively via OWNED `BoundaryMessage` payloads (`Box<dyn Any + Send>` deep copies),
// never shared handles. The orchestrator owns the regions in a `Vec`, never clones a
// `Region`, and dispatches them to workers only through `rayon::par_iter_mut`, which
// hands each region as a disjoint, non-aliasing `&mut` to exactly one worker for the
// duration of one run-to-boundary (or commit). No region's `Rc` refcount is ever touched
// from two threads at once, and the `par_iter_mut` join re-establishes a happens-before
// edge before the sequential exchange reads any region state. This single impl is the
// entire trust boundary of the parallel tier.
unsafe impl Send for Region {}
