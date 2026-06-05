//! Cross-crate integration tests for the two reference examples, driven through
//! the `systemrs` facade exactly as a model author would.

use systemrs::prelude::*;
use systemrs_examples::{counter, rv32i};

/// The word address the RV32I program stores its result at.
const RESULT_ADDR: u32 = 0x100;

/// Verifies the counter increments once per clock period over many periods.
#[test]
fn counter_increments_each_period() {
    let sim = Sim::new();
    let counter = counter::build(&sim, SimTime::from_ns(10));

    // Posedges at 0,10,…,90 ns → 10 increments by 95 ns.
    sim.run_until(SimTime::from_ns(95));
    assert_eq!(counter.count.read(&sim.ctx()), 10);
}

/// Verifies the RV32I hart computes `sum(1..=10) = 55`, communicating the result
/// through the memory target via `b_transport`.
#[test]
fn rv32i_hart_computes_sum() {
    let sim = Sim::new();

    let mem = Memory::new(4096, SimTime::from_ns(2));
    let target = TargetSocket::new(&sim, "mem");
    mem.connect(&sim, &target);

    let isock = InitiatorSocket::new(&sim, "hart.isock");
    isock.bind(&sim, &target);

    mem.load(0, &rv32i::program_sum_1_to_n(10, RESULT_ADDR));
    rv32i::build_hart(&sim, isock, 0, SimTime::from_ns(1));

    sim.run_until(SimTime::from_us(100));
    assert_eq!(mem.read_u32(RESULT_ADDR as usize), 55);
    // The hart halted (ecall) before the deadline, so the run ended at starvation.
    assert!(sim.now() < SimTime::from_us(100));
}

/// Verifies a larger summation (`sum(1..=100) = 5050`) to exercise the loop and
/// the byte-accurate store/backdoor-read path more heavily.
#[test]
fn rv32i_hart_larger_sum() {
    let sim = Sim::new();

    let mem = Memory::new(4096, SimTime::ZERO); // zero latency: pure functional check
    let target = TargetSocket::new(&sim, "mem");
    mem.connect(&sim, &target);

    let isock = InitiatorSocket::new(&sim, "hart.isock");
    isock.bind(&sim, &target);

    mem.load(0, &rv32i::program_sum_1_to_n(100, RESULT_ADDR));
    rv32i::build_hart(&sim, isock, 0, SimTime::ZERO);

    sim.run_until(SimTime::from_us(100));
    assert_eq!(mem.read_u32(RESULT_ADDR as usize), 5050);
}
