//! The owned cross-region message and its canonical exchange-sort key.

use std::any::Any;

use systemrs_time::SimTime;

use crate::ids::{LinkId, RegionId};

/// An owned, type-erased boundary payload.
///
/// Cross-region communication is by **owned deep copy** (never a shared `Rc`), which is
/// exactly what keeps a region's `!Send` core sound to move to one worker
/// (`doc/systemrs-design.md` §8a). `Send` so a message may be routed across the barrier
/// even under the `rayon` feature.
pub(crate) type ErasedPayload = Box<dyn Any + Send>;

/// A region-local monotonic send sequence: the final, total tie-break that makes the
/// exchange sort independent of address, hash order, or which worker finished first.
pub(crate) type SrcSeq = u64;

/// One buffered cross-region message: an owned payload plus its canonical routing key.
///
/// Produced into a region's outbox during the parallel phase, drained and sorted during
/// the (sequential, deterministic) exchange phase, then delivered into the destination
/// region's ingress at commit.
pub(crate) struct BoundaryMessage {
    /// Absolute time the payload becomes visible in the destination region. Always
    /// `>= boundary` — the conservative-PDES guarantee (`send_time + latency`, with
    /// `latency >= quantum`).
    pub(crate) deliver_at: SimTime,
    /// Destination region (the primary sort key after time; also the inbox index).
    pub(crate) dst_region: RegionId,
    /// Destination link within that region.
    pub(crate) dst_link: LinkId,
    /// Per-source-region monotonic sequence: the final tie-break.
    pub(crate) src_seq: SrcSeq,
    /// The owned payload, delivered to the destination link's queue.
    pub(crate) payload: ErasedPayload,
}

impl BoundaryMessage {
    /// The canonical, **total** exchange-sort key: `(deliver_at, dst_region, dst_link,
    /// src_seq)`.
    ///
    /// No field is an address, a hash-iteration order, or a completion order — so the
    /// routed order is identical regardless of thread count or timing. The key is total
    /// because `(dst_region, dst_link)` identifies the unique source region, so two
    /// messages sharing it came from the same region and therefore have distinct
    /// `src_seq`.
    pub(crate) fn sort_key(&self) -> (SimTime, RegionId, LinkId, SrcSeq) {
        (
            self.deliver_at,
            self.dst_region,
            self.dst_link,
            self.src_seq,
        )
    }
}
