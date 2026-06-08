# Determinism guarantees

Determinism is not a nice-to-have in SystemRS — it is the product. Every other feature
(replay, twins, trustworthy telemetry, future parallelism) rests on it. This chapter
collects the guarantees and what preserves them.

## What is guaranteed

Given the same model and the same inputs, a run produces the **same committed timeline** —
the same sequence of channel updates, event firings, and transactions, at the same
`(time, delta)` coordinates — every time, on any machine.

## What preserves it

- **The evaluate/update split.** Readers in a delta see the old value until the update
  phase commits, so the result never depends on the order the scheduler ran the writers.
- **Explicit tie-breaks.** Where SystemC leaves equal-time ordering implementation-
  defined, SystemRS pins it with monotonic insertion sequence numbers. Two events at the
  same time fire in the order they were scheduled.
- **No order-dependent iteration.** No `HashMap` iteration order ever feeds an observable
  result; service lookups are by key, never by iteration.
- **Integer-only time.** All time arithmetic is integer `SimTime` (saturating add), so no
  floating-point accumulation can reorder the timeline across runs or partitions.
- **Notification collapse.** The immediate > delta > timed collapse rules are reproduced
  exactly, so duplicate notifications cannot perturb ordering.
- **Seeded randomness, never ambient.** Twin randomness draws from a seeded `Rng`
  service, not `thread_rng`, so randomness is part of the reproducible state.

## What does *not* affect the result

- **Tracing.** Sampling is read-only and the stage hook is a no-op when unused; a traced
  run is byte-identical to an untraced one. ([tracing](../obs/tracing.md))
- **Real-time pacing.** The pacer changes only how fast *wall-clock* time passes, never
  simulation results. ([pacing](../twins/pacing.md))
- **Temporal-decoupling quantum length.** A larger quantum is faster and coarser, but the
  *number and timing of syncs is deterministic* for a given quantum. ([temporal
  decoupling](../tlm/temporal-decoupling.md))

## What you must not do

Determinism is a contract with two sides. Do not write a model that depends on
implementation-defined order (it has none to depend on), and draw all randomness from the
RNG service. Hold those, and you get byte-identical reruns — and `JournalReplayer` to
prove it.

> **Go deeper:** design report §5 (load-bearing principles), §8 (determinism), §8a
> (parallel execution and the tie-break order).
