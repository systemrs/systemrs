//! A minimal RV32IM system-on-chip: hart + address-decoded fabric + peripherals.
//!
//! This wires the [`crate::rv32i`] machine-mode hart to an [`Interconnect`] that routes
//! the CPU's memory-mapped accesses by address to a RAM, a [`Uart`], and a CLINT-like
//! [`Clint`] timer. The timer asserts a level-sensitive [`IrqLine`] that the hart
//! samples into `mip.MTIP`; a small bare-metal firmware enables the timer interrupt,
//! prints to the UART, idles in `wfi`, and counts timer interrupts in RAM.
//!
//! Everything is pure Rust and fully deterministic: the same wiring run twice produces
//! byte-identical output (see `tests/rv32i_soc.rs`), so it doubles as the golden
//! reference an out-of-process QEMU bridge would be validated against.
//!
//! Register maps are simplified (small offsets, 32-bit registers) so the hand-assembled
//! firmware uses plain 12-bit load/store offsets; they are *CLINT-like*, not the
//! architectural CLINT offsets.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use systemrs::prelude::*;

use crate::rv32i::{self, HartIrqs, HartState, asm, build_soc_hart};

/// The RAM base address (the reset vector; firmware is loaded here).
pub const RAM_BASE: u32 = 0x8000_0000;
/// The RAM size in bytes (64 KiB).
pub const RAM_SIZE: u32 = 0x0001_0000;
/// The UART base address.
pub const UART_BASE: u32 = 0x1000_0000;
/// The CLINT (timer) base address.
pub const CLINT_BASE: u32 = 0x0200_0000;
/// The RAM offset at which the firmware's timer-interrupt counter lives.
pub const COUNTER_OFF: u32 = 0x0400;

/// UART register: transmit-data (write a byte to emit it).
const UART_TXDATA: u64 = 0x0;
/// UART register: status (read; bit 0 = transmitter ready).
const UART_STATUS: u64 = 0x4;

/// CLINT register: current machine time (read-only, 32-bit low half here).
const CLINT_MTIME: u64 = 0x0;
/// CLINT register: machine software interrupt pending (bit 0 → software IRQ).
const CLINT_MSIP: u64 = 0x4;
/// CLINT register: machine timer compare (write to (re-)arm the timer interrupt).
const CLINT_MTIMECMP: u64 = 0x8;

/// Reads the low (up to 4) little-endian bytes of a payload as a `u32`.
fn read_word(gp: &GenericPayload) -> u32 {
    let mut value = 0u32;
    for (i, &byte) in gp.data().iter().enumerate().take(4) {
        value |= u32::from(byte) << (8 * i);
    }
    value
}

/// Writes `value` little-endian into a payload's data buffer (up to 4 bytes).
fn write_word(gp: &mut GenericPayload, value: u32) {
    let bytes = value.to_le_bytes();
    for (i, slot) in gp.data_mut().iter_mut().enumerate().take(4) {
        *slot = bytes[i];
    }
}

/// A minimal memory-mapped UART: writes to `TXDATA` append a byte to an observable
/// output buffer.
#[derive(Clone)]
pub struct Uart {
    /// The transmitted bytes (shared so a testbench can read the output).
    out: Rc<RefCell<Vec<u8>>>,

    /// The modelled per-access latency (realized via `wait` inside `b_transport`).
    latency: SimTime,
}

impl Uart {
    /// Creates a UART with the given per-access latency.
    ///
    /// # Arguments
    ///
    /// * `latency` - The modelled access latency (may be `SimTime::ZERO`).
    ///
    /// # Returns
    ///
    /// A new, unconnected [`Uart`].
    pub fn new(latency: SimTime) -> Self {
        Uart {
            out: Rc::new(RefCell::new(Vec::new())),
            latency,
        }
    }

    /// Returns the shared transmit buffer (the observable output).
    pub fn output(&self) -> Rc<RefCell<Vec<u8>>> {
        Rc::clone(&self.out)
    }

    /// Registers this UART as the `b_transport` target of `target`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `target` - The target socket to service.
    pub fn connect(&self, sim: &Sim, target: &TargetSocket) {
        let out = Rc::clone(&self.out);
        let latency = self.latency;
        target.register_b_transport(sim, move |cx, gp, _delay| {
            if !latency.is_zero() {
                cx.wait(latency);
            }
            match gp.command() {
                Command::Write => match gp.address() {
                    UART_TXDATA => {
                        if let Some(&byte) = gp.data().first() {
                            out.borrow_mut().push(byte);
                        }
                        gp.set_response_status(ResponseStatus::Ok);
                    }
                    _ => gp.set_response_status(ResponseStatus::AddressError),
                },
                Command::Read => match gp.address() {
                    UART_STATUS => {
                        write_word(gp, 1); // transmitter always ready
                        gp.set_response_status(ResponseStatus::Ok);
                    }
                    UART_TXDATA => {
                        write_word(gp, 0);
                        gp.set_response_status(ResponseStatus::Ok);
                    }
                    _ => gp.set_response_status(ResponseStatus::AddressError),
                },
                Command::Ignore => gp.set_response_status(ResponseStatus::Ok),
            }
        });
    }
}

/// Sets the timer IRQ line from the `mtime >= mtimecmp` comparison.
fn eval_timer(cx: &Ctx, mtime: &Cell<u64>, mtimecmp: &Cell<u64>, irq: IrqLine) {
    irq.set_level(cx, mtime.get() >= mtimecmp.get());
}

/// A CLINT-like core-local interruptor: a free-running timer plus a software-interrupt
/// register, each driving a level-sensitive [`IrqLine`].
///
/// `mtime` increments once per `tick` of modelled time (driven by a self-clocking
/// `SC_METHOD`). When `mtime >= mtimecmp` the timer line is asserted (`mip.MTIP`);
/// writing a fresh `mtimecmp` re-arms it. Writing bit 0 of `MSIP` drives the software
/// line (`mip.MSIP`). `mtimecmp` starts at `u64::MAX`, so the timer never fires until
/// firmware programs it.
pub struct Clint {
    /// The free-running machine-time counter (ticks elapsed).
    mtime: Rc<Cell<u64>>,

    /// The timer compare value; the timer fires when `mtime >= mtimecmp`.
    mtimecmp: Rc<Cell<u64>>,

    /// The machine-timer interrupt line (`mip.MTIP`).
    timer_irq: IrqLine,

    /// The machine-software interrupt line (`mip.MSIP`).
    soft_irq: IrqLine,

    /// The modelled time between `mtime` increments.
    tick: SimTime,
}

impl Clint {
    /// Creates a CLINT whose `mtime` advances once per `tick`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction (allocates the IRQ lines).
    /// * `tick` - The modelled time per `mtime` increment.
    ///
    /// # Returns
    ///
    /// A new, unconnected [`Clint`].
    pub fn new(sim: &Sim, tick: SimTime) -> Self {
        Clint {
            mtime: Rc::new(Cell::new(0)),
            mtimecmp: Rc::new(Cell::new(u64::MAX)),
            timer_irq: IrqLine::new(sim),
            soft_irq: IrqLine::new(sim),
            tick,
        }
    }

    /// Returns the machine-timer interrupt line (`mip.MTIP`).
    pub fn timer_irq(&self) -> IrqLine {
        self.timer_irq
    }

    /// Returns the machine-software interrupt line (`mip.MSIP`).
    pub fn software_irq(&self) -> IrqLine {
        self.soft_irq
    }

    /// Connects the CLINT registers as the `b_transport` target of `target` and starts
    /// the self-clocking `mtime` process.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `target` - The target socket to service.
    pub fn connect(&self, sim: &Sim, target: &TargetSocket) {
        // The self-clocking mtime tick: increment once per `tick` and re-evaluate the
        // timer line. `armed` skips the initial (t=0) invocation so mtime counts
        // elapsed ticks.
        let tick = self.tick;
        let tick_ev = sim.alloc_event();
        let mtime_tick = Rc::clone(&self.mtime);
        let mtimecmp_tick = Rc::clone(&self.mtimecmp);
        let timer_irq = self.timer_irq;
        let mut armed = false;
        sim.add_method("clint.tick", &[tick_ev], true, move |cx| {
            if armed {
                mtime_tick.set(mtime_tick.get() + 1);
                eval_timer(cx, &mtime_tick, &mtimecmp_tick, timer_irq);
            }
            armed = true;
            cx.notify_after(tick_ev, tick);
        });

        // The register interface.
        let mtime = Rc::clone(&self.mtime);
        let mtimecmp = Rc::clone(&self.mtimecmp);
        let soft_irq = self.soft_irq;
        target.register_b_transport(sim, move |cx, gp, _delay| match gp.command() {
            Command::Read => {
                let value = match gp.address() {
                    CLINT_MTIME => mtime.get() as u32,
                    CLINT_MSIP => u32::from(soft_irq.pending(cx)),
                    CLINT_MTIMECMP => mtimecmp.get() as u32,
                    _ => {
                        gp.set_response_status(ResponseStatus::AddressError);
                        return;
                    }
                };
                write_word(gp, value);
                gp.set_response_status(ResponseStatus::Ok);
            }
            Command::Write => {
                let value = read_word(gp);
                match gp.address() {
                    CLINT_MTIMECMP => {
                        mtimecmp.set(u64::from(value));
                        eval_timer(cx, &mtime, &mtimecmp, timer_irq);
                    }
                    CLINT_MSIP => soft_irq.set_level(cx, value & 1 != 0),
                    CLINT_MTIME => {} // read-only
                    _ => {
                        gp.set_response_status(ResponseStatus::AddressError);
                        return;
                    }
                }
                gp.set_response_status(ResponseStatus::Ok);
            }
            Command::Ignore => gp.set_response_status(ResponseStatus::Ok),
        });
    }
}

/// Emits `lui`/`addi` loading the 32-bit `addr` into `rd` (with the standard
/// sign-corrected hi/lo split).
fn li_addr(rd: u32, addr: u32) -> [u32; 2] {
    let hi = addr.wrapping_add(0x800) & 0xffff_f000;
    let lo = addr.wrapping_sub(hi) as i32; // in [-2048, 2047]
    [asm::lui(rd, hi), asm::addi(rd, rd, lo)]
}

/// The number of timer ticks between successive timer interrupts.
const TIMER_DELTA: i32 = 20;

/// Builds the demo firmware: install a trap handler, enable + arm the machine timer,
/// print `"Hi\n"` to the UART, then idle in `wfi`; each timer interrupt re-arms the
/// timer and increments a counter word in RAM at [`COUNTER_OFF`].
///
/// The instruction layout is fixed (every address load is exactly two instructions),
/// so the trap-vector address is the constant `RAM_BASE + 22 * 4`.
///
/// # Returns
///
/// The assembled little-endian machine code, to be loaded at [`RAM_BASE`].
pub fn firmware_demo() -> Vec<u8> {
    use asm::{addi, csrrs, csrrw, jal, lw, sw, wfi};

    // The trap handler begins after the 22-instruction main routine (each li_addr is
    // exactly two instructions, so this offset is stable).
    let mtvec = RAM_BASE + 22 * 4;

    let mut code: Vec<u32> = Vec::new();
    // --- main ---
    code.extend(li_addr(5, mtvec)); // x5 = trap handler
    code.push(csrrw(0, rv32i::CSR_MTVEC, 5)); // mtvec = x5
    code.extend(li_addr(6, CLINT_BASE)); // x6 = CLINT base
    code.push(lw(7, CLINT_MTIME as i32, 6)); // x7 = mtime
    code.push(addi(7, 7, TIMER_DELTA)); // x7 += DELTA
    code.push(sw(7, CLINT_MTIMECMP as i32, 6)); // mtimecmp = x7
    code.push(addi(8, 0, rv32i::MIP_MTIP as i32)); // x8 = MTIE bit
    code.push(csrrs(0, rv32i::CSR_MIE, 8)); // mie |= MTIE
    code.push(addi(8, 0, rv32i::MSTATUS_MIE as i32)); // x8 = MIE bit
    code.push(csrrs(0, rv32i::CSR_MSTATUS, 8)); // mstatus |= MIE
    code.extend(li_addr(10, UART_BASE)); // x10 = UART base
    for byte in *b"Hi\n" {
        code.push(addi(11, 0, i32::from(byte))); // x11 = char
        code.push(sw(11, UART_TXDATA as i32, 10)); // UART TXDATA = char
    }
    // wait_loop: wfi; loop back to the wfi.
    code.push(wfi()); // index 20
    code.push(jal(0, -4)); // index 21: back to the wfi

    debug_assert_eq!(
        code.len(),
        22,
        "main routine length feeds the mtvec constant"
    );

    // --- trap handler (index 22) ---
    code.extend(li_addr(28, CLINT_BASE)); // x28 = CLINT base
    code.push(lw(29, CLINT_MTIME as i32, 28)); // x29 = mtime
    code.push(addi(29, 29, TIMER_DELTA)); // x29 += DELTA
    code.push(sw(29, CLINT_MTIMECMP as i32, 28)); // re-arm mtimecmp (clears MTIP)
    code.extend(li_addr(30, RAM_BASE)); // x30 = RAM base
    code.push(lw(31, COUNTER_OFF as i32, 30)); // x31 = counter
    code.push(addi(31, 31, 1)); // x31 += 1
    code.push(sw(31, COUNTER_OFF as i32, 30)); // counter = x31
    code.push(asm::mret()); // return

    rv32i::assemble(&code)
}

/// The handles a built [`build_soc`] exposes for inspection and stimulus.
pub struct Soc {
    /// The system RAM (firmware image + the interrupt counter; inspect via the
    /// backdoor `read_u32`).
    pub ram: Memory,

    /// The UART's transmitted bytes.
    pub uart_out: Rc<RefCell<Vec<u8>>>,

    /// The hart's architectural state (also registered as a `Sim` service).
    pub state: Rc<RefCell<HartState>>,
}

impl Soc {
    /// Reads the firmware's timer-interrupt counter (backdoor, no modelled time).
    pub fn interrupt_count(&self) -> u32 {
        self.ram.read_u32(COUNTER_OFF as usize)
    }
}

/// Builds the complete demo SoC into `sim`: RAM + UART + CLINT behind an
/// address-decoding [`Interconnect`], a machine-mode hart, and the demo firmware.
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
///
/// # Returns
///
/// The [`Soc`] handles for inspection.
pub fn build_soc(sim: &Sim) -> Soc {
    // Targets.
    let ram = Memory::new(RAM_SIZE as usize, SimTime::from_ns(1));
    let ram_t = TargetSocket::new(sim, "ram");
    ram.connect(sim, &ram_t);

    let uart = Uart::new(SimTime::from_ns(2));
    let uart_t = TargetSocket::new(sim, "uart");
    uart.connect(sim, &uart_t);
    let uart_out = uart.output();

    let clint = Clint::new(sim, SimTime::from_ns(100));
    let clint_t = TargetSocket::new(sim, "clint");
    clint.connect(sim, &clint_t);
    let irqs = HartIrqs {
        timer: Some(clint.timer_irq()),
        software: Some(clint.software_irq()),
        external: None,
    };

    // The address-decoding interconnect (relative addressing per region).
    let bus = Interconnect::new(sim, "bus");
    bus.map(sim, u64::from(RAM_BASE), u64::from(RAM_SIZE), true, &ram_t);
    bus.map(sim, u64::from(UART_BASE), 0x1000, true, &uart_t);
    bus.map(sim, u64::from(CLINT_BASE), 0x1000, true, &clint_t);
    bus.connect(sim);

    // Firmware + hart.
    ram.load(0, &firmware_demo());
    let isock = InitiatorSocket::new(sim, "hart.isock");
    isock.bind(sim, &bus.target());
    let state = Rc::new(RefCell::new(HartState::new(RAM_BASE)));
    build_soc_hart(sim, isock, Rc::clone(&state), irqs, SimTime::from_ns(1));

    Soc {
        ram,
        uart_out,
        state,
    }
}

#[cfg(test)]
mod tests {
    use systemrs::prelude::*;

    use super::{build_soc, firmware_demo};

    /// The demo SoC prints `"Hi\n"` and takes at least one timer interrupt.
    #[test]
    fn soc_runs_firmware_and_takes_timer_interrupts() {
        let sim = Sim::new();
        let soc = build_soc(&sim);
        sim.run_until(SimTime::from_us(20));

        assert_eq!(soc.uart_out.borrow().as_slice(), b"Hi\n");
        assert!(
            soc.interrupt_count() >= 1,
            "expected at least one timer interrupt, got {}",
            soc.interrupt_count()
        );
    }

    /// The firmware image is the expected fixed size (22 main + 11 handler words).
    #[test]
    fn firmware_image_layout() {
        assert_eq!(firmware_demo().len(), 33 * 4);
    }
}
