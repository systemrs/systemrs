//! Transport interfaces, the protocol traits, and DMI.

use systemrs_kernel::Ctx;
use systemrs_time::SimTime;

use crate::gp::GenericPayload;
use crate::phase::{Phase, TlmSync};

/// DMI access rights: plain booleans (`doc/systemrs-design.md` §6d SIMPLIFY — no
/// `bitflags` dependency).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DmiAccess {
    /// Whether reads are granted.
    pub read: bool,

    /// Whether writes are granted.
    pub write: bool,
}

/// Direct-memory-interface access rights and window for a region.
///
/// The design models the DMI backdoor as an arena handle/slice with a re-entrancy
/// guard (`doc/systemrs-design.md` §6d). This SIMPLIFY descriptor carries the access
/// flags, the granted address window, and the modelled per-direction latencies; the
/// re-entrancy guard lives in the socket registry.
#[derive(Debug, Clone, Default)]
pub struct Dmi {
    /// Whether reads are granted.
    pub read_allowed: bool,

    /// Whether writes are granted.
    pub write_allowed: bool,

    /// The inclusive start address of the granted region.
    pub start_address: u64,

    /// The inclusive end address of the granted region.
    pub end_address: u64,

    /// The modelled read latency for the region.
    pub read_latency: SimTime,

    /// The modelled write latency for the region.
    pub write_latency: SimTime,
}

impl Dmi {
    /// Returns the access rights as a [`DmiAccess`].
    pub fn access(&self) -> DmiAccess {
        DmiAccess {
            read: self.read_allowed,
            write: self.write_allowed,
        }
    }

    /// Sets the access rights from a [`DmiAccess`].
    ///
    /// # Arguments
    ///
    /// * `access` - The read/write grant.
    pub fn set_access(&mut self, access: DmiAccess) {
        self.read_allowed = access.read;
        self.write_allowed = access.write;
    }
}

/// A protocol traits struct: the payload and phase types a socket carries.
///
/// Mirrors SystemC's `TYPES` template parameter (`doc/systemrs-design.md` §6d).
pub trait Protocol: 'static {
    /// The transaction payload type.
    type Payload;

    /// The phase type.
    type Phase: Copy + PartialEq;
}

/// The TLM-2.0 base protocol: [`GenericPayload`] over the base [`Phase`] set.
#[derive(Debug, Clone, Copy)]
pub struct BaseProtocol;

impl Protocol for BaseProtocol {
    type Payload = GenericPayload;
    type Phase = Phase;
}

/// The backward (target → initiator) binding tag for a socket's response path.
///
/// A compile-time tag distinguishing the crossed backward `Port`/`Export` from the
/// forward [`BaseProtocol`] bind, so both resolve independently through the channel
/// binding registry (`doc/systemrs-design.md` §6d).
#[derive(Debug, Clone, Copy)]
pub struct BwBaseProtocol;

/// The forward transport interface (initiator → target).
///
/// `b_transport` is the only blocking method; it may yield the calling coroutine
/// via `ctx.wait`. `transport_dbg` takes no [`Ctx`], structurally forbidding waits
/// and notifications so it is callable off-scheduler (`doc/systemrs-design.md` §6d).
pub trait FwTransport {
    /// Blocking transport with timing annotation. May call `ctx.wait`.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle (the call may yield the calling thread).
    /// * `txn` - The transaction payload, aliased and mutated in place.
    /// * `delay` - The timing annotation; the callee may increase it.
    fn b_transport(&mut self, ctx: &Ctx, txn: &mut GenericPayload, delay: &mut SimTime);

    /// Non-blocking forward transport; advances the four-phase FSM.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle.
    /// * `txn` - The transaction payload.
    /// * `phase` - The current phase.
    /// * `delay` - The timing annotation.
    ///
    /// # Returns
    ///
    /// The [`TlmSync`] outcome.
    fn nb_transport_fw(
        &mut self,
        ctx: &Ctx,
        txn: &mut GenericPayload,
        phase: Phase,
        delay: &mut SimTime,
    ) -> TlmSync {
        let _ = (ctx, txn, phase, delay);
        TlmSync::Accepted
    }

    /// Side-effect-free, wait-free debug access (no [`Ctx`]).
    ///
    /// # Arguments
    ///
    /// * `txn` - The transaction payload to service.
    ///
    /// # Returns
    ///
    /// The number of bytes serviced.
    fn transport_dbg(&mut self, txn: &mut GenericPayload) -> u32 {
        let _ = txn;
        0
    }

    /// Requests a DMI grant for the transaction's region.
    ///
    /// # Arguments
    ///
    /// * `txn` - The transaction describing the region.
    /// * `dmi` - The DMI descriptor to populate.
    ///
    /// # Returns
    ///
    /// `true` if DMI is granted.
    fn get_direct_mem_ptr(&mut self, txn: &GenericPayload, dmi: &mut Dmi) -> bool {
        let _ = (txn, dmi);
        false
    }
}

/// The backward transport interface (target → initiator).
pub trait BwTransport {
    /// Non-blocking backward transport; advances the four-phase FSM.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle.
    /// * `txn` - The transaction payload.
    /// * `phase` - The current phase.
    /// * `delay` - The timing annotation.
    ///
    /// # Returns
    ///
    /// The [`TlmSync`] outcome.
    fn nb_transport_bw(
        &mut self,
        ctx: &Ctx,
        txn: &mut GenericPayload,
        phase: Phase,
        delay: &mut SimTime,
    ) -> TlmSync {
        let _ = (ctx, txn, phase, delay);
        TlmSync::Accepted
    }

    /// Invalidates a previously-granted DMI region.
    ///
    /// HARD RULE: must not call `get_direct_mem_ptr` (the re-entrancy ban,
    /// `doc/systemrs-design.md` §3.9).
    ///
    /// # Arguments
    ///
    /// * `start` - The inclusive start address.
    /// * `end` - The inclusive end address.
    fn invalidate_direct_mem_ptr(&mut self, start: u64, end: u64) {
        let _ = (start, end);
    }
}
