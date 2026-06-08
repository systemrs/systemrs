//! A simple byte-addressable memory target.

use std::cell::RefCell;
use std::rc::Rc;

use systemrs_kernel::Sim;
use systemrs_time::SimTime;

use crate::gp::{ByteEnable, Command, ResponseStatus};
use crate::socket::TargetSocket;

/// A byte-addressable memory modelled as a `b_transport` target.
///
/// The backing storage is shared (`Rc<RefCell<Vec<u8>>>`) so a testbench can load
/// a program and inspect results around the simulation, while the registered
/// `b_transport` callback services reads/writes. Access latency is modelled by
/// calling `ctx.wait` *inside* `b_transport` — a direct demonstration of the
/// design's stackful-coroutine property (`doc/systemrs-design.md` §6a).
///
/// # Examples
///
/// An initiator writes a word to memory and reads it back over a bound socket:
///
/// ```
/// use systemrs_tlm2::{GenericPayload, InitiatorSocket, Memory, TargetSocket};
/// use systemrs_kernel::Sim;
/// use systemrs_time::SimTime;
///
/// let sim = Sim::new();
/// let mem = Memory::new(256, SimTime::from_ns(5)); // 256 bytes, 5 ns/access
/// let target = TargetSocket::new(&sim, "mem");
/// mem.connect(&sim, &target);
///
/// let isock = InitiatorSocket::new(&sim, "cpu");
/// isock.bind(&sim, &target);
///
/// sim.add_thread("cpu", &[], true, move |cx| {
///     let mut delay = SimTime::ZERO;
///     let mut wr = GenericPayload::write(0x40, 0xCAFEu32.to_le_bytes().to_vec());
///     isock.b_transport(cx, &mut wr, &mut delay); // waits 5 ns inside
///     let mut rd = GenericPayload::read(0x40, 4);
///     isock.b_transport(cx, &mut rd, &mut delay);
///     assert_eq!(u32::from_le_bytes(rd.data().try_into().unwrap()), 0xCAFE);
/// });
/// sim.run_until(SimTime::from_ns(100));
/// assert_eq!(mem.read_u32(0x40), 0xCAFE); // backdoor read, no modelled latency
/// ```
#[derive(Clone)]
pub struct Memory {
    /// The shared backing bytes.
    storage: Rc<RefCell<Vec<u8>>>,

    /// The per-access latency modelled via `wait`.
    latency: SimTime,
}

impl Memory {
    /// Creates a zero-initialized memory of `size` bytes with `latency` per access.
    ///
    /// # Arguments
    ///
    /// * `size` - The memory size in bytes.
    /// * `latency` - The latency modelled on each access (may be `SimTime::ZERO`).
    ///
    /// # Returns
    ///
    /// A new memory (not yet connected to a socket; see [`Memory::connect`]).
    pub fn new(size: usize, latency: SimTime) -> Self {
        Memory {
            storage: Rc::new(RefCell::new(vec![0u8; size])),
            latency,
        }
    }

    /// Returns the memory size in bytes.
    pub fn size(&self) -> usize {
        self.storage.borrow().len()
    }

    /// Loads `bytes` into memory starting at `addr` (elaboration-time, no latency).
    ///
    /// # Arguments
    ///
    /// * `addr` - The destination byte offset.
    /// * `bytes` - The bytes to copy in.
    ///
    /// # Panics
    ///
    /// Panics if `addr + bytes.len()` exceeds the memory size.
    pub fn load(&self, addr: usize, bytes: &[u8]) {
        self.storage.borrow_mut()[addr..addr + bytes.len()].copy_from_slice(bytes);
    }

    /// Reads a single byte (backdoor, no latency).
    ///
    /// # Arguments
    ///
    /// * `addr` - The byte offset.
    ///
    /// # Returns
    ///
    /// The byte at `addr`.
    pub fn read_byte(&self, addr: usize) -> u8 {
        self.storage.borrow()[addr]
    }

    /// Reads a little-endian `u32` (backdoor, no latency).
    ///
    /// # Arguments
    ///
    /// * `addr` - The word's byte offset.
    ///
    /// # Returns
    ///
    /// The 32-bit little-endian word at `addr`.
    pub fn read_u32(&self, addr: usize) -> u32 {
        let m = self.storage.borrow();
        u32::from_le_bytes([m[addr], m[addr + 1], m[addr + 2], m[addr + 3]])
    }

    /// Connects this memory as the `b_transport` (and debug) target of `target`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `target` - The target socket to service.
    pub fn connect(&self, sim: &Sim, target: &TargetSocket) {
        let storage = Rc::clone(&self.storage);
        let latency = self.latency;
        target.register_b_transport(sim, move |ctx, txn, _delay| {
            // Model access time by waiting *inside* b_transport (wait-from-depth).
            if !latency.is_zero() {
                ctx.wait(latency);
            }
            let addr = txn.address() as usize;
            let len = txn.len();
            let mut mem = storage.borrow_mut();

            if addr.checked_add(len).is_none_or(|end| end > mem.len()) {
                txn.set_response_status(ResponseStatus::AddressError);
                return;
            }

            match txn.command() {
                Command::Read => {
                    // Byte-enables constrain reads too (IEEE-1666): disabled bytes
                    // are left untouched in the initiator's buffer.
                    if matches!(txn.byte_enable(), ByteEnable::All) {
                        txn.data_mut().copy_from_slice(&mem[addr..addr + len]);
                    } else {
                        let be = txn.byte_enable().clone();
                        let data = txn.data_mut();
                        for i in 0..len {
                            if be.enabled(i) {
                                data[i] = mem[addr + i];
                            }
                        }
                    }
                    txn.set_response_status(ResponseStatus::Ok);
                }
                Command::Write => {
                    {
                        let data = txn.data();
                        let be = txn.byte_enable();
                        for i in 0..len {
                            if be.enabled(i) {
                                mem[addr + i] = data[i];
                            }
                        }
                    }
                    txn.set_response_status(ResponseStatus::Ok);
                }
                Command::Ignore => txn.set_response_status(ResponseStatus::Ok),
            }
        });

        // A backdoor (latency-free) peek/poke for inspection: it does not advance
        // simulation time, but it does read or write the backing store.
        let dbg_storage = Rc::clone(&self.storage);
        target.register_transport_dbg(sim, move |txn| {
            let addr = txn.address() as usize;
            let len = txn.len();
            let mut mem = dbg_storage.borrow_mut();
            if addr.checked_add(len).is_none_or(|end| end > mem.len()) {
                txn.set_response_status(ResponseStatus::AddressError);
                return 0;
            }
            match txn.command() {
                Command::Read => {
                    txn.data_mut().copy_from_slice(&mem[addr..addr + len]);
                    txn.set_response_status(ResponseStatus::Ok);
                    len as u32
                }
                Command::Write => {
                    mem[addr..addr + len].copy_from_slice(txn.data());
                    txn.set_response_status(ResponseStatus::Ok);
                    len as u32
                }
                Command::Ignore => {
                    txn.set_response_status(ResponseStatus::Ok);
                    0
                }
            }
        });
    }
}
