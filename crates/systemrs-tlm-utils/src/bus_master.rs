//! [`BusMaster`] — the CPU-initiator convenience seam over an [`InitiatorSocket`].
//!
//! A bus master (a CPU model, a DMA engine, or — later — an out-of-process emulator
//! bridge) issues memory-mapped reads and writes as TLM [`GenericPayload`] blocking
//! transactions. [`BusMaster`] wraps the raw socket with ergonomic
//! [`read32`](BusMaster::read32)/[`write32`](BusMaster::write32)-style helpers that
//! build the payload, run `b_transport`, and translate the [`ResponseStatus`] into a
//! [`BusFault`] the caller can take as a bus error — the same mapping an in-tree ISS
//! and an out-of-tree bridge both reuse unchanged.
//!
//! All multi-byte accessors are **little-endian** (RV32 is a little-endian ISA).

use systemrs_kernel::Ctx;
use systemrs_time::SimTime;
use systemrs_tlm2::{GenericPayload, InitiatorSocket, ResponseStatus};

/// A bus error surfaced to a master when a transaction does not complete `Ok`.
///
/// Maps the TLM [`ResponseStatus`] error space onto the fault a CPU would take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusFault {
    /// The address did not decode to any target (`ResponseStatus::AddressError`).
    AddressError,

    /// The target does not support the requested command (`CommandError`).
    CommandError,

    /// A byte-enable error (`ByteEnableError`).
    ByteEnableError,

    /// Any other non-`Ok` status (generic/burst error, or an unexpected `Incomplete`).
    Other(ResponseStatus),
}

impl BusFault {
    /// Maps a non-`Ok` [`ResponseStatus`] to the corresponding [`BusFault`].
    ///
    /// # Arguments
    ///
    /// * `status` - The transaction's final response status (must not be `Ok`).
    ///
    /// # Returns
    ///
    /// The fault classifying `status`.
    fn from_status(status: ResponseStatus) -> Self {
        match status {
            ResponseStatus::AddressError => BusFault::AddressError,
            ResponseStatus::CommandError => BusFault::CommandError,
            ResponseStatus::ByteEnableError => BusFault::ByteEnableError,
            other => BusFault::Other(other),
        }
    }
}

/// A CPU-style bus initiator wrapping an [`InitiatorSocket`].
///
/// A `Copy` handle (the underlying socket is `Copy`), so it can be captured by an
/// `SC_THREAD` body and used to drive transactions from any call depth.
///
/// # Examples
///
/// ```
/// use systemrs_tlm2::{InitiatorSocket, Memory, TargetSocket};
/// use systemrs_tlm_utils::BusMaster;
/// use systemrs_kernel::Sim;
/// use systemrs_time::SimTime;
///
/// let sim = Sim::new();
/// let mem = Memory::new(256, SimTime::ZERO);
/// let target = TargetSocket::new(&sim, "mem");
/// mem.connect(&sim, &target);
/// let isock = InitiatorSocket::new(&sim, "cpu");
/// isock.bind(&sim, &target);
///
/// sim.add_thread("cpu", &[], true, move |cx| {
///     let bus = BusMaster::new(isock);
///     bus.write32(cx, 0x40, 0xDEAD_BEEF).unwrap();
///     assert_eq!(bus.read32(cx, 0x40).unwrap(), 0xDEAD_BEEF);
///     assert!(bus.read32(cx, 0xF000).is_err()); // out of range → AddressError
/// });
/// sim.run_until(SimTime::from_ns(10));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct BusMaster {
    /// The wrapped forward initiator socket.
    socket: InitiatorSocket,
}

impl BusMaster {
    /// Wraps an [`InitiatorSocket`] as a bus master.
    ///
    /// # Arguments
    ///
    /// * `socket` - The (bound) forward initiator socket to drive.
    ///
    /// # Returns
    ///
    /// A `Copy` [`BusMaster`] handle.
    pub fn new(socket: InitiatorSocket) -> Self {
        BusMaster { socket }
    }

    /// Returns the wrapped initiator socket.
    pub fn socket(&self) -> InitiatorSocket {
        self.socket
    }

    /// Reads `len` bytes from `addr`, returning the data or a [`BusFault`].
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle (the transaction may `wait` for modelled latency).
    /// * `addr` - The byte address.
    /// * `len` - The number of bytes to read.
    ///
    /// # Returns
    ///
    /// The `len` bytes read on success.
    ///
    /// # Errors
    ///
    /// Returns a [`BusFault`] if the target reports any non-`Ok` status.
    pub fn read(&self, cx: &Ctx, addr: u64, len: usize) -> Result<Vec<u8>, BusFault> {
        let mut txn = GenericPayload::read(addr, len);
        let mut delay = SimTime::ZERO;
        self.socket.b_transport(cx, &mut txn, &mut delay);
        match txn.response_status() {
            ResponseStatus::Ok => Ok(txn.data().to_vec()),
            other => Err(BusFault::from_status(other)),
        }
    }

    /// Writes `bytes` to `addr`, returning `Ok(())` or a [`BusFault`].
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    /// * `addr` - The byte address.
    /// * `bytes` - The bytes to write (moved into the payload).
    ///
    /// # Errors
    ///
    /// Returns a [`BusFault`] if the target reports any non-`Ok` status.
    pub fn write(&self, cx: &Ctx, addr: u64, bytes: Vec<u8>) -> Result<(), BusFault> {
        let mut txn = GenericPayload::write(addr, bytes);
        let mut delay = SimTime::ZERO;
        self.socket.b_transport(cx, &mut txn, &mut delay);
        match txn.response_status() {
            ResponseStatus::Ok => Ok(()),
            other => Err(BusFault::from_status(other)),
        }
    }

    /// Reads a little-endian `u32` from `addr`.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    /// * `addr` - The word address.
    ///
    /// # Returns
    ///
    /// The 32-bit little-endian word.
    ///
    /// # Errors
    ///
    /// Returns a [`BusFault`] on a non-`Ok` status (or a malformed-length response).
    pub fn read32(&self, cx: &Ctx, addr: u64) -> Result<u32, BusFault> {
        let bytes = self.read(cx, addr, 4)?;
        let arr: [u8; 4] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| BusFault::Other(ResponseStatus::GenericError))?;
        Ok(u32::from_le_bytes(arr))
    }

    /// Reads a little-endian `u16` from `addr`.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    /// * `addr` - The halfword address.
    ///
    /// # Returns
    ///
    /// The 16-bit little-endian halfword.
    ///
    /// # Errors
    ///
    /// Returns a [`BusFault`] on a non-`Ok` status (or a malformed-length response).
    pub fn read16(&self, cx: &Ctx, addr: u64) -> Result<u16, BusFault> {
        let bytes = self.read(cx, addr, 2)?;
        let arr: [u8; 2] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| BusFault::Other(ResponseStatus::GenericError))?;
        Ok(u16::from_le_bytes(arr))
    }

    /// Reads a single byte from `addr`.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    /// * `addr` - The byte address.
    ///
    /// # Returns
    ///
    /// The byte at `addr`.
    ///
    /// # Errors
    ///
    /// Returns a [`BusFault`] on a non-`Ok` status (or a malformed-length response).
    pub fn read8(&self, cx: &Ctx, addr: u64) -> Result<u8, BusFault> {
        let bytes = self.read(cx, addr, 1)?;
        bytes
            .first()
            .copied()
            .ok_or(BusFault::Other(ResponseStatus::GenericError))
    }

    /// Writes a little-endian `u32` to `addr`.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    /// * `addr` - The word address.
    /// * `value` - The 32-bit value (stored little-endian).
    ///
    /// # Errors
    ///
    /// Returns a [`BusFault`] on a non-`Ok` status.
    pub fn write32(&self, cx: &Ctx, addr: u64, value: u32) -> Result<(), BusFault> {
        self.write(cx, addr, value.to_le_bytes().to_vec())
    }

    /// Writes a little-endian `u16` to `addr`.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    /// * `addr` - The halfword address.
    /// * `value` - The 16-bit value (stored little-endian).
    ///
    /// # Errors
    ///
    /// Returns a [`BusFault`] on a non-`Ok` status.
    pub fn write16(&self, cx: &Ctx, addr: u64, value: u16) -> Result<(), BusFault> {
        self.write(cx, addr, value.to_le_bytes().to_vec())
    }

    /// Writes a single byte to `addr`.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    /// * `addr` - The byte address.
    /// * `value` - The byte to write.
    ///
    /// # Errors
    ///
    /// Returns a [`BusFault`] on a non-`Ok` status.
    pub fn write8(&self, cx: &Ctx, addr: u64, value: u8) -> Result<(), BusFault> {
        self.write(cx, addr, vec![value])
    }
}

#[cfg(test)]
mod tests {
    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;
    use systemrs_tlm2::{InitiatorSocket, Memory, TargetSocket};

    use super::{BusFault, BusMaster};

    /// Word/halfword/byte round-trips through a memory target, and an out-of-range
    /// access surfaces as `BusFault::AddressError`.
    #[test]
    fn word_round_trip_and_fault() {
        let sim = Sim::new();
        let mem = Memory::new(256, SimTime::from_ns(2));
        let target = TargetSocket::new(&sim, "mem");
        mem.connect(&sim, &target);
        let isock = InitiatorSocket::new(&sim, "cpu");
        isock.bind(&sim, &target);

        sim.add_thread("cpu", &[], true, move |cx| {
            let bus = BusMaster::new(isock);
            bus.write32(cx, 0x10, 0x1234_5678).unwrap();
            assert_eq!(bus.read32(cx, 0x10).unwrap(), 0x1234_5678);
            assert_eq!(bus.read16(cx, 0x10).unwrap(), 0x5678);
            assert_eq!(bus.read8(cx, 0x10).unwrap(), 0x78);
            bus.write8(cx, 0x10, 0xFF).unwrap();
            assert_eq!(bus.read8(cx, 0x10).unwrap(), 0xFF);
            assert_eq!(bus.read32(cx, 0x1000), Err(BusFault::AddressError));
        });

        sim.run_until(SimTime::from_ns(100));
        assert_eq!(mem.read_byte(0x10), 0xFF);
    }
}
