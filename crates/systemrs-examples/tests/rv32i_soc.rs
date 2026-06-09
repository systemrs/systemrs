//! Integration test: the RV32IM SoC is deterministic end to end.
//!
//! The whole platform (hart → address-decoding interconnect → RAM + UART + CLINT timer,
//! with a level-sensitive timer interrupt) is built and run twice. With no external
//! nondeterminism, the two runs must agree byte-for-byte on every observable — UART
//! output, the firmware's interrupt counter, the final architectural state, and the
//! final simulation time. This is the `--verify-determinism` property the in-tree ISS
//! gives trivially, and the golden reference any out-of-process QEMU bridge would later
//! be diffed against.

use systemrs::prelude::*;
use systemrs_examples::rv32i::HartState;
use systemrs_examples::soc::build_soc;

/// One observable snapshot of a finished run.
#[derive(Debug, PartialEq, Eq)]
struct RunResult {
    /// The bytes the firmware transmitted on the UART.
    uart: Vec<u8>,

    /// The firmware's timer-interrupt counter (in RAM).
    interrupts: u32,

    /// The hart's final architectural state.
    state: HartState,

    /// The simulation time at the end of the run, in picoseconds.
    end_time_ps: u64,
}

/// Builds the SoC, runs it to `end`, and captures the observable result.
fn run_to(end: SimTime) -> RunResult {
    let sim = Sim::new();
    let soc = build_soc(&sim);
    sim.run_until(end);
    RunResult {
        uart: soc.uart_out.borrow().clone(),
        interrupts: soc.interrupt_count(),
        state: *soc.state.borrow(),
        end_time_ps: sim.now().units(),
    }
}

/// The firmware prints the expected string and takes timer interrupts.
#[test]
fn soc_produces_expected_output() {
    let result = run_to(SimTime::from_us(20));
    assert_eq!(result.uart, b"Hi\n");
    assert!(
        result.interrupts >= 1,
        "expected at least one timer interrupt, got {}",
        result.interrupts
    );
}

/// Two independent runs of the SoC are byte-identical across every observable
/// (UART output, interrupt count, final hart state, and final sim time).
#[test]
fn soc_is_deterministic_across_runs() {
    let a = run_to(SimTime::from_us(20));
    let b = run_to(SimTime::from_us(20));
    assert_eq!(a, b, "two SoC runs diverged — determinism is broken");
}
