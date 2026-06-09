//! The global PDES orchestrator: the per-quantum, 3-phase barrier-synchronous loop.

use systemrs_time::{Resolution, SimTime};

use crate::error::PdesError;
use crate::ids::{LinkId, RegionId};
use crate::link::BoundaryLink;
use crate::message::BoundaryMessage;
use crate::region::Region;

/// The integer-only quantum boundary (the absolute time at the **end** of quantum
/// `q_index`, 0-based): `(q_index + 1) * quantum`, saturating.
///
/// Integer-only by construction — no `f64` ever touches a committed boundary
/// (`doc/systemrs-design.md` §8a: "compute a quantum boundary … is integer arithmetic
/// only").
///
/// # Arguments
///
/// * `q_index` - The 0-based quantum index.
/// * `quantum` - The global quantum length.
///
/// # Returns
///
/// The absolute boundary time.
#[must_use]
pub fn global_quantum_boundary(q_index: u64, quantum: SimTime) -> SimTime {
    SimTime::from_units(quantum.units().saturating_mul(q_index.saturating_add(1)))
}

/// Builds a [`Orchestrator`]: declare regions, connect latency-bearing links, then
/// `build`. The partition is frozen once built.
pub struct OrchestratorBuilder {
    quantum: SimTime,
    resolution: Resolution,
    regions: Vec<Region>,
    next_link: u32,
}

impl OrchestratorBuilder {
    /// Creates a builder with the given global quantum and the default (1 ps)
    /// resolution.
    #[must_use]
    pub fn new(quantum: SimTime) -> Self {
        Self::with_resolution(quantum, Resolution::default())
    }

    /// Creates a builder with an explicit time resolution (match the Tier-0 reference).
    ///
    /// # Arguments
    ///
    /// * `quantum` - The global quantum (the lookahead floor for every link).
    /// * `resolution` - The frozen time resolution for every region.
    #[must_use]
    pub fn with_resolution(quantum: SimTime, resolution: Resolution) -> Self {
        OrchestratorBuilder {
            quantum,
            resolution,
            regions: Vec::new(),
            next_link: 0,
        }
    }

    /// Adds a region and returns its [`RegionId`] (its index in the frozen table).
    pub fn add_region(&mut self) -> RegionId {
        let id = RegionId(u32::try_from(self.regions.len()).expect("region count fits u32"));
        self.regions.push(Region::new(id, self.resolution));
        id
    }

    /// Borrows a region to build its model (`builder.region(id).sim().add_thread(...)`).
    ///
    /// # Arguments
    ///
    /// * `id` - The region to access.
    #[must_use]
    pub fn region(&self, id: RegionId) -> &Region {
        &self.regions[id.index()]
    }

    /// Connects a typed, latency-bearing link from `src` to `dst`.
    ///
    /// The consumer's arrival event is allocated in `dst`'s `Sim`. Returns the `Copy`
    /// [`BoundaryLink`] handle for the producer (`send`) and consumer (`recv`).
    ///
    /// # Arguments
    ///
    /// * `src` - The producer region.
    /// * `dst` - The consumer region.
    /// * `latency` - The link delay; must be `>= quantum` (the lookahead).
    ///
    /// # Returns
    ///
    /// The link handle.
    ///
    /// # Errors
    ///
    /// [`PdesError::LatencyBelowQuantum`] if `latency < quantum` — the lookahead
    /// constraint is a construction error, surfaced up front rather than at run time.
    pub fn connect<T: Send + 'static>(
        &mut self,
        src: RegionId,
        dst: RegionId,
        latency: SimTime,
    ) -> Result<BoundaryLink<T>, PdesError> {
        let _ = src; // routing is by (dst_region, dst_link); src is informational.
        if latency.units() < self.quantum.units() {
            return Err(PdesError::LatencyBelowQuantum {
                latency: latency.units(),
                quantum: self.quantum.units(),
            });
        }
        let link_id = LinkId(self.next_link);
        self.next_link += 1;
        let arrival = self.regions[dst.index()].alloc_event();
        self.regions[dst.index()].register_inbound(link_id, arrival);
        Ok(BoundaryLink::new(dst, link_id, latency, arrival))
    }

    /// Freezes the partition into an [`Orchestrator`].
    #[must_use]
    pub fn build(self) -> Orchestrator {
        let n = self.regions.len();
        Orchestrator {
            regions: self.regions,
            quantum: self.quantum,
            inboxes: (0..n).map(|_| Vec::new()).collect(),
        }
    }
}

/// The global PDES orchestrator: holds the regions, the global quantum, and the frozen
/// partition; drives the per-quantum 3-phase loop to `end`.
///
/// Deterministic and correct running regions **sequentially**; identical result with the
/// `rayon` feature on. A Tier-1 run is bit-identical to the serial Tier-0 run of the same
/// model with the same quantum + partition (`doc/systemrs-design.md` §8a).
pub struct Orchestrator {
    regions: Vec<Region>,
    quantum: SimTime,
    /// Per-region scratch inbox, reused each quantum.
    inboxes: Vec<Vec<BoundaryMessage>>,
}

impl Orchestrator {
    /// Begins building an orchestrator with the given global quantum.
    #[must_use]
    pub fn builder(quantum: SimTime) -> OrchestratorBuilder {
        OrchestratorBuilder::new(quantum)
    }

    /// The global quantum.
    #[must_use]
    pub fn quantum(&self) -> SimTime {
        self.quantum
    }

    /// Borrows a region after building (e.g. to read out results via its `Sim`).
    ///
    /// # Arguments
    ///
    /// * `id` - The region to access.
    #[must_use]
    pub fn region(&self, id: RegionId) -> &Region {
        &self.regions[id.index()]
    }

    /// Runs all regions to `end`, advancing one quantum at a time.
    ///
    /// Per quantum: (1) **PARALLEL** — every region runs its delta/timed loop to the
    /// boundary; (2) **EXCHANGE** (sequential, deterministic) — drain all outboxes, sort
    /// by the canonical key, route to per-region inboxes; (3) **COMMIT** — each region
    /// injects its inbox. Terminates when the boundary reaches `end`.
    ///
    /// # Arguments
    ///
    /// * `end` - The time to stop at.
    pub fn run(&mut self, end: SimTime) {
        let mut q_index: u64 = 0;
        loop {
            let boundary = global_quantum_boundary(q_index, self.quantum).min(end);

            // 1. PARALLEL: each region runs to the boundary.
            self.run_regions_to(boundary);

            // 2. EXCHANGE: drain outboxes, sort canonically, route to inboxes.
            for inbox in &mut self.inboxes {
                inbox.clear();
            }
            let mut routed: Vec<BoundaryMessage> = Vec::new();
            for r in &self.regions {
                routed.append(&mut r.drain_outbox());
            }
            routed.sort_unstable_by_key(BoundaryMessage::sort_key);
            for m in routed {
                self.inboxes[m.dst_region.index()].push(m);
            }

            // 3. COMMIT: each region injects its inbox.
            self.commit_regions();

            if boundary >= end {
                break;
            }
            q_index = q_index.saturating_add(1);
        }
    }

    /// PARALLEL phase dispatch: sequential by default, `par_iter_mut` under `rayon`.
    fn run_regions_to(&mut self, boundary: SimTime) {
        #[cfg(not(feature = "rayon"))]
        for r in &self.regions {
            r.run_to_boundary(boundary);
        }
        #[cfg(feature = "rayon")]
        {
            use rayon::prelude::*;
            self.regions
                .par_iter_mut()
                .for_each(|r| r.run_to_boundary(boundary));
        }
    }

    /// COMMIT phase dispatch: sequential by default, `par_iter_mut` under `rayon`.
    fn commit_regions(&mut self) {
        #[cfg(not(feature = "rayon"))]
        for (r, inbox) in self.regions.iter().zip(self.inboxes.iter_mut()) {
            r.commit(std::mem::take(inbox));
        }
        #[cfg(feature = "rayon")]
        {
            use rayon::prelude::*;
            self.regions
                .par_iter_mut()
                .zip(self.inboxes.par_iter_mut())
                .for_each(|(r, inbox)| r.commit(std::mem::take(inbox)));
        }
    }
}
