//! Runs the partitionable pipeline as a single Tier-0 kernel and as a Tier-1 region
//! partition, and verifies their traces are bit-identical — deterministic conservative
//! PDES (`doc/systemrs-design.md` §8a).

use systemrs::{SimTime, assert_traces_match};
use systemrs_examples::pipeline::{demo_params, run_tier0, run_tier1};

fn main() {
    let quantum = SimTime::from_ns(10);
    let p = demo_params(quantum);

    let tier0 = run_tier0(p);
    let tier1 = run_tier1(p, quantum);

    println!("Tier-0 (single kernel): {tier0:?}");
    println!("Tier-1 (3 regions)    : {tier1:?}");

    match assert_traces_match(&tier0, &tier1) {
        Ok(()) => println!(
            "OK: Tier-0 and Tier-1 traces match ({} records) — same result, regardless of partition.",
            tier0.len()
        ),
        Err(e) => {
            eprintln!("DETERMINISM FAILURE: {e}");
            std::process::exit(1);
        }
    }
}
