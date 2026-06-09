//! Example 2: an RV32IM CPU hart driving a TLM SoC fabric.
//!
//! An `SC_THREAD` runs a fetch-decode-execute loop over the RV32I base integer
//! instruction set **plus the M (multiply/divide) extension**. **Every** memory access
//! — instruction fetch, load, store — goes through `b_transport` over an initiator
//! socket, so `wait()` (the modelled access latency) is reached from deep inside the
//! transport call on the hart's coroutine stack. This is the design's central property
//! in action (`doc/systemrs-design.md` §6a, §6d).
//!
//! Two harts are provided:
//! - [`build_hart`] — a minimal hart that halts on any trap (the original sum-program
//!   demo; the hart binds directly to one memory).
//! - [`build_soc_hart`] — a machine-mode hart with a minimal CSR/trap unit and
//!   level-sensitive interrupt sampling, intended to drive an address-decoded SoC
//!   ([`Interconnect`] → RAM + peripherals) and take timer/software/external
//!   interrupts. Its [`HartState`] lives in an `Rc<RefCell<…>>` of plain columnar
//!   state (register file + PC + CSRs), so it is snapshot/restore-clean
//!   (`doc/systemrs-design.md` §6f) and `--verify-determinism`-trivial.
//!
//! The instruction semantics are decoupled from the kernel via the [`Bus`] trait, so
//! the ISA (including the M extension, CSR access, and trap entry) is unit-tested
//! directly against an in-memory bus (see the tests), while the simulation drives a
//! socket-backed bus.
//!
//! Scope: machine mode only. The privileged subset is the minimum to *take* an
//! interrupt — `mstatus` (MIE/MPIE), `mie`, `mip`, `mtvec`, `mepc`, `mcause`,
//! `mscratch`. Deferred: the A/F/D/C extensions, user/supervisor modes, PMP, paging,
//! interrupt delegation, and `misa`/`mhartid` quirks (unimplemented CSRs read as zero).

use std::cell::RefCell;
use std::rc::Rc;

use systemrs::prelude::*;

/// The number of integer registers (`x0`–`x31`).
pub const NUM_REGS: usize = 32;

/// `mstatus` CSR address.
pub const CSR_MSTATUS: u32 = 0x300;
/// `mie` (machine interrupt-enable) CSR address.
pub const CSR_MIE: u32 = 0x304;
/// `mtvec` (machine trap-vector base) CSR address.
pub const CSR_MTVEC: u32 = 0x305;
/// `mscratch` CSR address.
pub const CSR_MSCRATCH: u32 = 0x340;
/// `mepc` (machine exception PC) CSR address.
pub const CSR_MEPC: u32 = 0x341;
/// `mcause` (machine trap cause) CSR address.
pub const CSR_MCAUSE: u32 = 0x342;
/// `mip` (machine interrupt-pending) CSR address.
pub const CSR_MIP: u32 = 0x344;

/// `mstatus.MIE` — the global machine interrupt-enable bit.
pub const MSTATUS_MIE: u32 = 1 << 3;
/// `mstatus.MPIE` — the saved (pre-trap) interrupt-enable bit.
pub const MSTATUS_MPIE: u32 = 1 << 7;
/// `mstatus.MPP` — the saved privilege mode (always machine here).
pub const MSTATUS_MPP: u32 = 0b11 << 11;

/// `mie.MSIE` / `mip.MSIP` — machine software interrupt.
pub const MIP_MSIP: u32 = 1 << 3;
/// `mie.MTIE` / `mip.MTIP` — machine timer interrupt.
pub const MIP_MTIP: u32 = 1 << 7;
/// `mie.MEIE` / `mip.MEIP` — machine external interrupt.
pub const MIP_MEIP: u32 = 1 << 11;

/// A bus access fault surfaced to the ISA (maps to a load/store/fetch access-fault
/// trap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusError;

/// A byte-addressable bus the hart reads instructions and data through.
///
/// Abstracting memory access behind this trait lets the RV32IM core ([`step`]) be
/// exercised by both the socket-backed simulation bus and a plain in-memory test bus.
pub trait Bus {
    /// Reads `len` (1–4) little-endian bytes at `addr`, zero-extended to a `u32`.
    ///
    /// # Arguments
    ///
    /// * `addr` - The byte address.
    /// * `len` - The number of bytes (1, 2, or 4).
    ///
    /// # Returns
    ///
    /// The little-endian value, zero-extended.
    ///
    /// # Errors
    ///
    /// Returns [`BusError`] if the access does not complete successfully.
    fn read(&mut self, addr: u32, len: usize) -> Result<u32, BusError>;

    /// Writes the low `len` little-endian bytes of `value` at `addr`.
    ///
    /// # Arguments
    ///
    /// * `addr` - The byte address.
    /// * `value` - The value whose low `len` bytes are written.
    /// * `len` - The number of bytes (1, 2, or 4).
    ///
    /// # Errors
    ///
    /// Returns [`BusError`] if the access does not complete successfully.
    fn write(&mut self, addr: u32, value: u32, len: usize) -> Result<(), BusError>;
}

/// The outcome of executing one instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    /// Continue fetching the next instruction (the PC has been updated).
    Continue,

    /// The instruction raised a synchronous trap; the PC was *not* advanced.
    Trapped(Trap),

    /// A `WFI` was executed; the PC was advanced. The hart should idle until an
    /// interrupt becomes pending.
    WaitForInterrupt,
}

/// A machine-mode trap cause (synchronous exception or asynchronous interrupt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trap {
    /// An instruction fetch faulted.
    InstructionAccessFault,

    /// An illegal/unrecognized instruction.
    IllegalInstruction,

    /// An `EBREAK`.
    Breakpoint,

    /// A load access faulted.
    LoadAccessFault,

    /// A store access faulted.
    StoreAccessFault,

    /// An `ECALL` from machine mode.
    EcallFromM,

    /// A machine software interrupt (`mip.MSIP`).
    MachineSoftware,

    /// A machine timer interrupt (`mip.MTIP`).
    MachineTimer,

    /// A machine external interrupt (`mip.MEIP`).
    MachineExternal,
}

impl Trap {
    /// Returns the `mcause` value for this trap (interrupt bit 31 set for interrupts).
    pub fn cause(self) -> u32 {
        match self {
            Trap::InstructionAccessFault => 1,
            Trap::IllegalInstruction => 2,
            Trap::Breakpoint => 3,
            Trap::LoadAccessFault => 5,
            Trap::StoreAccessFault => 7,
            Trap::EcallFromM => 11,
            Trap::MachineSoftware => 0x8000_0003,
            Trap::MachineTimer => 0x8000_0007,
            Trap::MachineExternal => 0x8000_000b,
        }
    }

    /// Returns `true` if this is an asynchronous interrupt (vs a synchronous
    /// exception).
    pub fn is_interrupt(self) -> bool {
        matches!(
            self,
            Trap::MachineSoftware | Trap::MachineTimer | Trap::MachineExternal
        )
    }
}

/// The minimal machine-mode control/status register file.
///
/// Only the registers needed to take and return from an interrupt are modelled; all
/// are plain `u32` columnar state (snapshot-clean).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Csrs {
    /// `mstatus`: only MIE/MPIE/MPP are meaningful here.
    pub mstatus: u32,

    /// `mie`: per-source interrupt enables (MSIE/MTIE/MEIE).
    pub mie: u32,

    /// `mip`: per-source interrupt pending (driven by the IRQ lines).
    pub mip: u32,

    /// `mtvec`: the trap-vector base (direct mode; low two bits ignored).
    pub mtvec: u32,

    /// `mepc`: the PC saved on a trap.
    pub mepc: u32,

    /// `mcause`: the cause of the last trap.
    pub mcause: u32,

    /// `mscratch`: a scratch register for trap handlers.
    pub mscratch: u32,
}

/// The complete architectural state of one hart: register file, PC, and CSRs.
///
/// `Copy` (≈160 bytes) so the [`build_soc_hart`] loop can lift it out of its
/// `Rc<RefCell<…>>` home into stack locals for the duration of one instruction —
/// avoiding holding a `RefCell` borrow across the `wait()` inside a load/store, while
/// keeping the canonical state columnar and snapshottable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HartState {
    /// The 32 integer registers (`regs[0]` is hard-wired zero).
    pub regs: [u32; NUM_REGS],

    /// The program counter.
    pub pc: u32,

    /// The machine-mode CSR file.
    pub csr: Csrs,
}

impl HartState {
    /// Creates a reset hart state with all registers and CSRs zero and `pc = entry`.
    ///
    /// # Arguments
    ///
    /// * `entry` - The reset program-counter value.
    ///
    /// # Returns
    ///
    /// The reset [`HartState`].
    pub fn new(entry: u32) -> Self {
        HartState {
            regs: [0u32; NUM_REGS],
            pc: entry,
            csr: Csrs::default(),
        }
    }
}

/// Sign-extends the low `bits` bits of `value` to a 32-bit signed integer.
fn sign_extend(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

/// Writes `value` to register `rd`, honouring the hard-wired-zero `x0`.
fn set_reg(regs: &mut [u32; NUM_REGS], rd: usize, value: u32) {
    if rd != 0 {
        regs[rd] = value;
    }
}

/// Computes the high 32 bits of a signed×signed 64-bit product (`MULH`).
fn mulh(a: i32, b: i32) -> u32 {
    ((i64::from(a) * i64::from(b)) >> 32) as u32
}

/// Computes the high 32 bits of a signed×unsigned 64-bit product (`MULHSU`).
fn mulhsu(a: i32, b: u32) -> u32 {
    ((i64::from(a) * i64::from(b)) >> 32) as u32
}

/// Computes the high 32 bits of an unsigned×unsigned 64-bit product (`MULHU`).
fn mulhu(a: u32, b: u32) -> u32 {
    ((u64::from(a) * u64::from(b)) >> 32) as u32
}

/// Signed division with RISC-V edge cases (`DIV`): ÷0 → −1; `INT_MIN/−1` → `INT_MIN`.
fn div_s(a: i32, b: i32) -> u32 {
    if b == 0 {
        u32::MAX
    } else {
        a.wrapping_div(b) as u32
    }
}

/// Unsigned division with the RISC-V ÷0 case (`DIVU`): ÷0 → all-ones.
fn div_u(a: u32, b: u32) -> u32 {
    a.checked_div(b).unwrap_or(u32::MAX)
}

/// Signed remainder with RISC-V edge cases (`REM`): ÷0 → dividend; `INT_MIN/−1` → 0.
fn rem_s(a: i32, b: i32) -> u32 {
    if b == 0 {
        a as u32
    } else {
        a.wrapping_rem(b) as u32
    }
}

/// Unsigned remainder with the RISC-V ÷0 case (`REMU`): ÷0 → dividend.
fn rem_u(a: u32, b: u32) -> u32 {
    a.checked_rem(b).unwrap_or(a)
}

/// Reads a CSR by address (unimplemented CSRs read as zero).
fn csr_read(csr: &Csrs, addr: u16) -> u32 {
    match u32::from(addr) {
        CSR_MSTATUS => csr.mstatus,
        CSR_MIE => csr.mie,
        CSR_MTVEC => csr.mtvec,
        CSR_MSCRATCH => csr.mscratch,
        CSR_MEPC => csr.mepc,
        CSR_MCAUSE => csr.mcause,
        CSR_MIP => csr.mip,
        _ => 0,
    }
}

/// Writes a CSR by address. `mip` is hardware-driven (the IRQ lines); writes to it and
/// to unimplemented CSRs are ignored.
fn csr_write(csr: &mut Csrs, addr: u16, value: u32) {
    match u32::from(addr) {
        CSR_MSTATUS => csr.mstatus = value,
        CSR_MIE => csr.mie = value,
        CSR_MTVEC => csr.mtvec = value,
        CSR_MSCRATCH => csr.mscratch = value,
        CSR_MEPC => csr.mepc = value,
        CSR_MCAUSE => csr.mcause = value,
        _ => {}
    }
}

/// Executes a `Zicsr` CSR read-modify-write, writing the old value to `rd`.
fn csr_op(st: &mut HartState, funct3: u32, addr: u16, rs1_field: usize, rs1v: u32, rd: usize) {
    let old = csr_read(&st.csr, addr);
    let do_write = match funct3 {
        1 | 5 => true,                   // CSRRW / CSRRWI always write
        2 | 3 | 6 | 7 => rs1_field != 0, // set/clear with x0/uimm=0 do not write
        _ => false,
    };
    if do_write {
        let imm = rs1_field as u32;
        let new = match funct3 {
            1 => rs1v,        // CSRRW
            2 => old | rs1v,  // CSRRS
            3 => old & !rs1v, // CSRRC
            5 => imm,         // CSRRWI
            6 => old | imm,   // CSRRSI
            7 => old & !imm,  // CSRRCI
            _ => old,
        };
        csr_write(&mut st.csr, addr, new);
    }
    set_reg(&mut st.regs, rd, old);
}

/// Restores `mstatus` on `MRET`: `MIE ← MPIE`, `MPIE ← 1`.
fn apply_mret(csr: &mut Csrs) {
    if csr.mstatus & MSTATUS_MPIE != 0 {
        csr.mstatus |= MSTATUS_MIE;
    } else {
        csr.mstatus &= !MSTATUS_MIE;
    }
    csr.mstatus |= MSTATUS_MPIE;
}

/// Enters a trap handler: saves the PC and cause, masks interrupts, and vectors to
/// `mtvec`.
///
/// `mstatus`: `MPIE ← MIE`, `MIE ← 0`, `MPP ← M`. `mepc ← pc`, `mcause ← trap.cause()`,
/// `pc ← mtvec & !3` (direct mode). For a synchronous exception the PC still points at
/// the faulting instruction; for an interrupt it points at the next instruction to run.
///
/// # Arguments
///
/// * `st` - The hart state to update.
/// * `trap` - The trap cause.
pub fn take_trap(st: &mut HartState, trap: Trap) {
    st.csr.mepc = st.pc;
    st.csr.mcause = trap.cause();
    if st.csr.mstatus & MSTATUS_MIE != 0 {
        st.csr.mstatus |= MSTATUS_MPIE;
    } else {
        st.csr.mstatus &= !MSTATUS_MPIE;
    }
    st.csr.mstatus &= !MSTATUS_MIE;
    st.csr.mstatus |= MSTATUS_MPP;
    st.pc = st.csr.mtvec & !0x3;
}

/// Decodes and executes a single RV32IM instruction.
///
/// # Arguments
///
/// * `bus` - The memory bus for loads and stores.
/// * `st` - The hart state (registers, PC, CSRs).
/// * `inst` - The 32-bit instruction word.
///
/// # Returns
///
/// [`StepResult::Continue`] on a normal instruction (PC advanced),
/// [`StepResult::Trapped`] on a synchronous trap (PC unchanged, ready for
/// [`take_trap`]), or [`StepResult::WaitForInterrupt`] on a `WFI`.
pub fn step(bus: &mut dyn Bus, st: &mut HartState, inst: u32) -> StepResult {
    let opcode = inst & 0x7f;
    let rd = ((inst >> 7) & 0x1f) as usize;
    let funct3 = (inst >> 12) & 0x7;
    let rs1 = ((inst >> 15) & 0x1f) as usize;
    let rs2 = ((inst >> 20) & 0x1f) as usize;
    let funct7 = (inst >> 25) & 0x7f;

    let imm_i = sign_extend((inst >> 20) & 0xfff, 12);
    let imm_s = sign_extend((((inst >> 25) & 0x7f) << 5) | ((inst >> 7) & 0x1f), 12);
    let imm_b = sign_extend(
        (((inst >> 31) & 1) << 12)
            | (((inst >> 7) & 1) << 11)
            | (((inst >> 25) & 0x3f) << 5)
            | (((inst >> 8) & 0xf) << 1),
        13,
    );
    let imm_u = inst & 0xffff_f000;
    let imm_j = sign_extend(
        (((inst >> 31) & 1) << 20)
            | (((inst >> 12) & 0xff) << 12)
            | (((inst >> 20) & 1) << 11)
            | (((inst >> 21) & 0x3ff) << 1),
        21,
    );

    let cur_pc = st.pc;
    let rs1v = st.regs[rs1];
    let rs2v = st.regs[rs2];
    let mut next_pc = cur_pc.wrapping_add(4);
    let mut wfi = false;

    match opcode {
        // LUI
        0x37 => set_reg(&mut st.regs, rd, imm_u),
        // AUIPC
        0x17 => set_reg(&mut st.regs, rd, cur_pc.wrapping_add(imm_u)),
        // JAL
        0x6F => {
            set_reg(&mut st.regs, rd, next_pc);
            next_pc = cur_pc.wrapping_add(imm_j as u32);
        }
        // JALR
        0x67 => {
            let target = rs1v.wrapping_add(imm_i as u32) & !1;
            set_reg(&mut st.regs, rd, next_pc);
            next_pc = target;
        }
        // BRANCH
        0x63 => {
            let take = match funct3 {
                0 => rs1v == rs2v,                   // BEQ
                1 => rs1v != rs2v,                   // BNE
                4 => (rs1v as i32) < (rs2v as i32),  // BLT
                5 => (rs1v as i32) >= (rs2v as i32), // BGE
                6 => rs1v < rs2v,                    // BLTU
                7 => rs1v >= rs2v,                   // BGEU
                _ => return StepResult::Trapped(Trap::IllegalInstruction),
            };
            if take {
                next_pc = cur_pc.wrapping_add(imm_b as u32);
            }
        }
        // LOAD
        0x03 => {
            let addr = rs1v.wrapping_add(imm_i as u32);
            let len = match funct3 {
                0 | 4 => 1, // LB / LBU
                1 | 5 => 2, // LH / LHU
                2 => 4,     // LW
                _ => return StepResult::Trapped(Trap::IllegalInstruction),
            };
            let Ok(raw) = bus.read(addr, len) else {
                return StepResult::Trapped(Trap::LoadAccessFault);
            };
            let value = match funct3 {
                0 => sign_extend(raw, 8) as u32,  // LB
                1 => sign_extend(raw, 16) as u32, // LH
                _ => raw,                         // LW / LBU / LHU
            };
            set_reg(&mut st.regs, rd, value);
        }
        // STORE
        0x23 => {
            let addr = rs1v.wrapping_add(imm_s as u32);
            let len = match funct3 {
                0 => 1, // SB
                1 => 2, // SH
                2 => 4, // SW
                _ => return StepResult::Trapped(Trap::IllegalInstruction),
            };
            if bus.write(addr, rs2v, len).is_err() {
                return StepResult::Trapped(Trap::StoreAccessFault);
            }
        }
        // OP-IMM
        0x13 => {
            let imm = imm_i as u32;
            let shamt = (inst >> 20) & 0x1f;
            let value = match funct3 {
                0 => rs1v.wrapping_add(imm),           // ADDI
                2 => u32::from((rs1v as i32) < imm_i), // SLTI
                3 => u32::from(rs1v < imm),            // SLTIU
                4 => rs1v ^ imm,                       // XORI
                6 => rs1v | imm,                       // ORI
                7 => rs1v & imm,                       // ANDI
                1 => rs1v << shamt,                    // SLLI
                5 => {
                    if funct7 & 0x20 != 0 {
                        ((rs1v as i32) >> shamt) as u32 // SRAI
                    } else {
                        rs1v >> shamt // SRLI
                    }
                }
                _ => return StepResult::Trapped(Trap::IllegalInstruction),
            };
            set_reg(&mut st.regs, rd, value);
        }
        // OP (base + M extension)
        0x33 => {
            let value = if funct7 == 0x01 {
                // RV32M
                match funct3 {
                    0 => rs1v.wrapping_mul(rs2v),         // MUL
                    1 => mulh(rs1v as i32, rs2v as i32),  // MULH
                    2 => mulhsu(rs1v as i32, rs2v),       // MULHSU
                    3 => mulhu(rs1v, rs2v),               // MULHU
                    4 => div_s(rs1v as i32, rs2v as i32), // DIV
                    5 => div_u(rs1v, rs2v),               // DIVU
                    6 => rem_s(rs1v as i32, rs2v as i32), // REM
                    7 => rem_u(rs1v, rs2v),               // REMU
                    _ => return StepResult::Trapped(Trap::IllegalInstruction),
                }
            } else {
                let shamt = rs2v & 0x1f;
                match (funct3, funct7) {
                    (0, 0x00) => rs1v.wrapping_add(rs2v),               // ADD
                    (0, 0x20) => rs1v.wrapping_sub(rs2v),               // SUB
                    (1, _) => rs1v << shamt,                            // SLL
                    (2, _) => u32::from((rs1v as i32) < (rs2v as i32)), // SLT
                    (3, _) => u32::from(rs1v < rs2v),                   // SLTU
                    (4, _) => rs1v ^ rs2v,                              // XOR
                    (5, 0x00) => rs1v >> shamt,                         // SRL
                    (5, 0x20) => ((rs1v as i32) >> shamt) as u32,       // SRA
                    (6, _) => rs1v | rs2v,                              // OR
                    (7, _) => rs1v & rs2v,                              // AND
                    _ => return StepResult::Trapped(Trap::IllegalInstruction),
                }
            };
            set_reg(&mut st.regs, rd, value);
        }
        // FENCE (no-op in this model)
        0x0F => {}
        // SYSTEM (ECALL/EBREAK/MRET/WFI and Zicsr)
        0x73 => {
            let csr_addr = ((inst >> 20) & 0xfff) as u16;
            match funct3 {
                0 => match (inst >> 20) & 0xfff {
                    0x000 => return StepResult::Trapped(Trap::EcallFromM),
                    0x001 => return StepResult::Trapped(Trap::Breakpoint),
                    0x302 => {
                        // MRET
                        next_pc = st.csr.mepc;
                        apply_mret(&mut st.csr);
                    }
                    0x105 => wfi = true, // WFI
                    _ => return StepResult::Trapped(Trap::IllegalInstruction),
                },
                1 | 2 | 3 | 5 | 6 | 7 => csr_op(st, funct3, csr_addr, rs1, rs1v, rd),
                _ => return StepResult::Trapped(Trap::IllegalInstruction),
            }
        }
        // Any unrecognized opcode is illegal.
        _ => return StepResult::Trapped(Trap::IllegalInstruction),
    }

    st.pc = next_pc;
    if wfi {
        StepResult::WaitForInterrupt
    } else {
        StepResult::Continue
    }
}

/// A minimal RV32IM assembler: instruction encoders matching [`step`]'s decoder.
///
/// These make example programs correct by construction and document the encoding.
pub mod asm {
    /// Encodes an R-type instruction.
    fn r(opcode: u32, rd: u32, funct3: u32, rs1: u32, rs2: u32, funct7: u32) -> u32 {
        opcode | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (rs2 << 20) | (funct7 << 25)
    }

    /// Encodes an I-type instruction (12-bit signed immediate).
    fn i(opcode: u32, rd: u32, funct3: u32, rs1: u32, imm: i32) -> u32 {
        let imm = (imm as u32) & 0xfff;
        opcode | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (imm << 20)
    }

    /// Encodes an S-type instruction (12-bit signed immediate).
    fn s(opcode: u32, funct3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
        let imm = (imm as u32) & 0xfff;
        opcode
            | ((imm & 0x1f) << 7)
            | (funct3 << 12)
            | (rs1 << 15)
            | (rs2 << 20)
            | (((imm >> 5) & 0x7f) << 25)
    }

    /// Encodes a B-type instruction (13-bit signed, even, branch offset).
    fn b(opcode: u32, funct3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
        let imm = imm as u32;
        opcode
            | (((imm >> 11) & 1) << 7)
            | (((imm >> 1) & 0xf) << 8)
            | (funct3 << 12)
            | (rs1 << 15)
            | (rs2 << 20)
            | (((imm >> 5) & 0x3f) << 25)
            | (((imm >> 12) & 1) << 31)
    }

    /// Encodes a U-type instruction (upper 20-bit immediate).
    fn u(opcode: u32, rd: u32, imm: u32) -> u32 {
        opcode | (rd << 7) | (imm & 0xffff_f000)
    }

    /// Encodes a J-type instruction (21-bit signed, even, jump offset).
    fn j(opcode: u32, rd: u32, imm: i32) -> u32 {
        let imm = imm as u32;
        opcode
            | (rd << 7)
            | (((imm >> 12) & 0xff) << 12)
            | (((imm >> 11) & 1) << 20)
            | (((imm >> 1) & 0x3ff) << 21)
            | (((imm >> 20) & 1) << 31)
    }

    /// `addi rd, rs1, imm`
    pub fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(0x13, rd, 0, rs1, imm)
    }

    /// `ori rd, rs1, imm`
    pub fn ori(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(0x13, rd, 6, rs1, imm)
    }

    /// `andi rd, rs1, imm`
    pub fn andi(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(0x13, rd, 7, rs1, imm)
    }

    /// `add rd, rs1, rs2`
    pub fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 0, rs1, rs2, 0x00)
    }

    /// `sub rd, rs1, rs2`
    pub fn sub(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 0, rs1, rs2, 0x20)
    }

    /// `and rd, rs1, rs2`
    pub fn and(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 7, rs1, rs2, 0x00)
    }

    /// `or rd, rs1, rs2`
    pub fn or(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 6, rs1, rs2, 0x00)
    }

    /// `xor rd, rs1, rs2`
    pub fn xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 4, rs1, rs2, 0x00)
    }

    /// `slli rd, rs1, shamt`
    pub fn slli(rd: u32, rs1: u32, shamt: u32) -> u32 {
        i(0x13, rd, 1, rs1, shamt as i32)
    }

    /// `mul rd, rs1, rs2`
    pub fn mul(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 0, rs1, rs2, 0x01)
    }

    /// `mulh rd, rs1, rs2`
    pub fn mulh(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 1, rs1, rs2, 0x01)
    }

    /// `mulhsu rd, rs1, rs2`
    pub fn mulhsu(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 2, rs1, rs2, 0x01)
    }

    /// `mulhu rd, rs1, rs2`
    pub fn mulhu(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 3, rs1, rs2, 0x01)
    }

    /// `div rd, rs1, rs2`
    pub fn div(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 4, rs1, rs2, 0x01)
    }

    /// `divu rd, rs1, rs2`
    pub fn divu(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 5, rs1, rs2, 0x01)
    }

    /// `rem rd, rs1, rs2`
    pub fn rem(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 6, rs1, rs2, 0x01)
    }

    /// `remu rd, rs1, rs2`
    pub fn remu(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 7, rs1, rs2, 0x01)
    }

    /// `lui rd, imm` (imm occupies the upper 20 bits).
    pub fn lui(rd: u32, imm: u32) -> u32 {
        u(0x37, rd, imm)
    }

    /// `auipc rd, imm` (imm occupies the upper 20 bits).
    pub fn auipc(rd: u32, imm: u32) -> u32 {
        u(0x17, rd, imm)
    }

    /// `jal rd, offset`
    pub fn jal(rd: u32, offset: i32) -> u32 {
        j(0x6f, rd, offset)
    }

    /// `jalr rd, rs1, imm`
    pub fn jalr(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(0x67, rd, 0, rs1, imm)
    }

    /// `beq rs1, rs2, offset`
    pub fn beq(rs1: u32, rs2: u32, offset: i32) -> u32 {
        b(0x63, 0, rs1, rs2, offset)
    }

    /// `bne rs1, rs2, offset`
    pub fn bne(rs1: u32, rs2: u32, offset: i32) -> u32 {
        b(0x63, 1, rs1, rs2, offset)
    }

    /// `blt rs1, rs2, offset`
    pub fn blt(rs1: u32, rs2: u32, offset: i32) -> u32 {
        b(0x63, 4, rs1, rs2, offset)
    }

    /// `bge rs1, rs2, offset`
    pub fn bge(rs1: u32, rs2: u32, offset: i32) -> u32 {
        b(0x63, 5, rs1, rs2, offset)
    }

    /// `lw rd, offset(rs1)`
    pub fn lw(rd: u32, offset: i32, rs1: u32) -> u32 {
        i(0x03, rd, 2, rs1, offset)
    }

    /// `lbu rd, offset(rs1)`
    pub fn lbu(rd: u32, offset: i32, rs1: u32) -> u32 {
        i(0x03, rd, 4, rs1, offset)
    }

    /// `sw rs2, offset(rs1)`
    pub fn sw(rs2: u32, offset: i32, rs1: u32) -> u32 {
        s(0x23, 2, rs1, rs2, offset)
    }

    /// `sb rs2, offset(rs1)`
    pub fn sb(rs2: u32, offset: i32, rs1: u32) -> u32 {
        s(0x23, 0, rs1, rs2, offset)
    }

    /// `csrrw rd, csr, rs1`
    pub fn csrrw(rd: u32, csr: u32, rs1: u32) -> u32 {
        i(0x73, rd, 1, rs1, csr as i32)
    }

    /// `csrrs rd, csr, rs1`
    pub fn csrrs(rd: u32, csr: u32, rs1: u32) -> u32 {
        i(0x73, rd, 2, rs1, csr as i32)
    }

    /// `csrrc rd, csr, rs1`
    pub fn csrrc(rd: u32, csr: u32, rs1: u32) -> u32 {
        i(0x73, rd, 3, rs1, csr as i32)
    }

    /// `csrrwi rd, csr, uimm`
    pub fn csrrwi(rd: u32, csr: u32, uimm: u32) -> u32 {
        i(0x73, rd, 5, uimm, csr as i32)
    }

    /// `csrrsi rd, csr, uimm`
    pub fn csrrsi(rd: u32, csr: u32, uimm: u32) -> u32 {
        i(0x73, rd, 6, uimm, csr as i32)
    }

    /// `ecall`
    pub fn ecall() -> u32 {
        0x0000_0073
    }

    /// `ebreak`
    pub fn ebreak() -> u32 {
        0x0010_0073
    }

    /// `mret`
    pub fn mret() -> u32 {
        0x3020_0073
    }

    /// `wfi`
    pub fn wfi() -> u32 {
        0x1050_0073
    }
}

/// Assembles a slice of instruction words into little-endian bytes.
///
/// # Arguments
///
/// * `instructions` - The instruction words, in program order.
///
/// # Returns
///
/// The machine-code bytes.
pub fn assemble(instructions: &[u32]) -> Vec<u8> {
    instructions
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect()
}

/// Builds a program computing `1 + 2 + … + n` and storing the sum (a word) at
/// `result_addr`, then halting with `ecall`.
///
/// # Arguments
///
/// * `n` - The (inclusive) upper bound of the summation.
/// * `result_addr` - The word address to store the result at (must fit a 12-bit
///   signed store offset from `x0`, i.e. ≤ 2047).
///
/// # Returns
///
/// The assembled machine-code bytes, intended to be loaded at address 0.
pub fn program_sum_1_to_n(n: u32, result_addr: u32) -> Vec<u8> {
    use asm::{add, addi, blt, ecall, sw};
    // Registers: x1 = sum, x2 = i, x3 = limit (= n + 1).
    let program = [
        addi(1, 0, 0),                // 0:  x1 = 0
        addi(2, 0, 1),                // 4:  x2 = 1
        addi(3, 0, (n + 1) as i32),   // 8:  x3 = n + 1
        add(1, 1, 2),                 // 12: loop: x1 += x2
        addi(2, 2, 1),                // 16: x2 += 1
        blt(2, 3, -8),                // 20: if x2 < x3 goto loop
        sw(1, result_addr as i32, 0), // 24: mem[result_addr] = x1
        ecall(),                      // 28: halt
    ];
    assemble(&program)
}

/// A [`Bus`] backed by an initiator socket: every access is a `b_transport`.
struct SocketBus<'a> {
    /// The kernel handle for the access (may yield on modelled latency).
    cx: &'a Ctx,

    /// The initiator socket to the bus fabric (memory or interconnect).
    isock: InitiatorSocket,
}

impl Bus for SocketBus<'_> {
    fn read(&mut self, addr: u32, len: usize) -> Result<u32, BusError> {
        let bytes = BusMaster::new(self.isock)
            .read(self.cx, u64::from(addr), len)
            .map_err(|_| BusError)?;
        let mut value = 0u32;
        for (i, &byte) in bytes.iter().enumerate().take(4) {
            value |= u32::from(byte) << (8 * i);
        }
        Ok(value)
    }

    fn write(&mut self, addr: u32, value: u32, len: usize) -> Result<(), BusError> {
        let bytes = value.to_le_bytes();
        BusMaster::new(self.isock)
            .write(self.cx, u64::from(addr), bytes[..len].to_vec())
            .map_err(|_| BusError)
    }
}

/// Builds a minimal RV32IM hart as an `SC_THREAD` that **halts on any trap**.
///
/// The hart fetches from `entry`, decodes and executes via [`step`], and waits `cycle`
/// per instruction. It halts on `ecall`/`ebreak`, an illegal instruction, a `wfi`, or a
/// bus fault — suitable for run-to-completion programs (e.g. [`program_sum_1_to_n`])
/// bound directly to a single memory. For interrupt-driven SoCs use [`build_soc_hart`].
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
/// * `isock` - The hart's initiator socket (must be bound to a memory or bus target).
/// * `entry` - The reset program-counter value.
/// * `cycle` - The modelled time per executed instruction.
// ANCHOR: hart
pub fn build_hart(sim: &Sim, isock: InitiatorSocket, entry: u32, cycle: SimTime) {
    sim.add_thread("hart", &[], true, move |cx| {
        let mut st = HartState::new(entry);
        loop {
            let mut bus = SocketBus { cx, isock };
            // a b_transport, several calls deep; a fetch fault halts the simple hart
            let Ok(inst) = bus.read(st.pc, 4) else { break };
            match step(&mut bus, &mut st, inst) {
                StepResult::Continue => {}
                _ => break, // any trap or wfi halts the simple hart
            }
            if !cycle.is_zero() {
                cx.wait(cycle);
            }
        }
    });
}
// ANCHOR_END: hart

/// The interrupt lines a [`build_soc_hart`] samples into `mip` each instruction.
///
/// Each present line maps to one `mip`/`mie` bit (`MTIP`, `MSIP`, `MEIP`). Absent
/// (`None`) sources never assert.
#[derive(Debug, Clone, Copy, Default)]
pub struct HartIrqs {
    /// The machine-timer interrupt line (drives `MTIP`).
    pub timer: Option<IrqLine>,

    /// The machine-software interrupt line (drives `MSIP`).
    pub software: Option<IrqLine>,

    /// The machine-external interrupt line (drives `MEIP`).
    pub external: Option<IrqLine>,
}

/// Samples the IRQ lines into `csr.mip` (level-sensitive: the bit tracks the line).
fn sample_irqs(cx: &Ctx, csr: &mut Csrs, irqs: &HartIrqs) {
    let mut mip = csr.mip & !(MIP_MSIP | MIP_MTIP | MIP_MEIP);
    if irqs.software.is_some_and(|l| l.pending(cx)) {
        mip |= MIP_MSIP;
    }
    if irqs.timer.is_some_and(|l| l.pending(cx)) {
        mip |= MIP_MTIP;
    }
    if irqs.external.is_some_and(|l| l.pending(cx)) {
        mip |= MIP_MEIP;
    }
    csr.mip = mip;
}

/// Returns the highest-priority enabled+pending interrupt, if any (external > software
/// > timer), gated by the global `mstatus.MIE`.
fn pending_interrupt(csr: &Csrs) -> Option<Trap> {
    if csr.mstatus & MSTATUS_MIE == 0 {
        return None;
    }
    let active = csr.mip & csr.mie;
    if active & MIP_MEIP != 0 {
        Some(Trap::MachineExternal)
    } else if active & MIP_MSIP != 0 {
        Some(Trap::MachineSoftware)
    } else if active & MIP_MTIP != 0 {
        Some(Trap::MachineTimer)
    } else {
        None
    }
}

/// Builds a machine-mode RV32IM hart as an `SC_THREAD` driving an interrupt-capable
/// SoC.
///
/// The hart's architectural state lives in `state` (an `Rc<RefCell<HartState>>`) so it
/// is snapshot/restore-clean. Each iteration the hart: (1) samples the IRQ lines into
/// `mip` and takes the highest-priority enabled interrupt (vectoring to `mtvec`);
/// (2) fetches and executes one instruction via [`step`]; (3) on a synchronous trap
/// enters the handler, on `wfi` idles on [`irq_wake_event`] until a line changes; and
/// (4) advances `cycle` of modelled time at the per-instruction boundary (the
/// snapshot-safe point — the state is fully committed and no borrow is held). The
/// state is lifted into stack locals for the body of each instruction, so the
/// `RefCell` borrow is never held across the `wait()` inside a load/store.
///
/// The shared `state` is registered as a kernel service so the (necessarily `Send`)
/// thread body can retrieve it via `cx` rather than capturing the `!Send` `Rc` — the
/// same discipline the DMA example uses for its register file. One hart per simulation
/// (the service is keyed by type); the caller keeps its `state` clone for inspection
/// and snapshot/restore.
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
/// * `isock` - The hart's initiator socket (bound to the bus interconnect).
/// * `state` - The shared hart state (register file + PC + CSRs); set `pc` to the reset
///   vector before building.
/// * `irqs` - The interrupt lines feeding `mip`.
/// * `cycle` - The modelled time per executed instruction.
pub fn build_soc_hart(
    sim: &Sim,
    isock: InitiatorSocket,
    state: Rc<RefCell<HartState>>,
    irqs: HartIrqs,
    cycle: SimTime,
) {
    // Register the state so the Send thread body can fetch it via `cx` (it must not
    // capture the !Send Rc directly). The caller keeps its own clone.
    sim.register_service(state);
    let wake = irq_wake_event(sim);
    sim.add_thread("hart", &[], true, move |cx| {
        let state = cx.service::<RefCell<HartState>>();
        loop {
            // Lift the state into stack locals (no RefCell borrow held across waits).
            let mut st = *state.borrow();

            // 1. Sample interrupt lines and take a pending enabled interrupt.
            sample_irqs(cx, &mut st.csr, &irqs);
            if let Some(trap) = pending_interrupt(&st.csr) {
                take_trap(&mut st, trap); // st.pc ← mtvec
            }

            // 2. Fetch.
            let mut bus = SocketBus { cx, isock };
            let Ok(inst) = bus.read(st.pc, 4) else {
                take_trap(&mut st, Trap::InstructionAccessFault);
                *state.borrow_mut() = st;
                if !cycle.is_zero() {
                    cx.wait(cycle);
                }
                continue;
            };

            // 3. Execute.
            match step(&mut bus, &mut st, inst) {
                StepResult::Continue => {}
                StepResult::Trapped(trap) => take_trap(&mut st, trap),
                StepResult::WaitForInterrupt => {
                    *state.borrow_mut() = st;
                    // Idle until an interrupt is pending; re-sample to avoid a missed
                    // wake, then park on the aggregate change event if still idle.
                    let mut probe = *state.borrow();
                    sample_irqs(cx, &mut probe.csr, &irqs);
                    *state.borrow_mut() = probe;
                    if probe.csr.mip & probe.csr.mie == 0 {
                        cx.wait_event(wake);
                    }
                    continue;
                }
            }

            // 4. Commit and advance time (the snapshot-safe boundary).
            *state.borrow_mut() = st;
            if !cycle.is_zero() {
                cx.wait(cycle);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        Bus, BusError, CSR_MSCRATCH, CSR_MTVEC, HartState, MSTATUS_MIE, MSTATUS_MPIE, NUM_REGS,
        StepResult, Trap, asm, step, take_trap,
    };

    /// A simple in-memory bus for unit-testing the ISA without a simulation.
    struct VecBus {
        /// The backing bytes.
        mem: Vec<u8>,
    }

    impl Bus for VecBus {
        fn read(&mut self, addr: u32, len: usize) -> Result<u32, BusError> {
            let a = addr as usize;
            if a + len > self.mem.len() {
                return Err(BusError);
            }
            let mut value = 0u32;
            for (i, &byte) in self.mem[a..a + len].iter().enumerate() {
                value |= u32::from(byte) << (8 * i);
            }
            Ok(value)
        }

        fn write(&mut self, addr: u32, value: u32, len: usize) -> Result<(), BusError> {
            let a = addr as usize;
            if a + len > self.mem.len() {
                return Err(BusError);
            }
            let bytes = value.to_le_bytes();
            self.mem[a..a + len].copy_from_slice(&bytes[..len]);
            Ok(())
        }
    }

    /// Executes one instruction against a fresh hart state and returns it.
    fn exec(inst: u32, setup: impl FnOnce(&mut [u32; NUM_REGS])) -> HartState {
        let mut st = HartState::new(0);
        setup(&mut st.regs);
        let mut bus = VecBus { mem: vec![0; 64] };
        assert_eq!(step(&mut bus, &mut st, inst), StepResult::Continue);
        st
    }

    /// Verifies `addi`, including writes to `x0` being discarded.
    #[test]
    fn addi_and_x0_hardwired() {
        let st = exec(asm::addi(1, 0, 42), |_| {});
        assert_eq!(st.regs[1], 42);
        let st = exec(asm::addi(0, 0, 42), |_| {});
        assert_eq!(st.regs[0], 0); // x0 stays zero
    }

    /// Verifies `add` and `sub`, including two's-complement wraparound.
    #[test]
    fn add_and_sub() {
        let st = exec(asm::add(3, 1, 2), |r| {
            r[1] = 100;
            r[2] = 23;
        });
        assert_eq!(st.regs[3], 123);
        let st = exec(asm::sub(3, 1, 2), |r| {
            r[1] = 5;
            r[2] = 8;
        });
        assert_eq!(st.regs[3], (-3i32) as u32);
    }

    /// Verifies `xor` and `slli`.
    #[test]
    fn xor_and_shift() {
        let st = exec(asm::xor(3, 1, 2), |r| {
            r[1] = 0xff00;
            r[2] = 0x0ff0;
        });
        assert_eq!(st.regs[3], 0xf0f0);
        let st = exec(asm::slli(3, 1, 4), |r| r[1] = 1);
        assert_eq!(st.regs[3], 16);
    }

    /// Verifies the RV32M multiply variants, including the high-half products.
    #[test]
    fn m_extension_multiply() {
        let st = exec(asm::mul(3, 1, 2), |r| {
            r[1] = 6;
            r[2] = 7;
        });
        assert_eq!(st.regs[3], 42);
        // MULHU of two large unsigned values: 0xFFFF_FFFF * 0xFFFF_FFFF >> 32.
        let st = exec(asm::mulhu(3, 1, 2), |r| {
            r[1] = 0xFFFF_FFFF;
            r[2] = 0xFFFF_FFFF;
        });
        assert_eq!(st.regs[3], 0xFFFF_FFFE);
        // MULH of (-1) * (-1) = 1, high half is 0.
        let st = exec(asm::mulh(3, 1, 2), |r| {
            r[1] = (-1i32) as u32;
            r[2] = (-1i32) as u32;
        });
        assert_eq!(st.regs[3], 0);
        // MULHSU of (-1) * 2: signed -1 × unsigned 2 = -2, high half is all-ones.
        let st = exec(asm::mulhsu(3, 1, 2), |r| {
            r[1] = (-1i32) as u32;
            r[2] = 2;
        });
        assert_eq!(st.regs[3], 0xFFFF_FFFF);
    }

    /// Verifies the RV32M divide/remainder variants and their RISC-V edge cases.
    #[test]
    fn m_extension_divide_edge_cases() {
        let st = exec(asm::div(3, 1, 2), |r| {
            r[1] = 20;
            r[2] = 6;
        });
        assert_eq!(st.regs[3], 3);
        let st = exec(asm::rem(3, 1, 2), |r| {
            r[1] = 20;
            r[2] = 6;
        });
        assert_eq!(st.regs[3], 2);
        // Divide by zero: DIV → -1, REM → dividend.
        let st = exec(asm::div(3, 1, 2), |r| {
            r[1] = 123;
            r[2] = 0;
        });
        assert_eq!(st.regs[3], u32::MAX);
        let st = exec(asm::rem(3, 1, 2), |r| {
            r[1] = 123;
            r[2] = 0;
        });
        assert_eq!(st.regs[3], 123);
        // Signed overflow: INT_MIN / -1 → INT_MIN, REM → 0.
        let st = exec(asm::div(3, 1, 2), |r| {
            r[1] = i32::MIN as u32;
            r[2] = (-1i32) as u32;
        });
        assert_eq!(st.regs[3], i32::MIN as u32);
        let st = exec(asm::rem(3, 1, 2), |r| {
            r[1] = i32::MIN as u32;
            r[2] = (-1i32) as u32;
        });
        assert_eq!(st.regs[3], 0);
        // Unsigned divide by zero: DIVU → all-ones, REMU → dividend.
        let st = exec(asm::divu(3, 1, 2), |r| {
            r[1] = 50;
            r[2] = 0;
        });
        assert_eq!(st.regs[3], u32::MAX);
        let st = exec(asm::remu(3, 1, 2), |r| {
            r[1] = 50;
            r[2] = 0;
        });
        assert_eq!(st.regs[3], 50);
    }

    /// Verifies `lui` places the immediate in the upper 20 bits.
    #[test]
    fn lui_upper_immediate() {
        let st = exec(asm::lui(1, 0xABCD_E000), |_| {});
        assert_eq!(st.regs[1], 0xABCD_E000);
    }

    /// Verifies a taken `blt` redirects the PC by the (negative) branch offset.
    #[test]
    fn blt_taken_branches() {
        let mut st = HartState::new(20);
        st.regs[1] = 1;
        st.regs[2] = 10;
        let mut bus = VecBus { mem: vec![0; 64] };
        // blt x1, x2, -8 : 1 < 10 ⇒ taken ⇒ pc = 12.
        step(&mut bus, &mut st, asm::blt(1, 2, -8));
        assert_eq!(st.pc, 12);
    }

    /// Verifies `jal` links the return address and jumps.
    #[test]
    fn jal_links_and_jumps() {
        let mut st = HartState::new(100);
        let mut bus = VecBus { mem: vec![0; 64] };
        step(&mut bus, &mut st, asm::jal(1, 16));
        assert_eq!(st.regs[1], 104); // return address = pc + 4
        assert_eq!(st.pc, 116); // jump target = pc + 16
    }

    /// Verifies a `sw` then `lw` round-trips a word through the bus.
    #[test]
    fn store_then_load_word() {
        let mut st = HartState::new(0);
        st.regs[1] = 0xDEAD_BEEF; // value
        st.regs[2] = 0; // base
        let mut bus = VecBus { mem: vec![0; 64] };
        step(&mut bus, &mut st, asm::sw(1, 8, 2));
        st.pc = 0;
        step(&mut bus, &mut st, asm::lw(3, 8, 2));
        assert_eq!(st.regs[3], 0xDEAD_BEEF);
    }

    /// Verifies `ecall` raises an environment-call trap without advancing the PC.
    #[test]
    fn ecall_traps() {
        let mut st = HartState::new(0x40);
        let mut bus = VecBus { mem: vec![0; 64] };
        assert_eq!(
            step(&mut bus, &mut st, asm::ecall()),
            StepResult::Trapped(Trap::EcallFromM)
        );
        assert_eq!(st.pc, 0x40); // PC not advanced; take_trap will save it
    }

    /// Verifies an out-of-range load surfaces as a load access fault.
    #[test]
    fn load_fault_traps() {
        let mut st = HartState::new(0);
        st.regs[1] = 0x1000; // out of the 64-byte VecBus
        let mut bus = VecBus { mem: vec![0; 64] };
        assert_eq!(
            step(&mut bus, &mut st, asm::lw(2, 0, 1)),
            StepResult::Trapped(Trap::LoadAccessFault)
        );
    }

    /// Verifies CSR read-modify-write: `csrrw` swaps, `csrrs` sets bits, and `csrrwi`
    /// writes an immediate.
    #[test]
    fn csr_read_modify_write() {
        let mut st = HartState::new(0);
        st.regs[1] = 0xAA;
        let mut bus = VecBus { mem: vec![0; 64] };
        // csrrw x2, mscratch, x1 : old (0) → x2, mscratch ← 0xAA.
        step(&mut bus, &mut st, asm::csrrw(2, CSR_MSCRATCH, 1));
        assert_eq!(st.regs[2], 0);
        assert_eq!(st.csr.mscratch, 0xAA);
        // csrrs x3, mscratch, x1 : old (0xAA) → x3, mscratch ← 0xAA | 0xAA.
        step(&mut bus, &mut st, asm::csrrs(3, CSR_MSCRATCH, 1));
        assert_eq!(st.regs[3], 0xAA);
        assert_eq!(st.csr.mscratch, 0xAA);
        // csrrwi x0, mtvec, 8 : mtvec ← 8.
        step(&mut bus, &mut st, asm::csrrwi(0, CSR_MTVEC, 8));
        assert_eq!(st.csr.mtvec, 8);
    }

    /// Verifies the trap-entry/return dance: `take_trap` saves state and vectors to
    /// `mtvec`; `mret` restores `mstatus.MIE` and returns to `mepc`.
    #[test]
    fn trap_entry_and_mret() {
        let mut st = HartState::new(0x100);
        st.csr.mtvec = 0x800;
        st.csr.mstatus = MSTATUS_MIE; // interrupts enabled
        take_trap(&mut st, Trap::EcallFromM);
        assert_eq!(st.csr.mepc, 0x100); // faulting PC saved
        assert_eq!(st.csr.mcause, 11); // ECALL-from-M cause
        assert_eq!(st.pc, 0x800); // vectored to mtvec
        assert_eq!(st.csr.mstatus & MSTATUS_MIE, 0); // MIE cleared
        assert_eq!(st.csr.mstatus & MSTATUS_MPIE, MSTATUS_MPIE); // MPIE saved the old MIE

        // Returning: mret restores MIE from MPIE and jumps to mepc.
        let mut bus = VecBus { mem: vec![0; 64] };
        st.csr.mepc = 0x104; // handler advanced past the ecall
        step(&mut bus, &mut st, asm::mret());
        assert_eq!(st.pc, 0x104);
        assert_eq!(st.csr.mstatus & MSTATUS_MIE, MSTATUS_MIE); // MIE restored
    }

    /// Runs the bundled `sum 1..=n` program on the in-memory bus and checks the stored
    /// result (1 + 2 + … + 10 = 55).
    #[test]
    fn sum_program_pure() {
        let result_addr = 0x100u32;
        let code = super::program_sum_1_to_n(10, result_addr);
        let mut mem = vec![0u8; 1024];
        mem[..code.len()].copy_from_slice(&code);
        let mut bus = VecBus { mem };

        let mut st = HartState::new(0);
        // Bounded to avoid an accidental infinite loop in a broken build.
        for _ in 0..1000 {
            let inst = bus.read(st.pc, 4).unwrap();
            if step(&mut bus, &mut st, inst) != StepResult::Continue {
                break; // the trailing ecall traps
            }
        }
        assert_eq!(bus.read(result_addr, 4).unwrap(), 55);
    }
}
