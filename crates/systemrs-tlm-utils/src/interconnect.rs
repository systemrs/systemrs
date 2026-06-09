//! [`Interconnect`] — an address-decoding bus router (a "lightweight AXI" fabric).
//!
//! The core ships only direct one-initiator-to-one-target socket binding; the
//! multi/passthrough sockets that fan a *target* out to many initiators are deferred
//! to M5. A CPU SoC needs the dual: a single master whose transactions are routed **by
//! address** to one of many targets (RAM, UART, timer, …). That 1→N decode needs none
//! of the deferred fan-out machinery — each downstream region is its own ordinary 1:1
//! [`InitiatorSocket`] — so it is buildable today.
//!
//! [`Interconnect`] presents one upstream [`TargetSocket`] (the master binds to it via
//! [`target`](Interconnect::target)); each [`map`](Interconnect::map) records a
//! `(base, size)` region backed by its own downstream initiator;
//! [`connect`](Interconnect::connect) sorts and validates the map (overlaps are a
//! FATAL at elaboration) and registers a single re-entrancy-safe decode closure.
//!
//! Decode forwards via `register_b_transport` (an `&self` closure), **not**
//! `set_fw_transport` (an `&mut self` trait object): a downstream `b_transport` may
//! `wait()`, during which a second master could legally re-enter the router — a
//! `&mut self` target would double-borrow, the closure does not.

use std::cell::{Cell, RefCell};

use systemrs_diag::report_fatal;
use systemrs_kernel::Sim;
use systemrs_tlm2::{InitiatorSocket, ResponseStatus, TargetSocket};

/// The diagnostics message-type tag for interconnect FATALs.
const DIAG: &str = "SYSTEMRS/TLM-UTILS/INTERCONNECT";

/// One mapped address region and the initiator that reaches its target.
#[derive(Debug, Clone, Copy)]
struct Region {
    /// The inclusive start address of the region.
    base: u64,

    /// The region size in bytes (`base ..= base + size - 1`).
    size: u64,

    /// Whether the forwarded address is rebased to the region start (relative
    /// addressing) or passed through unchanged (absolute addressing).
    relative: bool,

    /// The 1:1 initiator bound to this region's downstream target.
    sock: InitiatorSocket,
}

/// Decodes `addr` against the sorted, non-overlapping `regions`.
///
/// # Arguments
///
/// * `regions` - The regions, sorted ascending by `base` and non-overlapping.
/// * `addr` - The absolute address to decode.
///
/// # Returns
///
/// The matching [`Region`] (copied), or `None` if `addr` falls in no region.
fn decode(regions: &[Region], addr: u64) -> Option<Region> {
    let i = regions.partition_point(|r| r.base <= addr);
    if i == 0 {
        return None;
    }
    let r = regions[i - 1];
    (addr - r.base < r.size).then_some(r)
}

/// An address-decoding interconnect routing one master to many targets.
///
/// # Examples
///
/// Route a master to two memories at different bases (relative addressing):
///
/// ```
/// use systemrs_tlm2::{InitiatorSocket, Memory, TargetSocket};
/// use systemrs_tlm_utils::{BusMaster, Interconnect};
/// use systemrs_kernel::Sim;
/// use systemrs_time::SimTime;
///
/// let sim = Sim::new();
/// let ram = Memory::new(256, SimTime::ZERO);
/// let rom = Memory::new(256, SimTime::ZERO);
/// let ram_t = TargetSocket::new(&sim, "ram");
/// let rom_t = TargetSocket::new(&sim, "rom");
/// ram.connect(&sim, &ram_t);
/// rom.connect(&sim, &rom_t);
///
/// let bus = Interconnect::new(&sim, "bus");
/// bus.map(&sim, 0x0000, 0x1000, true, &rom_t);
/// bus.map(&sim, 0x1000, 0x1000, true, &ram_t);
/// bus.connect(&sim);
///
/// let cpu = InitiatorSocket::new(&sim, "cpu");
/// cpu.bind(&sim, &bus.target());
/// sim.add_thread("cpu", &[], true, move |cx| {
///     let m = BusMaster::new(cpu);
///     m.write32(cx, 0x1004, 0xABCD).unwrap(); // → ram offset 0x004
///     assert_eq!(m.read32(cx, 0x1004).unwrap(), 0xABCD);
///     assert!(m.read32(cx, 0x9000).is_err()); // unmapped → AddressError
/// });
/// sim.run_until(SimTime::from_ns(10));
/// assert_eq!(ram.read_u32(0x004), 0xABCD);
/// ```
pub struct Interconnect {
    /// The hierarchical base name used to name the upstream and downstream sockets.
    name: String,

    /// The upstream-facing target the master binds to.
    upstream: TargetSocket,

    /// The regions accumulated by [`map`](Interconnect::map) before
    /// [`connect`](Interconnect::connect).
    regions: RefCell<Vec<Region>>,

    /// Set once [`connect`](Interconnect::connect) has registered the decode closure.
    connected: Cell<bool>,
}

impl Interconnect {
    /// Creates an interconnect with an upstream target named `"{name}.s"`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - The hierarchical base name.
    ///
    /// # Returns
    ///
    /// A new, unmapped [`Interconnect`].
    pub fn new(sim: &Sim, name: &str) -> Self {
        Interconnect {
            name: name.to_string(),
            upstream: TargetSocket::new(sim, &format!("{name}.s")),
            regions: RefCell::new(Vec::new()),
            connected: Cell::new(false),
        }
    }

    /// Returns the upstream target socket the master binds to.
    pub fn target(&self) -> TargetSocket {
        self.upstream
    }

    /// Maps a `(base, size)` region to a downstream target.
    ///
    /// Creates an internal 1:1 initiator (named `"{name}.m{index}"`) bound to
    /// `downstream`. With `relative = true` the forwarded transaction's address is
    /// rebased to the region start (the target sees `0 ..= size - 1`); with
    /// `relative = false` the absolute address is passed through.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `base` - The region's inclusive start address.
    /// * `size` - The region size in bytes (must be non-zero; validated at connect).
    /// * `relative` - Whether to rebase the forwarded address to the region start.
    /// * `downstream` - The target servicing this region.
    ///
    /// # Panics
    ///
    /// Aborts (FATAL) if called after [`connect`](Interconnect::connect).
    pub fn map(&self, sim: &Sim, base: u64, size: u64, relative: bool, downstream: &TargetSocket) {
        if self.connected.get() {
            report_fatal(DIAG, "map() called after connect()");
        }
        let index = self.regions.borrow().len();
        let sock = InitiatorSocket::new(sim, &format!("{}.m{index}", self.name));
        sock.bind(sim, downstream);
        self.regions.borrow_mut().push(Region {
            base,
            size,
            relative,
            sock,
        });
    }

    /// Finalises the map and registers the address-decoding `b_transport` closure.
    ///
    /// Sorts the regions by base, validates them (each non-zero and non-wrapping, and
    /// the set non-overlapping), and installs one decode closure on the upstream
    /// target. The closure decodes the transaction address: on a hit it (optionally)
    /// rebases, forwards `b_transport` to the region's initiator, and **restores the
    /// original address on return**; on a miss it sets
    /// [`ResponseStatus::AddressError`].
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    ///
    /// # Panics
    ///
    /// Aborts (FATAL) if called more than once, or if any region is zero-sized, wraps
    /// the address space, or overlaps another region.
    pub fn connect(&self, sim: &Sim) {
        if self.connected.replace(true) {
            report_fatal(DIAG, "connect() called more than once");
        }

        let mut regions = self.regions.borrow().clone();
        regions.sort_by_key(|r| r.base);

        for r in &regions {
            if r.size == 0 {
                report_fatal(DIAG, &format!("zero-sized region at base {:#x}", r.base));
            }
            if r.base.checked_add(r.size).is_none() {
                report_fatal(
                    DIAG,
                    &format!("region at base {:#x} wraps the address space", r.base),
                );
            }
        }
        for pair in regions.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a.base + a.size > b.base {
                report_fatal(
                    DIAG,
                    &format!(
                        "overlapping regions: [{:#x}, {:#x}) and [{:#x}, {:#x})",
                        a.base,
                        a.base + a.size,
                        b.base,
                        b.base + b.size,
                    ),
                );
            }
        }

        self.upstream
            .register_b_transport(sim, move |cx, txn, delay| {
                let addr = txn.address();
                match decode(&regions, addr) {
                    Some(region) => {
                        let original = txn.address();
                        if region.relative {
                            txn.set_address(addr - region.base);
                        }
                        region.sock.b_transport(cx, txn, delay);
                        // Restore the master's absolute address on the return path.
                        txn.set_address(original);
                    }
                    None => txn.set_response_status(ResponseStatus::AddressError),
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;
    use systemrs_tlm2::{Command, GenericPayload, InitiatorSocket, Memory, TargetSocket};

    use super::Interconnect;
    use crate::{BusFault, BusMaster};

    /// Two memories at distinct bases route correctly; an unmapped address faults.
    #[test]
    fn routes_two_regions_and_faults_on_miss() {
        let sim = Sim::new();
        let rom = Memory::new(256, SimTime::ZERO);
        let ram = Memory::new(256, SimTime::ZERO);
        let rom_t = TargetSocket::new(&sim, "rom");
        let ram_t = TargetSocket::new(&sim, "ram");
        rom.connect(&sim, &rom_t);
        ram.connect(&sim, &ram_t);

        let bus = Interconnect::new(&sim, "bus");
        bus.map(&sim, 0x0000, 0x1000, true, &rom_t);
        bus.map(&sim, 0x1000, 0x1000, true, &ram_t);
        bus.connect(&sim);

        let cpu = InitiatorSocket::new(&sim, "cpu");
        cpu.bind(&sim, &bus.target());
        sim.add_thread("cpu", &[], true, move |cx| {
            let m = BusMaster::new(cpu);
            m.write32(cx, 0x0008, 0x1111_2222).unwrap(); // rom offset 0x008
            m.write32(cx, 0x1008, 0x3333_4444).unwrap(); // ram offset 0x008
            assert_eq!(m.read32(cx, 0x0008).unwrap(), 0x1111_2222);
            assert_eq!(m.read32(cx, 0x1008).unwrap(), 0x3333_4444);
            assert_eq!(m.read32(cx, 0x8000), Err(BusFault::AddressError));
        });

        sim.run_until(SimTime::from_ns(10));
        assert_eq!(rom.read_u32(0x008), 0x1111_2222);
        assert_eq!(ram.read_u32(0x008), 0x3333_4444);
    }

    /// Relative vs absolute addressing: the downstream target observes the rebased
    /// address when `relative`, and the absolute address otherwise; the master's
    /// payload address is restored either way.
    #[test]
    fn rebases_relative_and_restores_address() {
        let sim = Sim::new();
        let seen = Rc::new(Cell::new(u64::MAX));
        let recorder = TargetSocket::new(&sim, "rec");
        let seen_cb = Rc::clone(&seen);
        recorder.register_b_transport(&sim, move |_cx, txn, _d| {
            seen_cb.set(txn.address());
            if matches!(txn.command(), Command::Read) {
                for b in txn.data_mut() {
                    *b = 0;
                }
            }
            txn.set_response_status(systemrs_tlm2::ResponseStatus::Ok);
        });

        let bus = Interconnect::new(&sim, "bus");
        bus.map(&sim, 0x2000, 0x1000, true, &recorder); // relative
        bus.connect(&sim);

        let cpu = InitiatorSocket::new(&sim, "cpu");
        cpu.bind(&sim, &bus.target());
        sim.add_thread("cpu", &[], true, move |cx| {
            let mut txn = GenericPayload::read(0x2040, 4);
            let mut d = SimTime::ZERO;
            cpu.b_transport(cx, &mut txn, &mut d);
            assert_eq!(txn.address(), 0x2040); // restored for the master
        });
        sim.run_until(SimTime::from_ns(10));
        assert_eq!(seen.get(), 0x0040); // downstream observed the rebased address
    }

    /// Overlapping regions are rejected with a FATAL at `connect`.
    #[test]
    #[should_panic(expected = "overlapping regions")]
    fn overlap_is_fatal() {
        let sim = Sim::new();
        let a = TargetSocket::new(&sim, "a");
        let b = TargetSocket::new(&sim, "b");
        Memory::new(64, SimTime::ZERO).connect(&sim, &a);
        Memory::new(64, SimTime::ZERO).connect(&sim, &b);
        let bus = Interconnect::new(&sim, "bus");
        bus.map(&sim, 0x0000, 0x1000, true, &a);
        bus.map(&sim, 0x0800, 0x1000, true, &b); // overlaps [0x0,0x1000)
        bus.connect(&sim);
    }
}
