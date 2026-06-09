//! A region: a disjoint subgraph running its own single-threaded [`Sim`] kernel.

use std::collections::HashMap;
use std::rc::Rc;

use systemrs_kernel::{EventId, Sim};
use systemrs_time::{Resolution, SimTime};

use crate::ids::{LinkId, RegionId};
use crate::io::{RegionIngress, RegionOutbox};
use crate::message::BoundaryMessage;

/// A region: a disjoint subgraph running its own single-threaded [`Sim`] up to a quantum
/// boundary, with a region-side outbox (orchestrator-drained at exchange) and per-link
/// ingress queues (orchestrator-filled at commit).
///
/// Built exactly like a Tier-0 model — `region.sim().add_thread(...)` etc. A region is
/// `!Send` (its `Sim` is `Rc`/`RefCell`); it becomes movable to a rayon worker only under
/// the `rayon` feature, via the crate's single audited `unsafe impl Send for Region` (see
/// `handle.rs`), whose `// SAFETY:` note carries the full justification.
pub struct Region {
    id: RegionId,
    sim: Sim,
    /// The region's outbox (also a `Sim` service so `BoundaryLink::send` reaches it).
    outbox: Rc<RegionOutbox>,
    /// The region's ingress (also a `Sim` service so `BoundaryLink::recv` reaches it).
    ingress: Rc<RegionIngress>,
    /// Inbound link → its arrival event, populated as links are connected.
    inbound: HashMap<LinkId, EventId>,
}

impl Region {
    /// Creates a region wrapping a fresh `Sim`, with its outbox + ingress registered as
    /// services so process bodies reach them through `Ctx::service`.
    pub(crate) fn new(id: RegionId, resolution: Resolution) -> Self {
        let sim = Sim::with_resolution(resolution);
        let outbox = Rc::new(RegionOutbox::new());
        let ingress = Rc::new(RegionIngress::new());
        sim.register_service(Rc::clone(&outbox));
        sim.register_service(Rc::clone(&ingress));
        Region {
            id,
            sim,
            outbox,
            ingress,
            inbound: HashMap::new(),
        }
    }

    /// The wrapped simulation, for building the region's model.
    #[must_use]
    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    /// This region's id.
    #[must_use]
    pub fn id(&self) -> RegionId {
        self.id
    }

    /// Records an inbound link's arrival event and registers its ingress queue (called
    /// by the orchestrator's `connect` when this region is a link's destination).
    pub(crate) fn register_inbound(&mut self, link: LinkId, arrival: EventId) {
        self.ingress.register(link);
        self.inbound.insert(link, arrival);
    }

    /// Allocates an arrival event in this region's `Sim` (for an inbound link).
    pub(crate) fn alloc_event(&self) -> EventId {
        self.sim.alloc_event()
    }

    /// PARALLEL phase: run this region's delta/timed loop up to `boundary`.
    pub(crate) fn run_to_boundary(&self, boundary: SimTime) {
        self.sim.run_until(boundary);
    }

    /// Drains this region's outbox (exchange phase).
    pub(crate) fn drain_outbox(&self) -> Vec<BoundaryMessage> {
        self.outbox.drain()
    }

    /// COMMIT phase: inject the messages routed to this region — push each owned payload
    /// into its link's ingress queue, then arm each touched link's arrival event at its
    /// earliest pending delivery.
    ///
    /// `msgs` arrives in the canonical exchange order, so per-link delivery order is
    /// preserved by `push_back`. Arrival events are armed in sorted `LinkId` order (not
    /// hash order) so the injection is deterministic regardless of message arrival.
    pub(crate) fn commit(&self, msgs: Vec<BoundaryMessage>) {
        let mut touched: Vec<LinkId> = Vec::new();
        for msg in msgs {
            let link = msg.dst_link;
            self.ingress.deliver(link, msg.deliver_at, msg.payload);
            if !touched.contains(&link) {
                touched.push(link);
            }
        }
        touched.sort_unstable();
        for link in touched {
            if let (Some(at), Some(&arrival)) =
                (self.ingress.earliest_pending(link), self.inbound.get(&link))
            {
                self.sim.schedule_event_at(arrival, at);
            }
        }
    }

    /// The region's current simulation time (for diagnostics / boundary checks).
    #[must_use]
    pub fn now(&self) -> SimTime {
        self.sim.now()
    }
}
