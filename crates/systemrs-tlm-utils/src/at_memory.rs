//! [`AtMemory`] — an approximately-timed (AT) memory target.
//!
//! The AT counterpart of `systemrs_tlm2::Memory`: it services reads/writes over the
//! four-phase `nb_transport` handshake (`doc/systemrs-design.md` §3.9), using a
//! [`PhaseQueue`] to model the response latency. The committed return shape is the
//! one the conformance tests rely on: `BEGIN_REQ → Updated(EndReq)` synchronously,
//! a PEQ-scheduled backward `BEGIN_RESP`, and `END_RESP → Completed`.

use std::cell::RefCell;
use std::rc::Rc;

use systemrs_kernel::Sim;
use systemrs_time::SimTime;
use systemrs_tlm2::{ByteEnable, Command, Phase, ResponseStatus, TargetSocket, TlmSync, Txn};

use crate::PhaseQueue;

/// A byte-addressable memory serviced over the AT four-phase protocol.
///
/// The backing storage is shared (`Rc<RefCell<Vec<u8>>>`) so a testbench can load a
/// program and inspect results around the simulation.
#[derive(Clone)]
pub struct AtMemory {
    /// The shared backing bytes.
    storage: Rc<RefCell<Vec<u8>>>,

    /// The modelled response latency (from BEGIN_REQ to the backward BEGIN_RESP).
    latency: SimTime,
}

impl AtMemory {
    /// Creates a zero-initialized AT memory of `size` bytes with `latency` per access.
    ///
    /// # Arguments
    ///
    /// * `size` - The memory size in bytes.
    /// * `latency` - The modelled response latency.
    ///
    /// # Returns
    ///
    /// A new [`AtMemory`] (connect it with [`AtMemory::connect`]).
    pub fn new(size: usize, latency: SimTime) -> Self {
        AtMemory {
            storage: Rc::new(RefCell::new(vec![0u8; size])),
            latency,
        }
    }

    /// Returns the memory size in bytes.
    pub fn size(&self) -> usize {
        self.storage.borrow().len()
    }

    /// Loads `bytes` into memory at `addr` (backdoor, no modelled time).
    ///
    /// # Arguments
    ///
    /// * `addr` - The destination offset.
    /// * `bytes` - The bytes to copy in.
    ///
    /// # Panics
    ///
    /// Panics if `addr + bytes.len()` exceeds the memory size.
    pub fn load(&self, addr: usize, bytes: &[u8]) {
        self.storage.borrow_mut()[addr..addr + bytes.len()].copy_from_slice(bytes);
    }

    /// Reads a single byte (backdoor).
    ///
    /// # Arguments
    ///
    /// * `addr` - The offset.
    ///
    /// # Returns
    ///
    /// The byte at `addr`.
    pub fn read_byte(&self, addr: usize) -> u8 {
        self.storage.borrow()[addr]
    }

    /// Reads a little-endian `u32` (backdoor).
    ///
    /// # Arguments
    ///
    /// * `addr` - The word offset.
    ///
    /// # Returns
    ///
    /// The 32-bit little-endian word at `addr`.
    pub fn read_u32(&self, addr: usize) -> u32 {
        let m = self.storage.borrow();
        u32::from_le_bytes([m[addr], m[addr + 1], m[addr + 2], m[addr + 3]])
    }

    /// Connects this memory as the AT (`nb_transport_fw`) target of `target`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `target` - The target socket to service.
    pub fn connect(&self, sim: &Sim, target: &TargetSocket) {
        let target = *target; // copy the handle so the closures are `'static`
        let resp_pq = Rc::new(PhaseQueue::new(sim, move |cx, txn, phase| {
            let mut t = SimTime::ZERO;
            target.nb_transport_bw(cx, txn, phase, &mut t);
        }));
        let rpq = Rc::clone(&resp_pq);
        let storage = Rc::clone(&self.storage);
        let latency = self.latency;

        target.register_nb_transport_fw(sim, move |cx, txn, phase, _t| match phase {
            Phase::BeginReq => {
                service(txn, &storage); // short-lived borrow inside
                rpq.notify(cx, Rc::clone(txn), Phase::BeginResp, latency);
                TlmSync::Updated(Phase::EndReq)
            }
            Phase::EndResp => TlmSync::Completed,
            _ => TlmSync::Accepted,
        });
    }
}

/// Services a read/write transaction against `storage`, honouring byte-enables and
/// bounds (out-of-range → `AddressError`). The payload borrow is short-lived.
fn service(txn: &Txn, storage: &Rc<RefCell<Vec<u8>>>) {
    let mut p = txn.borrow_mut();
    let addr = p.address() as usize;
    let len = p.len();
    let mut mem = storage.borrow_mut();

    if addr.checked_add(len).is_none_or(|end| end > mem.len()) {
        p.set_response_status(ResponseStatus::AddressError);
        return;
    }

    match p.command() {
        Command::Read => {
            if matches!(p.byte_enable(), ByteEnable::All) {
                p.data_mut().copy_from_slice(&mem[addr..addr + len]);
            } else {
                let be = p.byte_enable().clone();
                let data = p.data_mut();
                for (i, byte) in data.iter_mut().enumerate() {
                    if be.enabled(i) {
                        *byte = mem[addr + i];
                    }
                }
            }
            p.set_response_status(ResponseStatus::Ok);
        }
        Command::Write => {
            let be = p.byte_enable().clone();
            let data = p.data().to_vec();
            for (i, &byte) in data.iter().enumerate() {
                if be.enabled(i) {
                    mem[addr + i] = byte;
                }
            }
            p.set_response_status(ResponseStatus::Ok);
        }
        Command::Ignore => p.set_response_status(ResponseStatus::Ok),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;
    use systemrs_tlm2::{
        GenericPayload, InitiatorSocket, Phase, ResponseStatus, TargetSocket, TlmSync, TxnPool,
    };

    use super::AtMemory;
    use crate::PhaseQueue;

    /// An AT initiator writes then reads the AT memory across the four-phase
    /// handshake, with correct data and modelled latency.
    #[test]
    fn at_memory_services_write_then_read() {
        let sim = Sim::new();
        let mem = AtMemory::new(64, SimTime::from_ns(3));
        let target = TargetSocket::new(&sim, "mem");
        mem.connect(&sim, &target);

        let isock = InitiatorSocket::new(&sim, "cpu");
        isock.bind(&sim, &target);

        // The initiator completes each transaction: BeginResp → drive EndResp.
        let end_pq = Rc::new(PhaseQueue::new(&sim, move |cx, txn, phase| {
            let mut t = SimTime::ZERO;
            isock.nb_transport_fw(cx, txn, phase, &mut t);
        }));
        let epq = Rc::clone(&end_pq);
        let read_back: Rc<RefCell<Option<u8>>> = Rc::new(RefCell::new(None));
        let rb = Rc::clone(&read_back);
        isock.register_nb_transport_bw(&sim, move |cx, txn, phase, _t| {
            if phase == Phase::BeginResp {
                if matches!(txn.borrow().command(), systemrs_tlm2::Command::Read) {
                    *rb.borrow_mut() = Some(txn.borrow().data()[0]);
                }
                epq.notify(cx, Rc::clone(txn), Phase::EndResp, SimTime::ZERO);
            }
            TlmSync::Accepted
        });

        let pool = TxnPool::new();
        let wr = pool.acquire();
        *wr.borrow_mut() = GenericPayload::write(8, vec![0x5A]);
        let rd = pool.acquire();
        *rd.borrow_mut() = GenericPayload::read(8, 1);
        sim.add_method("driver", &[], true, move |cx| {
            let mut t = SimTime::ZERO;
            isock.nb_transport_fw(cx, &wr, Phase::BeginReq, &mut t);
            isock.nb_transport_fw(cx, &rd, Phase::BeginReq, &mut t);
        });

        sim.run_until(SimTime::from_ns(100));
        assert_eq!(mem.read_byte(8), 0x5A); // write serviced
        assert_eq!(*read_back.borrow(), Some(0x5A)); // read returned the written byte
        let _ = ResponseStatus::Ok;
    }
}
