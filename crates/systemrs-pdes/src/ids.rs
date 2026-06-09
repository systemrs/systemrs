//! Region and link identifiers — small `Copy` newtypes, ordered structurally so the
//! deterministic exchange-sort key never depends on an address or a hash order.

/// A region's index in the orchestrator's frozen region table.
///
/// Obtained from [`OrchestratorBuilder::add_region`](crate::OrchestratorBuilder::add_region);
/// its numeric value is the region's position in the orchestrator's `Vec`, so routing
/// is an array index, never a hash lookup.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RegionId(pub(crate) u32);

impl RegionId {
    /// The region's index as a `usize` (its slot in the orchestrator's region table).
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A cross-region link's id, unique across one orchestrator.
///
/// Because each link has exactly one source region, the pair `(dst_region, dst_link)`
/// identifies the source — so a per-region send sequence is enough to make the exchange
/// sort total, with no cross-source collision.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LinkId(pub(crate) u32);
