//! Runnable example: a register-programmed DMA engine over the AT protocol.
//!
//! Run with `cargo run --example dma`. A CPU programs the DMA's registers over LT,
//! starts it, and waits for the completion interrupt; the DMA copies a block through
//! the AT four-phase handshake to memory. The before/after destination and the time
//! the interrupt fires are printed.

use systemrs::AtMemory;
use systemrs::prelude::*;
use systemrs_examples::dma::{Dma, REG_DST, REG_LEN, REG_SRC, REG_START};

/// Source address, destination address, and word count for the demo copy.
const SRC: u32 = 0x100;
const DST: u32 = 0x200;
const WORDS: u32 = 8;

/// Builds a CPU + DMA + AT memory, runs a block copy, and prints the result.
fn main() {
    let sim = Sim::new();

    let mem = AtMemory::new(1024, SimTime::from_ns(2));
    let mem_target = TargetSocket::new(&sim, "mem");
    mem.connect(&sim, &mem_target);

    let dma_mem = InitiatorSocket::new(&sim, "dma.mem");
    dma_mem.bind(&sim, &mem_target);
    let ctrl = TargetSocket::new(&sim, "dma.ctrl");
    let irq = sim.alloc_event();
    Dma::build(&sim, &ctrl, dma_mem, irq);

    let cpu = InitiatorSocket::new(&sim, "cpu");
    cpu.bind(&sim, &ctrl);

    // Seed the source region with 0xA0, 0xA1, … so the copy is visible.
    let src_bytes: Vec<u8> = (0u32..WORDS * 4)
        .map(|i| 0xA0u8.wrapping_add(i.to_le_bytes()[0]))
        .collect();
    mem.load(SRC as usize, &src_bytes);

    println!("dst[0..8] before: {:02X?}", read_dst(&mem));

    sim.add_thread("cpu", &[], true, move |cx| {
        let prog = |reg: u64, val: u32| {
            let mut pay = GenericPayload::write(reg, val.to_le_bytes().to_vec());
            let mut delay = SimTime::ZERO;
            cpu.b_transport(cx, &mut pay, &mut delay);
        };
        prog(REG_SRC, SRC);
        prog(REG_DST, DST);
        prog(REG_LEN, WORDS);
        println!("[{}] CPU started the DMA ({WORDS} words)", cx.now());
        prog(REG_START, 1);
        cx.wait_event(irq);
        println!("[{}] CPU received the completion interrupt", cx.now());
    });

    sim.run_until(SimTime::from_us(10));

    println!("dst[0..8] after:  {:02X?}", read_dst(&mem));
    println!("Done at {}.", sim.now());
}

/// Reads the first eight destination bytes (backdoor) for display.
fn read_dst(mem: &AtMemory) -> Vec<u8> {
    (0..8).map(|i| mem.read_byte(DST as usize + i)).collect()
}
