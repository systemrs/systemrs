//! Snapshots a self-clocking accumulator mid-run, restores it onto a fresh rebuild, and
//! verifies the continued trajectory is byte-identical to a straight run — M7 slice 2
//! bounded snapshot/restore (`doc/systemrs-design.md` §6f).

use systemrs::SimTime;
use systemrs_examples::checkpoint::{run_straight, run_with_checkpoint};

fn main() {
    let period = SimTime::from_ns(10);
    let step = 7;
    let split = SimTime::from_ns(45);
    let end = SimTime::from_ns(100);

    let reference = run_straight(period, step, end);
    let (before, after) = run_with_checkpoint(period, step, split, end);
    let mut restored = before;
    restored.extend(after);

    println!("straight  : {reference:?}");
    println!("checkpoint: {restored:?}");
    println!("(snapshot taken mid-run, then restored onto a fresh rebuild)");

    if restored == reference {
        println!(
            "OK: snapshot/restore continued byte-identically ({} samples).",
            reference.len()
        );
    } else {
        eprintln!("SNAPSHOT MISMATCH");
        std::process::exit(1);
    }
}
