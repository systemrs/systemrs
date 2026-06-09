//! Region-local I/O services and the shared receive discipline.
//!
//! [`RegionOutbox`] and [`RegionIngress`] are registered as `Sim` services so a `Send`
//! process body reaches them through [`Ctx::service`](systemrs_kernel::Ctx::service),
//! and are also held directly by the [`Region`](crate::Region) (or the Tier-0
//! [`LocalHost`](crate::LocalHost)) so the orchestrator can drain/fill them at the
//! barrier. The shared [`recv`] function is the consumer discipline used identically by
//! the Tier-1 [`BoundaryLink`](crate::BoundaryLink) and the Tier-0
//! [`LocalLink`](crate::LocalLink).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};

use systemrs_kernel::{Ctx, EventId};
use systemrs_time::SimTime;

use crate::ids::LinkId;
use crate::message::{BoundaryMessage, ErasedPayload, SrcSeq};

/// A region's outbox: every send on a [`BoundaryLink`](crate::BoundaryLink) from inside
/// the region pushes a message here; the orchestrator drains it at the exchange phase.
pub(crate) struct RegionOutbox {
    /// Buffered outbound messages (this quantum).
    msgs: RefCell<Vec<BoundaryMessage>>,
    /// The per-region monotonic send sequence source.
    seq: Cell<SrcSeq>,
}

impl RegionOutbox {
    pub(crate) fn new() -> Self {
        RegionOutbox {
            msgs: RefCell::new(Vec::new()),
            seq: Cell::new(0),
        }
    }

    /// Returns the next per-region monotonic send sequence (the deterministic tie-break).
    pub(crate) fn next_seq(&self) -> SrcSeq {
        let s = self.seq.get();
        self.seq.set(s + 1);
        s
    }

    /// Buffers an outbound message.
    pub(crate) fn push(&self, msg: BoundaryMessage) {
        self.msgs.borrow_mut().push(msg);
    }

    /// Takes all buffered messages out (drained at the barrier).
    pub(crate) fn drain(&self) -> Vec<BoundaryMessage> {
        std::mem::take(&mut self.msgs.borrow_mut())
    }
}

/// A region's ingress: per-[`LinkId`] delivery queues, filled by the orchestrator at
/// commit (payload + its absolute `deliver_at`) and drained by the consumer via
/// [`recv`].
///
/// Entries on one link are pushed in non-decreasing `deliver_at` order (the exchange
/// sort guarantees it, and later quanta only ever add later deliveries), so the queue
/// front is always the earliest — no per-push sorting is needed.
pub(crate) struct RegionIngress {
    /// One FIFO of `(deliver_at, payload)` per inbound link.
    queues: RefCell<HashMap<LinkId, VecDeque<(SimTime, ErasedPayload)>>>,
}

impl RegionIngress {
    pub(crate) fn new() -> Self {
        RegionIngress {
            queues: RefCell::new(HashMap::new()),
        }
    }

    /// Registers an (initially empty) queue for an inbound link.
    pub(crate) fn register(&self, link: LinkId) {
        self.queues.borrow_mut().entry(link).or_default();
    }

    /// Pushes a delivered payload onto a link's queue (at the back, preserving order).
    pub(crate) fn deliver(&self, link: LinkId, deliver_at: SimTime, payload: ErasedPayload) {
        self.queues
            .borrow_mut()
            .entry(link)
            .or_default()
            .push_back((deliver_at, payload));
    }

    /// The `deliver_at` of the earliest queued payload on `link`, if any.
    pub(crate) fn earliest_pending(&self, link: LinkId) -> Option<SimTime> {
        self.queues
            .borrow()
            .get(&link)
            .and_then(|q| q.front().map(|&(at, _)| at))
    }

    /// Pops the front payload of `link` if its `deliver_at <= now`.
    pub(crate) fn try_take(&self, link: LinkId, now: SimTime) -> Option<ErasedPayload> {
        let mut q = self.queues.borrow_mut();
        let queue = q.get_mut(&link)?;
        match queue.front() {
            Some(&(at, _)) if at <= now => queue.pop_front().map(|(_, p)| p),
            _ => None,
        }
    }
}

/// The shared consumer receive discipline: block until a payload on `link` is ready,
/// then return it (downcast to `T`).
///
/// One `EventId` has a single pending slot, so a batch of deliveries cannot all arm the
/// arrival event at once. Instead the consumer **self-paces**: it drains every payload
/// already ready (`deliver_at <= now`), and otherwise waits — on the link's
/// `arrival_event` when the queue is empty (the sender/orchestrator arms it for the
/// soonest delivery), or on a timed `wait` until the next queued payload's `deliver_at`
/// when one is buffered for the future. This discipline is identical under Tier-0
/// (`LocalLink`) and Tier-1 (`BoundaryLink`), which is what makes their traces match.
///
/// # Arguments
///
/// * `cx` - The consumer process's kernel handle (must be a thread context — it waits).
/// * `link` - The inbound link to receive from.
/// * `arrival_event` - The link's arrival event, fired by the kernel at a delivery time.
///
/// # Returns
///
/// The next payload, in `deliver_at` then `src_seq` order.
///
/// # Panics
///
/// Panics if a queued payload's runtime type is not `T` — impossible by construction,
/// since a link is typed and only its own `send` ever enqueues to it.
pub(crate) fn recv<T: 'static>(cx: &Ctx, link: LinkId, arrival_event: EventId) -> T {
    loop {
        let now = cx.now();
        let (ready, next) = {
            let ingress = cx.service::<RegionIngress>();
            (ingress.try_take(link, now), ingress.earliest_pending(link))
        };
        if let Some(payload) = ready {
            return *payload
                .downcast::<T>()
                .expect("link payload type matches its BoundaryLink<T>");
        }
        match next {
            // A payload is buffered for the future: self-pace to its delivery time.
            Some(at) if at > now => cx.wait(at - now),
            // The front is somehow not-yet-takeable at `now` (should not happen); loop.
            Some(_) => {}
            // Nothing queued: park on the arrival event (armed for the soonest delivery).
            None => cx.wait_event(arrival_event),
        }
    }
}
