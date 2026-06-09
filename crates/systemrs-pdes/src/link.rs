//! Cross-region links: the shared send/receive traits and the Tier-1 [`BoundaryLink`].

use std::marker::PhantomData;

use systemrs_kernel::{Ctx, EventId};
use systemrs_time::SimTime;

use crate::ids::{LinkId, RegionId};
use crate::io::{self, RegionOutbox};
use crate::message::BoundaryMessage;

/// The send side of a latency-bearing link.
///
/// Implemented by both the Tier-1 [`BoundaryLink`] and the Tier-0
/// [`LocalLink`](crate::LocalLink), so a model's process bodies are written once and run
/// unchanged under either tier — which is what makes a determinism comparison honest.
/// `Copy + Send` so a thread body may capture the handle.
pub trait LinkSender<T>: Copy + Send {
    /// Sends `value` over the link (it arrives `latency` later in the destination).
    ///
    /// # Arguments
    ///
    /// * `cx` - The sending process's kernel handle.
    /// * `value` - The owned value to deliver (deep-copied across the boundary).
    fn send(&self, cx: &Ctx, value: T);
}

/// The receive side of a latency-bearing link (see [`LinkSender`]).
pub trait LinkReceiver<T>: Copy + Send {
    /// Blocks until the next payload is ready and returns it, in `(deliver_at, src_seq)`
    /// order. Must be called from a thread context (it waits).
    ///
    /// # Arguments
    ///
    /// * `cx` - The receiving process's kernel handle.
    ///
    /// # Returns
    ///
    /// The next received value.
    fn recv(&self, cx: &Ctx) -> T;
}

/// A typed, latency-bearing, one-way link from a producer region to a consumer region.
///
/// The link's `latency` is the conservative-PDES lookahead and **must be `>= the
/// quantum`** (enforced at
/// [`connect`](crate::OrchestratorBuilder::connect)): a value sent in quantum *k*
/// arrives at `send_time + latency >= the next boundary`, so it can never need delivery
/// within quantum *k* (`doc/systemrs-design.md` §8a).
///
/// The handle is a small `Copy` value (routing ids + the arrival event) — it holds no
/// `Rc`, so a `Send` process body may capture it; send/receive reach the region-local
/// outbox/ingress through services on the running [`Ctx`]. `Copy` for **every** `T`
/// (the payload type appears only in `PhantomData`).
pub struct BoundaryLink<T> {
    dst_region: RegionId,
    link_id: LinkId,
    latency: SimTime,
    arrival_event: EventId,
    _marker: PhantomData<fn(T)>,
}

// Manual Clone/Copy so the bound is unconditional (the derive would add `T: Copy`,
// but the handle stores no `T`).
impl<T> Clone for BoundaryLink<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for BoundaryLink<T> {}

impl<T> std::fmt::Debug for BoundaryLink<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundaryLink")
            .field("dst_region", &self.dst_region)
            .field("link_id", &self.link_id)
            .field("latency", &self.latency)
            .field("arrival_event", &self.arrival_event)
            .finish_non_exhaustive()
    }
}

impl<T> BoundaryLink<T> {
    /// Builds a boundary-link handle (called by the orchestrator's `connect`).
    pub(crate) fn new(
        dst_region: RegionId,
        link_id: LinkId,
        latency: SimTime,
        arrival_event: EventId,
    ) -> Self {
        BoundaryLink {
            dst_region,
            link_id,
            latency,
            arrival_event,
            _marker: PhantomData,
        }
    }

    /// The event a consumer process is woken by; fired by the kernel at a delivery time.
    #[must_use]
    pub fn arrival_event(&self) -> EventId {
        self.arrival_event
    }

    /// The link's latency (its lookahead).
    #[must_use]
    pub fn latency(&self) -> SimTime {
        self.latency
    }
}

impl<T: Send + 'static> LinkSender<T> for BoundaryLink<T> {
    fn send(&self, cx: &Ctx, value: T) {
        let outbox = cx.service::<RegionOutbox>();
        let src_seq = outbox.next_seq();
        outbox.push(BoundaryMessage {
            deliver_at: cx.now() + self.latency,
            dst_region: self.dst_region,
            dst_link: self.link_id,
            src_seq,
            payload: Box::new(value),
        });
    }
}

impl<T: Send + 'static> LinkReceiver<T> for BoundaryLink<T> {
    fn recv(&self, cx: &Ctx) -> T {
        io::recv::<T>(cx, self.link_id, self.arrival_event)
    }
}
