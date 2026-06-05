//! The TLM-2.0 generic payload and its scalar enums.
//!
//! See `doc/systemrs-design.md` §3.8, §6d. SystemC's signed-int conventions become
//! Rust sum types whose invariants are structural ([`Command`], [`ResponseStatus`]),
//! and — the decisive idiomatic change — the data buffer is **owned** (`Vec<u8>`)
//! rather than a borrowed `*unsigned char`, because in the AT flow the payload
//! outlives the call that created it. This is strictly safer and indistinguishable
//! from the model's point of view.

use crate::extension::ExtensionMap;

/// The transaction command (`tlm_command`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// A read: the target fills the payload's data buffer.
    Read,

    /// A write: the target consumes the payload's data buffer.
    Write,

    /// Neither read nor write (used for e.g. DMI-only or extension-only requests).
    Ignore,
}

/// The transaction response status (`tlm_response_status`, `tlm_gp.h:96-103`).
///
/// `Ok` (= 1) is the sole success value; `Incomplete` (= 0) is the initial,
/// not-yet-processed state and is **not** an error; the five error values are
/// strictly negative. [`ResponseStatus::is_error`] is therefore exactly
/// "discriminant < 0" (`doc/systemrs-design.md` §3.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ResponseStatus {
    /// Initial, not-yet-processed (0) — not an error.
    Incomplete = 0,

    /// The sole OK value (1).
    Ok = 1,

    /// A generic error (-1).
    GenericError = -1,

    /// The address was invalid (-2).
    AddressError = -2,

    /// The command was unsupported (-3).
    CommandError = -3,

    /// A burst error (-4).
    BurstError = -4,

    /// A byte-enable error (-5).
    ByteEnableError = -5,
}

impl ResponseStatus {
    /// Returns the signed discriminant value (matching SystemC's integer encoding).
    pub fn discriminant(self) -> i32 {
        self as i32
    }

    /// Returns `true` if this is the sole OK value.
    pub fn is_ok(self) -> bool {
        matches!(self, ResponseStatus::Ok)
    }

    /// Returns `true` if this is an error (discriminant < 0), excluding both `Ok`
    /// and `Incomplete`.
    pub fn is_error(self) -> bool {
        self.discriminant() < 0
    }
}

/// A byte-enable mask: either all bytes enabled, or a repeating `0xff`/`0x00`
/// pattern (`doc/systemrs-design.md` §6d). The pattern repeats modulo its length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ByteEnable {
    /// All bytes are enabled.
    All,

    /// A repeating mask; byte `i` is enabled iff `mask[i % mask.len()] != 0`.
    Mask(Vec<u8>),
}

impl ByteEnable {
    /// Returns `true` if byte index `i` is enabled.
    ///
    /// # Arguments
    ///
    /// * `i` - The byte index within the data buffer.
    pub fn enabled(&self, i: usize) -> bool {
        match self {
            ByteEnable::All => true,
            ByteEnable::Mask(mask) => !mask.is_empty() && mask[i % mask.len()] != 0x00,
        }
    }
}

/// The TLM-2.0 generic payload: the universal transaction object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericPayload {
    /// The command.
    command: Command,

    /// The transaction address.
    address: u64,

    /// The owned data buffer (read target fills it; write target consumes it).
    data: Vec<u8>,

    /// The streaming width (defaults to the data length when zero).
    streaming_width: u32,

    /// The byte-enable mask.
    byte_enable: ByteEnable,

    /// The response status.
    response_status: ResponseStatus,

    /// Whether the target permits DMI for this region.
    dmi_allowed: bool,

    /// Type-keyed extensions.
    extensions: ExtensionMap,
}

impl GenericPayload {
    /// Creates an empty `Ignore` payload with an empty data buffer.
    pub fn new() -> Self {
        GenericPayload {
            command: Command::Ignore,
            address: 0,
            data: Vec::new(),
            streaming_width: 0,
            byte_enable: ByteEnable::All,
            response_status: ResponseStatus::Incomplete,
            dmi_allowed: false,
            extensions: ExtensionMap::default(),
        }
    }

    /// Builds a read transaction for `len` bytes at `address` (data zero-filled).
    ///
    /// # Arguments
    ///
    /// * `address` - The start address.
    /// * `len` - The number of bytes to read.
    ///
    /// # Returns
    ///
    /// A payload with `Command::Read`, an `len`-byte buffer, and `Incomplete` status.
    pub fn read(address: u64, len: usize) -> Self {
        let mut gp = GenericPayload::new();
        gp.command = Command::Read;
        gp.address = address;
        gp.data = vec![0u8; len];
        gp
    }

    /// Builds a write transaction carrying `bytes` at `address`.
    ///
    /// # Arguments
    ///
    /// * `address` - The start address.
    /// * `bytes` - The data to write (moved into the payload).
    ///
    /// # Returns
    ///
    /// A payload with `Command::Write`, the given buffer, and `Incomplete` status.
    pub fn write(address: u64, bytes: Vec<u8>) -> Self {
        let mut gp = GenericPayload::new();
        gp.command = Command::Write;
        gp.address = address;
        gp.data = bytes;
        gp
    }

    /// Fully resets the payload to a fresh `Ignore` state.
    ///
    /// Unlike SystemC's `reset()` (which leaves scalar fields stale), this performs
    /// a full reset — reusing stale fields is a real source of SystemC bugs
    /// (`doc/systemrs-design.md` §6d).
    pub fn reset(&mut self) {
        self.command = Command::Ignore;
        self.address = 0;
        self.data.clear();
        self.streaming_width = 0;
        self.byte_enable = ByteEnable::All;
        self.response_status = ResponseStatus::Incomplete;
        self.dmi_allowed = false;
        self.extensions.clear();
    }

    /// Returns the command.
    pub fn command(&self) -> Command {
        self.command
    }

    /// Sets the command.
    pub fn set_command(&mut self, command: Command) {
        self.command = command;
    }

    /// Returns the address.
    pub fn address(&self) -> u64 {
        self.address
    }

    /// Sets the address.
    pub fn set_address(&mut self, address: u64) {
        self.address = address;
    }

    /// Returns the data length in bytes.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if the data buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns the data buffer.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns the data buffer mutably (for a target servicing a read).
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Returns the byte-enable mask.
    pub fn byte_enable(&self) -> &ByteEnable {
        &self.byte_enable
    }

    /// Sets the byte-enable mask.
    pub fn set_byte_enable(&mut self, byte_enable: ByteEnable) {
        self.byte_enable = byte_enable;
    }

    /// Returns the response status.
    pub fn response_status(&self) -> ResponseStatus {
        self.response_status
    }

    /// Sets the response status.
    pub fn set_response_status(&mut self, status: ResponseStatus) {
        self.response_status = status;
    }

    /// Returns `true` if the response is `Ok`.
    pub fn is_response_ok(&self) -> bool {
        self.response_status.is_ok()
    }

    /// Returns `true` if the response is an error.
    pub fn is_response_error(&self) -> bool {
        self.response_status.is_error()
    }

    /// Returns whether DMI is allowed for this transaction.
    pub fn is_dmi_allowed(&self) -> bool {
        self.dmi_allowed
    }

    /// Sets the DMI-allowed hint.
    pub fn set_dmi_allowed(&mut self, allowed: bool) {
        self.dmi_allowed = allowed;
    }

    /// Returns the extension map.
    pub fn extensions(&self) -> &ExtensionMap {
        &self.extensions
    }

    /// Returns the extension map mutably.
    pub fn extensions_mut(&mut self) -> &mut ExtensionMap {
        &mut self.extensions
    }
}

impl Default for GenericPayload {
    fn default() -> Self {
        GenericPayload::new()
    }
}
