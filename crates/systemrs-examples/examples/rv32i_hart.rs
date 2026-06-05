//! Runnable example: a basic RV32I CPU hart.
//!
//! Run with `cargo run --example rv32i_hart`.
//!
//! A hart `SC_THREAD` executes a small program that sums `1..=10` and stores the
//! result to memory. Every instruction fetch, load, and store is a `b_transport`
//! to a memory target whose modelled latency is realized by `wait()` *inside*
//! `b_transport` — so the hart's coroutine yields from deep in the transport call.

use systemrs::prelude::*;
use systemrs_examples::rv32i;

/// The word address the program stores its result at.
const RESULT_ADDR: u32 = 0x100;

/// Builds the platform (hart + memory), loads the program, and runs it.
fn main() {
    let sim = Sim::new();

    // A 4 KiB memory with 2 ns access latency (modelled via wait()).
    let mem = Memory::new(4096, SimTime::from_ns(2));
    let target = TargetSocket::new(&sim, "mem.socket");
    mem.connect(&sim, &target);

    // The hart's initiator socket, bound to the memory.
    let isock = InitiatorSocket::new(&sim, "hart.isock");
    isock.bind(&sim, &target);

    // Load the program (sum 1..=10) at address 0 and start the hart there.
    let code = rv32i::program_sum_1_to_n(10, RESULT_ADDR);
    mem.load(0, &code);
    rv32i::build_hart(&sim, isock, 0, SimTime::from_ns(1));

    println!("Running RV32I hart: sum(1..=10), result stored at {RESULT_ADDR:#x}.");
    sim.run_until(SimTime::from_us(1));

    let result = mem.read_u32(RESULT_ADDR as usize);
    println!("Hart halted at {}.", sim.now());
    println!("Result in memory[{RESULT_ADDR:#x}] = {result} (expected 55).");
    assert_eq!(result, 55, "RV32I hart produced the wrong sum");
}
