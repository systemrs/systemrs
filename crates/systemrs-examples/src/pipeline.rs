//! A partitionable pipeline, lowered to **both** a single Tier-0 kernel and a Tier-1
//! region partition from the *same* process bodies — the worked example for M7's
//! conservative, barrier-synchronous PDES (`doc/systemrs-design.md` §8a).
//!
//! `A` emits tokens, `B` transforms them, `C` records `(now, value)`. The three stages
//! are generic over the link traits, so they run unchanged whether the links are
//! in-kernel (Tier-0 [`LocalLink`](systemrs::LocalLink)) or cross-region (Tier-1
//! [`BoundaryLink`](systemrs::BoundaryLink)). A determinism check then asserts the two
//! lowerings produce a bit-identical trace — the product PDES exists to preserve.
//!
//! Run it with `cargo run --example pipeline`.

use std::sync::{Arc, Mutex};

use systemrs::pdes::{LinkReceiver, LinkSender, LocalHost, Orchestrator};
use systemrs::{Ctx, SimTime};

/// A recorded observation at the sink: `(time_units, value)`.
pub type Reading = (u64, u32);

/// The middle stage's transform — shared by both tiers so the model is identical.
fn transform(v: u32) -> u32 {
    v.wrapping_mul(2).wrapping_add(1)
}

// ANCHOR: stages
/// Stage A: emit `1..=count`, one token every `period`.
fn producer<S: LinkSender<u32>>(cx: &Ctx, out: S, period: SimTime, count: u32) {
    for i in 1..=count {
        cx.wait(period);
        out.send(cx, i);
    }
}

/// Stage B: receive, transform, forward.
fn middle<R: LinkReceiver<u32>, S: LinkSender<u32>>(cx: &Ctx, inp: R, out: S) {
    loop {
        let v = inp.recv(cx);
        out.send(cx, transform(v));
    }
}

/// Stage C: receive and record `(now, value)`.
fn sink<R: LinkReceiver<u32>>(cx: &Ctx, inp: R, trace: &Mutex<Vec<Reading>>) {
    loop {
        let v = inp.recv(cx);
        trace
            .lock()
            .expect("trace lock")
            .push((cx.now().units(), v));
    }
}
// ANCHOR_END: stages

/// Parameters for the pipeline (shared by both tiers so the model is identical).
#[derive(Clone, Copy)]
pub struct PipelineParams {
    /// The token emission period.
    pub period: SimTime,
    /// Number of tokens to emit.
    pub count: u32,
    /// The cross-stage link latency (the lookahead; must be `>= quantum` in Tier-1).
    pub latency: SimTime,
    /// The run end time.
    pub end: SimTime,
}

/// Default demo parameters for `quantum`: 5 tokens with `period == latency == quantum`
/// (the tightest lookahead), run long enough to drain.
#[must_use]
pub fn demo_params(quantum: SimTime) -> PipelineParams {
    PipelineParams {
        period: quantum,
        count: 5,
        latency: quantum,
        end: SimTime::from_ns(200),
    }
}

/// Runs the pipeline as a single Tier-0 kernel (the golden reference) and returns the
/// sink's trace.
#[must_use]
pub fn run_tier0(p: PipelineParams) -> Vec<Reading> {
    let mut host = LocalHost::new();
    let link_ab = host.connect::<u32>(p.latency);
    let link_bc = host.connect::<u32>(p.latency);
    let trace = Arc::new(Mutex::new(Vec::new()));

    host.sim().add_thread("A", &[], true, move |cx| {
        producer(cx, link_ab, p.period, p.count);
    });
    host.sim()
        .add_thread("B", &[], true, move |cx| middle(cx, link_ab, link_bc));
    let t = Arc::clone(&trace);
    host.sim()
        .add_thread("C", &[], true, move |cx| sink(cx, link_bc, &t));

    host.run_until(p.end);
    trace.lock().expect("trace lock").clone()
}

// ANCHOR: tier1
/// Runs the pipeline partitioned across three Tier-1 regions (`A | B | C`) and returns
/// the sink's trace. Identical to [`run_tier0`] for any `quantum <= latency`.
#[must_use]
pub fn run_tier1(p: PipelineParams, quantum: SimTime) -> Vec<Reading> {
    let mut b = Orchestrator::builder(quantum);
    let ra = b.add_region();
    let rb = b.add_region();
    let rc = b.add_region();
    let link_ab = b
        .connect::<u32>(ra, rb, p.latency)
        .expect("latency >= quantum");
    let link_bc = b
        .connect::<u32>(rb, rc, p.latency)
        .expect("latency >= quantum");

    let trace = Arc::new(Mutex::new(Vec::new()));
    b.region(ra).sim().add_thread("A", &[], true, move |cx| {
        producer(cx, link_ab, p.period, p.count);
    });
    b.region(rb)
        .sim()
        .add_thread("B", &[], true, move |cx| middle(cx, link_ab, link_bc));
    let t = Arc::clone(&trace);
    b.region(rc)
        .sim()
        .add_thread("C", &[], true, move |cx| sink(cx, link_bc, &t));

    let mut orch = b.build();
    orch.run(p.end);
    trace.lock().expect("trace lock").clone()
}
// ANCHOR_END: tier1

/// Same Tier-1 topology, but with the regions declared in the **reverse** order
/// (`C`, `B`, `A`) — so the `RegionId`s differ. The sink trace must be unchanged: the
/// deterministic exchange sort, not the declaration order, decides delivery order.
#[must_use]
pub fn run_tier1_reordered(p: PipelineParams, quantum: SimTime) -> Vec<Reading> {
    let mut b = Orchestrator::builder(quantum);
    let rc = b.add_region();
    let rb = b.add_region();
    let ra = b.add_region();
    let link_ab = b
        .connect::<u32>(ra, rb, p.latency)
        .expect("latency >= quantum");
    let link_bc = b
        .connect::<u32>(rb, rc, p.latency)
        .expect("latency >= quantum");

    let trace = Arc::new(Mutex::new(Vec::new()));
    b.region(ra).sim().add_thread("A", &[], true, move |cx| {
        producer(cx, link_ab, p.period, p.count);
    });
    b.region(rb)
        .sim()
        .add_thread("B", &[], true, move |cx| middle(cx, link_ab, link_bc));
    let t = Arc::clone(&trace);
    b.region(rc)
        .sim()
        .add_thread("C", &[], true, move |cx| sink(cx, link_bc, &t));

    let mut orch = b.build();
    orch.run(p.end);
    trace.lock().expect("trace lock").clone()
}
