//! Runnable example: an RV32IM hart driving a small SoC over a lightweight AXI bus.
//!
//! Run with `cargo run --example rv32i_soc`.
//!
//! A machine-mode hart `SC_THREAD` fetches and executes bare-metal firmware whose every
//! instruction fetch, load, and store is a `b_transport` routed by an address-decoding
//! [`systemrs::Interconnect`] to one of three targets: RAM, a UART, and a CLINT-like
//! timer. The firmware installs a trap handler, arms the machine timer, prints `"Hi\n"`
//! to the UART, then idles in `wfi`; each timer interrupt (a level-sensitive
//! [`systemrs::IrqLine`] sampled into `mip.MTIP`) vectors to the handler, which re-arms
//! the timer and bumps a counter in RAM.

use systemrs::prelude::*;
use systemrs_examples::soc::build_soc;

/// Builds the SoC, runs it for 20 µs, and reports the UART output and interrupt count.
fn main() {
    let sim = Sim::new();
    let soc = build_soc(&sim);

    println!("Running RV32IM SoC: hart → interconnect → {{RAM, UART, timer}}.");
    sim.run_until(SimTime::from_us(20));

    let text = String::from_utf8_lossy(&soc.uart_out.borrow()).into_owned();
    let pc = soc.state.borrow().pc;
    println!("UART output: {text:?}");
    println!("Timer interrupts taken: {}", soc.interrupt_count());
    println!("Hart idling at pc = {pc:#010x}, sim time = {}.", sim.now());

    assert_eq!(soc.uart_out.borrow().as_slice(), b"Hi\n");
    assert!(soc.interrupt_count() >= 1, "expected a timer interrupt");
}
