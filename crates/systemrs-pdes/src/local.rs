//! The Tier-0 reference: a single-kernel host whose links deliver in-kernel.
//!
//! [`LocalHost`] runs an entire model in one [`Sim`] with [`LocalLink`]s whose
//! send/receive use the *same* ingress + arrival-event + self-pacing discipline as the
//! Tier-1 [`BoundaryLink`](crate::BoundaryLink) — only the *delivery* differs (in-kernel
//! `notify_after` instead of the orchestrator's barrier injection). This is the golden
//! reference a Tier-1 partition must reproduce bit-for-bit (`doc/systemrs-design.md`
//! §8a: "build the deterministic single-thread core first as the golden reference").

use std::marker::PhantomData;
use std::rc::Rc;

use systemrs_kernel::{Ctx, EventId, Sim};
use systemrs_time::{Resolution, SimTime};

use crate::ids::LinkId;
use crate::io::{self, RegionIngress};
use crate::link::{LinkReceiver, LinkSender};

/// A typed, latency-bearing, in-kernel link for the Tier-0 reference. `Copy` for every
/// `T` (like [`BoundaryLink`](crate::BoundaryLink)).
pub struct LocalLink<T> {
    link_id: LinkId,
    latency: SimTime,
    arrival_event: EventId,
    _marker: PhantomData<fn(T)>,
}

impl<T> Clone for LocalLink<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for LocalLink<T> {}

impl<T> std::fmt::Debug for LocalLink<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalLink")
            .field("link_id", &self.link_id)
            .field("latency", &self.latency)
            .field("arrival_event", &self.arrival_event)
            .finish_non_exhaustive()
    }
}

impl<T> LocalLink<T> {
    fn new(link_id: LinkId, latency: SimTime, arrival_event: EventId) -> Self {
        LocalLink {
            link_id,
            latency,
            arrival_event,
            _marker: PhantomData,
        }
    }

    /// The event a consumer process is woken by.
    #[must_use]
    pub fn arrival_event(&self) -> EventId {
        self.arrival_event
    }
}

impl<T: Send + 'static> LinkSender<T> for LocalLink<T> {
    fn send(&self, cx: &Ctx, value: T) {
        // In-kernel delivery: enqueue into the host ingress and arm the arrival event at
        // this message's delivery time. The kernel's notify collapse keeps the soonest
        // pending; the consumer's self-pacing `recv` drains the rest — exactly mirroring
        // what the Tier-1 orchestrator does at a barrier.
        let ingress = cx.service::<RegionIngress>();
        ingress.deliver(self.link_id, cx.now() + self.latency, Box::new(value));
        cx.notify_after(self.arrival_event, self.latency);
    }
}

impl<T: Send + 'static> LinkReceiver<T> for LocalLink<T> {
    fn recv(&self, cx: &Ctx) -> T {
        io::recv::<T>(cx, self.link_id, self.arrival_event)
    }
}

/// The Tier-0 single-kernel host: one [`Sim`] plus a shared ingress, with
/// [`LocalLink`]s connecting its processes.
pub struct LocalHost {
    sim: Sim,
    ingress: Rc<RegionIngress>,
    next_link: u32,
}

impl LocalHost {
    /// Creates a Tier-0 host with the default (1 ps) resolution.
    #[must_use]
    pub fn new() -> Self {
        Self::with_resolution(Resolution::default())
    }

    /// Creates a Tier-0 host with an explicit time resolution.
    ///
    /// # Arguments
    ///
    /// * `resolution` - The frozen time resolution (match the Tier-1 regions for a
    ///   like-for-like comparison).
    #[must_use]
    pub fn with_resolution(resolution: Resolution) -> Self {
        let sim = Sim::with_resolution(resolution);
        let ingress = Rc::new(RegionIngress::new());
        sim.register_service(Rc::clone(&ingress));
        LocalHost {
            sim,
            ingress,
            next_link: 0,
        }
    }

    /// The wrapped simulation, for building the model (add threads/methods, etc.).
    #[must_use]
    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    /// Connects an in-kernel link carrying `T` with the given `latency`.
    ///
    /// # Arguments
    ///
    /// * `latency` - The link delay (use the same value as the Tier-1 partition).
    ///
    /// # Returns
    ///
    /// A `Copy` [`LocalLink`] handle for the producer (`send`) and consumer (`recv`).
    pub fn connect<T: Send + 'static>(&mut self, latency: SimTime) -> LocalLink<T> {
        let link_id = LinkId(self.next_link);
        self.next_link += 1;
        let arrival_event = self.sim.alloc_event();
        self.ingress.register(link_id);
        LocalLink::new(link_id, latency, arrival_event)
    }

    /// Runs the reference model to `end` (a single Tier-0 `run_until`).
    ///
    /// # Arguments
    ///
    /// * `end` - The time to stop at.
    pub fn run_until(&self, end: SimTime) {
        self.sim.run_until(end);
    }
}

impl Default for LocalHost {
    fn default() -> Self {
        LocalHost::new()
    }
}
