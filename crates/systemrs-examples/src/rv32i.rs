//! Example 2: a basic RV32I CPU hart.
//!
//! An `SC_THREAD` runs a fetch-decode-execute loop over the RV32I base integer
//! instruction set. **Every** memory access — instruction fetch, load, store — goes
//! through `b_transport` over an initiator socket to a memory target, so `wait()`
//! (the modelled access latency) is reached from deep inside the transport call on
//! the hart's coroutine stack. This is the design's central property in action
//! (`doc/systemrs-design.md` §6a, §6d).
//!
//! The instruction semantics are decoupled from the kernel via the [`Bus`] trait,
//! so the ISA is unit-tested directly against an in-memory bus (see the tests),
//! while the simulation drives a socket-backed bus.

use systemrs::prelude::*;

/// The number of integer registers (`x0`–`x31`).
pub const NUM_REGS: usize = 32;

/// A byte-addressable bus the hart reads instructions and data through.
///
/// Abstracting memory access behind this trait lets the RV32I core ([`step`]) be
/// exercised by both the socket-backed simulation bus and a plain in-memory test
/// bus.
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
    fn read(&mut self, addr: u32, len: usize) -> u32;

    /// Writes the low `len` little-endian bytes of `value` at `addr`.
    ///
    /// # Arguments
    ///
    /// * `addr` - The byte address.
    /// * `value` - The value whose low `len` bytes are written.
    /// * `len` - The number of bytes (1, 2, or 4).
    fn write(&mut self, addr: u32, value: u32, len: usize);
}

/// The outcome of executing one instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    /// Continue fetching the next instruction.
    Continue,

    /// Halt the hart (`ECALL`/`EBREAK`, or an illegal instruction).
    Halt,
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

/// Decodes and executes a single RV32I instruction.
///
/// # Arguments
///
/// * `bus` - The memory bus for loads and stores.
/// * `regs` - The 32 integer registers (`regs[0]` stays zero).
/// * `pc` - The program counter; updated to the next instruction address.
/// * `inst` - The 32-bit instruction word.
///
/// # Returns
///
/// [`StepResult::Continue`] normally, or [`StepResult::Halt`] on a system
/// instruction or an unrecognized opcode.
pub fn step(bus: &mut dyn Bus, regs: &mut [u32; NUM_REGS], pc: &mut u32, inst: u32) -> StepResult {
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

    let cur_pc = *pc;
    let rs1v = regs[rs1];
    let rs2v = regs[rs2];
    let mut next_pc = cur_pc.wrapping_add(4);

    match opcode {
        // LUI
        0x37 => set_reg(regs, rd, imm_u),
        // AUIPC
        0x17 => set_reg(regs, rd, cur_pc.wrapping_add(imm_u)),
        // JAL
        0x6F => {
            set_reg(regs, rd, next_pc);
            next_pc = cur_pc.wrapping_add(imm_j as u32);
        }
        // JALR
        0x67 => {
            let target = rs1v.wrapping_add(imm_i as u32) & !1;
            set_reg(regs, rd, next_pc);
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
                _ => false,
            };
            if take {
                next_pc = cur_pc.wrapping_add(imm_b as u32);
            }
        }
        // LOAD
        0x03 => {
            let addr = rs1v.wrapping_add(imm_i as u32);
            let value = match funct3 {
                0 => sign_extend(bus.read(addr, 1), 8) as u32,  // LB
                1 => sign_extend(bus.read(addr, 2), 16) as u32, // LH
                2 => bus.read(addr, 4),                         // LW
                4 => bus.read(addr, 1),                         // LBU
                5 => bus.read(addr, 2),                         // LHU
                _ => return StepResult::Halt,
            };
            set_reg(regs, rd, value);
        }
        // STORE
        0x23 => {
            let addr = rs1v.wrapping_add(imm_s as u32);
            let len = match funct3 {
                0 => 1, // SB
                1 => 2, // SH
                2 => 4, // SW
                _ => return StepResult::Halt,
            };
            bus.write(addr, rs2v, len);
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
                _ => return StepResult::Halt,
            };
            set_reg(regs, rd, value);
        }
        // OP
        0x33 => {
            let shamt = rs2v & 0x1f;
            let value = match (funct3, funct7) {
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
                _ => return StepResult::Halt,
            };
            set_reg(regs, rd, value);
        }
        // FENCE (no-op in this model)
        0x0F => {}
        // SYSTEM (ECALL/EBREAK) and any unrecognized opcode → halt.
        _ => return StepResult::Halt,
    }

    *pc = next_pc;
    StepResult::Continue
}

/// A minimal RV32I assembler: instruction encoders matching [`step`]'s decoder.
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

    /// `add rd, rs1, rs2`
    pub fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 0, rs1, rs2, 0x00)
    }

    /// `sub rd, rs1, rs2`
    pub fn sub(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 0, rs1, rs2, 0x20)
    }

    /// `xor rd, rs1, rs2`
    pub fn xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0x33, rd, 4, rs1, rs2, 0x00)
    }

    /// `slli rd, rs1, shamt`
    pub fn slli(rd: u32, rs1: u32, shamt: u32) -> u32 {
        i(0x13, rd, 1, rs1, shamt as i32)
    }

    /// `lui rd, imm` (imm occupies the upper 20 bits).
    pub fn lui(rd: u32, imm: u32) -> u32 {
        u(0x37, rd, imm)
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

    /// `lw rd, offset(rs1)`
    pub fn lw(rd: u32, offset: i32, rs1: u32) -> u32 {
        i(0x03, rd, 2, rs1, offset)
    }

    /// `sw rs2, offset(rs1)`
    pub fn sw(rs2: u32, offset: i32, rs1: u32) -> u32 {
        s(0x23, 2, rs1, rs2, offset)
    }

    /// `ecall` (used here to halt the hart).
    pub fn ecall() -> u32 {
        0x0000_0073
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

    /// The initiator socket to the memory target.
    isock: InitiatorSocket,
}

impl Bus for SocketBus<'_> {
    fn read(&mut self, addr: u32, len: usize) -> u32 {
        let mut txn = GenericPayload::read(u64::from(addr), len);
        let mut delay = SimTime::ZERO;
        self.isock.b_transport(self.cx, &mut txn, &mut delay);
        let data = txn.data();
        let mut value = 0u32;
        for (i, &byte) in data.iter().enumerate().take(4) {
            value |= u32::from(byte) << (8 * i);
        }
        value
    }

    fn write(&mut self, addr: u32, value: u32, len: usize) {
        let bytes = value.to_le_bytes();
        let mut txn = GenericPayload::write(u64::from(addr), bytes[..len].to_vec());
        let mut delay = SimTime::ZERO;
        self.isock.b_transport(self.cx, &mut txn, &mut delay);
    }
}

/// Builds an RV32I hart as an `SC_THREAD` into `sim`.
///
/// The hart fetches from `entry`, decodes and executes via [`step`], and waits
/// `cycle` per instruction. It halts on `ecall`/`ebreak`. Results are observed
/// through memory (RISC-V programs communicate via memory).
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
/// * `isock` - The hart's initiator socket (must be bound to a memory target).
/// * `entry` - The reset program-counter value.
/// * `cycle` - The modelled time per executed instruction.
pub fn build_hart(sim: &Sim, isock: InitiatorSocket, entry: u32, cycle: SimTime) {
    sim.add_thread("hart", &[], true, move |cx| {
        let mut regs = [0u32; NUM_REGS];
        let mut pc = entry;
        loop {
            let mut bus = SocketBus { cx, isock };
            let inst = bus.read(pc, 4);
            let result = step(&mut bus, &mut regs, &mut pc, inst);
            if result == StepResult::Halt {
                break;
            }
            if !cycle.is_zero() {
                cx.wait(cycle);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{Bus, NUM_REGS, StepResult, asm, step};

    /// A simple in-memory bus for unit-testing the ISA without a simulation.
    struct VecBus {
        /// The backing bytes.
        mem: Vec<u8>,
    }

    impl Bus for VecBus {
        fn read(&mut self, addr: u32, len: usize) -> u32 {
            let mut value = 0u32;
            for (i, &byte) in self.mem[addr as usize..addr as usize + len]
                .iter()
                .enumerate()
            {
                value |= u32::from(byte) << (8 * i);
            }
            value
        }

        fn write(&mut self, addr: u32, value: u32, len: usize) {
            let bytes = value.to_le_bytes();
            self.mem[addr as usize..addr as usize + len].copy_from_slice(&bytes[..len]);
        }
    }

    /// Executes one instruction against a fresh register file and returns it.
    fn exec(inst: u32, setup: impl FnOnce(&mut [u32; NUM_REGS])) -> [u32; NUM_REGS] {
        let mut regs = [0u32; NUM_REGS];
        setup(&mut regs);
        let mut pc = 0u32;
        let mut bus = VecBus { mem: vec![0; 64] };
        assert_eq!(
            step(&mut bus, &mut regs, &mut pc, inst),
            StepResult::Continue
        );
        regs
    }

    /// Verifies `addi`, including writes to `x0` being discarded.
    #[test]
    fn addi_and_x0_hardwired() {
        let regs = exec(asm::addi(1, 0, 42), |_| {});
        assert_eq!(regs[1], 42);
        let regs = exec(asm::addi(0, 0, 42), |_| {});
        assert_eq!(regs[0], 0); // x0 stays zero
    }

    /// Verifies `add` and `sub`, including two's-complement wraparound.
    #[test]
    fn add_and_sub() {
        let regs = exec(asm::add(3, 1, 2), |r| {
            r[1] = 100;
            r[2] = 23;
        });
        assert_eq!(regs[3], 123);
        let regs = exec(asm::sub(3, 1, 2), |r| {
            r[1] = 5;
            r[2] = 8;
        });
        assert_eq!(regs[3], (-3i32) as u32);
    }

    /// Verifies `xor` and `slli`.
    #[test]
    fn xor_and_shift() {
        let regs = exec(asm::xor(3, 1, 2), |r| {
            r[1] = 0xff00;
            r[2] = 0x0ff0;
        });
        assert_eq!(regs[3], 0xf0f0);
        let regs = exec(asm::slli(3, 1, 4), |r| r[1] = 1);
        assert_eq!(regs[3], 16);
    }

    /// Verifies `lui` places the immediate in the upper 20 bits.
    #[test]
    fn lui_upper_immediate() {
        let regs = exec(asm::lui(1, 0xABCD_E000), |_| {});
        assert_eq!(regs[1], 0xABCD_E000);
    }

    /// Verifies a taken `blt` redirects the PC by the (negative) branch offset.
    #[test]
    fn blt_taken_branches() {
        let mut regs = [0u32; NUM_REGS];
        regs[1] = 1;
        regs[2] = 10;
        let mut pc = 20u32;
        let mut bus = VecBus { mem: vec![0; 64] };
        // blt x1, x2, -8 : 1 < 10 ⇒ taken ⇒ pc = 12.
        step(&mut bus, &mut regs, &mut pc, asm::blt(1, 2, -8));
        assert_eq!(pc, 12);
    }

    /// Verifies `jal` links the return address and jumps.
    #[test]
    fn jal_links_and_jumps() {
        let mut regs = [0u32; NUM_REGS];
        let mut pc = 100u32;
        let mut bus = VecBus { mem: vec![0; 64] };
        step(&mut bus, &mut regs, &mut pc, asm::jal(1, 16));
        assert_eq!(regs[1], 104); // return address = pc + 4
        assert_eq!(pc, 116); // jump target = pc + 16
    }

    /// Verifies a `sw` then `lw` round-trips a word through the bus.
    #[test]
    fn store_then_load_word() {
        let mut regs = [0u32; NUM_REGS];
        regs[1] = 0xDEAD_BEEF; // value
        regs[2] = 0; // base
        let mut pc = 0u32;
        let mut bus = VecBus { mem: vec![0; 64] };
        step(&mut bus, &mut regs, &mut pc, asm::sw(1, 8, 2));
        let mut pc2 = 0u32;
        step(&mut bus, &mut regs, &mut pc2, asm::lw(3, 8, 2));
        assert_eq!(regs[3], 0xDEAD_BEEF);
    }

    /// Verifies `ecall` halts.
    #[test]
    fn ecall_halts() {
        let mut regs = [0u32; NUM_REGS];
        let mut pc = 0u32;
        let mut bus = VecBus { mem: vec![0; 64] };
        assert_eq!(
            step(&mut bus, &mut regs, &mut pc, asm::ecall()),
            StepResult::Halt
        );
    }

    /// Runs the bundled `sum 1..=n` program on the in-memory bus and checks the
    /// stored result (1 + 2 + … + 10 = 55).
    #[test]
    fn sum_program_pure() {
        let result_addr = 0x100u32;
        let code = super::program_sum_1_to_n(10, result_addr);
        let mut mem = vec![0u8; 1024];
        mem[..code.len()].copy_from_slice(&code);
        let mut bus = VecBus { mem };

        let mut regs = [0u32; NUM_REGS];
        let mut pc = 0u32;
        // Bounded to avoid an accidental infinite loop in a broken build.
        for _ in 0..1000 {
            let inst = bus.read(pc, 4);
            if step(&mut bus, &mut regs, &mut pc, inst) == StepResult::Halt {
                break;
            }
        }
        assert_eq!(bus.read(result_addr, 4), 55);
    }
}
