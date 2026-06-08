# Deterministic replay

The payoff of a deterministic core is **replay**: record a run, then reproduce it
byte-for-byte later — no live producer thread, no wall-clock pacing, the same transaction
trace every time. This is forensic gold for a twin (reproduce a field incident on your
desk) and the basis of regression testing against a golden trace.

Two ingredients make a run reproducible: the **inputs** and any **randomness**.

## Seeded randomness

A twin must not draw from an ambient `thread_rng` — that would make replay impossible.
Instead, install a seeded `Rng` service and have all model randomness draw from it:

```rust,ignore
let rng = Rng::install(&sim, seed);   // a deterministic SplitMix64 service
// inside a process:
let n = Rng::from_ctx(cx).gen_range(0, 6);
```

The same seed always produces the same sequence — so the seed is *load-bearing*: replay
with a different seed and the trace diverges.

## Journaling and replay

Wrap the external input in a journal recorder, which records every injection (and the
seed) as the run proceeds:

```rust,ignore
// Record (live): journal the injections; attach the recorder as the external input.
let (recorder, sender, stop, journal) = journal_input(seed, injector);
attach_external_input(&sim, recorder, stop.clone());
// ...run, then snapshot the journal:
let recorded = journal.borrow().clone();

// Replay (a fresh sim, same seed): the journal drives the injections — no live thread.
let replay = JournalReplayer::new(recorded, injector);
replay.install(&sim2);
```

In replay mode there is no producer thread and no pacing: a replay-driver process walks
the journal, re-injecting each value at the sim time it was recorded at. With the same
seed restored, the model makes the same decisions and produces the **same processed-value
sequence** it did live. The [sensor twin](ex-twin.md) tests exactly this — live run
versus replay, byte-identical — plus the guard that a *different* seed diverges.

> **Go deeper:** design report §6f (journal + replay), §8 (determinism is
> non-negotiable for twins).
