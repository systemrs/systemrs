# Worked example: a real-time sensor twin

`cargo run --example twin` ties the whole digital-twin layer into one model: a
**sensor-monitoring twin** that sits parked until a reading is injected from outside,
wakes to process it (with seeded measurement noise), broadcasts the result for live
monitoring, and journals the injections so the run replays byte-identically.

The twin is one thread that parks on a sample event, then drains and processes each
queued reading. Included from the example source:

```rust,ignore
{{#include ../../../../crates/systemrs-examples/src/twin.rs:sensor}}
```

Every twin subsystem appears here:

- **External input + parking.** The thread `cx.wait_event(sample)`s when caught up — and
  because the model is attached with `attach_external_input`, that wait *parks* the sim
  for the next reading instead of exiting ([external input](external-input.md)).
- **Seeded RNG.** Each reading is perturbed by `Rng::from_ctx(cx).gen_range(...)`, so the
  processed value depends on the seed ([replay](replay.md)).
- **Observability.** The processed reading is broadcast on an `AnalysisPort` — the demo
  binds a printer, the tests bind a collector ([analysis ports](../obs/analysis-ports.md)).
- **Real-time pacing.** A `RealTimePacer` throttles the per-sample time advance, so a
  burst of queued readings is processed at a steady wall-clock cadence ([pacing](pacing.md)).

The runnable demo has two phases. **Live:** a producer thread streams sensor readings,
which the twin processes at the paced cadence, parking in between. **Replay:** the
recorded journal — with the same seed — reproduces the *exact* processed-value sequence,
with no live thread and no pacing. Running it prints the same values in both phases:
deterministic replay, demonstrated end to end. The tests add the guards that prove it is
real — park-then-resume on each injection, byte-identical replay, and a *different* seed
diverging.

This is the shape of a digital twin in SystemRS: a deterministic model, driven by the
outside world, paced to real time, observed without perturbation, and replayable on
demand.

> **Go deeper:** design report §6f (the twin layer), §8 (determinism). Full source:
> `crates/systemrs-examples/src/twin.rs`.
