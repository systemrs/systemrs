# Parallel execution

Everything so far has run on a single, cooperative scheduler — and that is the golden
reference. But a large model can also run **in parallel** without giving up determinism,
through SystemRS's conservative, barrier-synchronous **PDES** tier (parallel
discrete-event simulation, `systemrs-pdes`).

## The idea: regions synchronized at quantum boundaries

The unit of parallelism is a **region**: a disjoint subgraph that runs its *own*
single-threaded kernel — a full Tier-0 simulation — up to a **quantum boundary**. An
`Orchestrator` drives all regions through a three-phase loop, once per quantum:

1. **Run** — every region runs its delta/timed loop to the boundary, in parallel.
2. **Exchange** — drain each region's outbound messages, sort them by a canonical key,
   and route them to their destinations (sequential and deterministic).
3. **Commit** — each region injects the messages addressed to it, then the quantum
   advances.

Regions talk only through **`BoundaryLink`s**: typed, one-way, latency-bearing links.
The latency is the conservative-PDES *lookahead* and **must be ≥ the quantum** — so a
message sent during one quantum can never need delivery within that same quantum, which
is exactly what makes the parallel run safe to barrier.

## Determinism is preserved

The headline guarantee: a Tier-1 (parallel) run produces a result **bit-identical** to
the Tier-0 (serial) run of the same model with the same quantum and partition,
*independent of thread count or timing*. The cross-region exchange sorts by
`(deliver_at, dst_region, dst_link, src_seq)` — no address, hash order, or
finish-order on the path — and all time arithmetic is integer. Parallelism here is a
*performance* tier layered over the deterministic core, never a change to the result.

This is verifiable: build a model both as a single Tier-0 `LocalHost` and as a
partitioned `Orchestrator`, run each, and compare the traces with `assert_traces_match`
(the `--verify-determinism` discipline). The reference `pipeline` example does exactly
that.

## The shape of it

A three-stage pipeline `A → B → C`, partitioned into three regions connected by two
latency links, driven by the orchestrator — included from the tested example source:

```rust,ignore
{{#include ../../../../crates/systemrs-examples/src/pipeline.rs:tier1}}
```

The producer/consumer process bodies are written **once**, generic over the link traits
(`LinkSender`/`LinkReceiver`), so the *same* bodies run whether the links are in-kernel
(Tier-0) or cross-region (Tier-1) — which is what makes the determinism comparison
honest. Run it: `cargo run --example pipeline`.

## Going actually-parallel

By default the orchestrator runs the regions **sequentially** — and that is already
deterministic and correct. Real OS-thread parallelism is the optional **`rayon`
feature** (`systemrs-pdes/rayon`, surfaced as `systemrs/pdes-rayon`): it swaps the run
and commit phases to `rayon::par_iter_mut`. The result is identical either way, so the
determinism tests run on the rayon-free build. The entire parallel trust boundary is a
*single audited* `unsafe impl Send for Region`, compiled only under that feature — each
region is moved to exactly one worker, used exclusively, and shares no `Rc` with any
other region.

## When to reach for it

- **Partition along latency.** Cut only across links with non-zero latency ≥ the quantum;
  keep immediate-notification edges and shared clocks within one region.
- **Declare regions explicitly.** Start with modeler-declared partitions; automatic
  partitioning is future work.
- **The quantum is the knob.** Larger = faster and coarser; smaller = finer and slower.
  A Tier-1 run is bit-exact to Tier-0 *for a given quantum + partition*.
- DMI cannot cross a region boundary.

> **Go deeper:** design report §8a (parallelization). Related: [temporal
> decoupling](../tlm/temporal-decoupling.md) (the quantum is the same lookahead) and the
> [determinism guarantees](../reference/determinism.md).
