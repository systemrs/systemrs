//! M7 PDES exit criteria: a partitioned Tier-1 run reproduces the single-kernel Tier-0
//! run bit-for-bit, independent of partition/declaration order, with the lookahead
//! constraint enforced (`doc/systemrs-design.md` §8a, `doc/plan-m7.md` E1–E4).

use systemrs::pdes::{Orchestrator, PdesError};
use systemrs::{SimTime, assert_traces_match};
use systemrs_examples::pipeline::{
    PipelineParams, demo_params, run_tier0, run_tier1, run_tier1_reordered,
};

/// E1 — a partitioned model's trace is bit-identical to its single-kernel Tier-0 run.
#[test]
fn tier0_equals_tier1() {
    let quantum = SimTime::from_ns(10);
    let p = demo_params(quantum);
    let tier0 = run_tier0(p);
    let tier1 = run_tier1(p, quantum);

    assert!(!tier0.is_empty(), "the sink must record something");
    assert_eq!(
        tier0.len(),
        p.count as usize,
        "one record per emitted token"
    );
    assert_traces_match(&tier0, &tier1).expect("Tier-1 must match Tier-0 bit-for-bit");
}

/// E2 — the result is independent of the region declaration order (the deterministic
/// exchange sort, not insertion order, decides delivery order).
#[test]
fn region_order_independent() {
    let quantum = SimTime::from_ns(10);
    let p = demo_params(quantum);
    let forward = run_tier1(p, quantum);
    let reversed = run_tier1_reordered(p, quantum);
    assert_traces_match(&forward, &reversed)
        .expect("declaring regions in a different order must not change the result");
}

/// E4 — messages in flight across more than one quantum still match Tier-0.
#[test]
fn latency_above_quantum_multi_in_flight() {
    let quantum = SimTime::from_ns(10);
    // latency = 2 * quantum: with tokens emitted every quantum, a link carries two
    // messages in flight at once, so the ingress queue and self-pacing recv must hold
    // and order multiple pending deliveries.
    let p = PipelineParams {
        latency: SimTime::from_ns(20),
        ..demo_params(quantum)
    };
    let tier0 = run_tier0(p);
    let tier1 = run_tier1(p, quantum);
    assert_traces_match(&tier0, &tier1).expect("multi-quantum-in-flight Tier-1 must match Tier-0");
}

/// E3 — a cross-region link with latency below the quantum is rejected at construction.
#[test]
fn lookahead_violation_rejected() {
    let quantum = SimTime::from_ns(10);
    let mut builder = Orchestrator::builder(quantum);
    let src = builder.add_region();
    let dst = builder.add_region();
    let err = builder
        .connect::<u32>(src, dst, SimTime::from_ns(5))
        .expect_err("latency < quantum must be rejected");
    assert!(matches!(err, PdesError::LatencyBelowQuantum { .. }));
}
