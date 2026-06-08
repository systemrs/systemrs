# What a twin needs

A **digital twin** is a long-lived, observable, sometimes wall-clock-coupled, replayable
model of a real system. SystemC was built as a *batch* simulator — start, run to an end,
exit. A twin is a *service*, and it needs four things SystemC does not provide, all built
on the same deterministic core:

| Need | Why a batch simulator lacks it | SystemRS |
|---|---|---|
| **Real-time pacing** | A batch sim runs as fast as it can | A pacer throttles *time advance* to wall clock ([pacing](pacing.md)) |
| **External-input gating** | A batch sim *exits* when idle | The sim **parks** for external input instead of exiting ([external input](external-input.md)) |
| **Deterministic replay** | Implicit ordering, ambient randomness | Explicit tie-breaks + a seeded RNG + an input journal ([replay](replay.md)) |
| **Live telemetry** | In-process only | An off-thread telemetry plane ([tracing](../obs/tracing.md)) |

The single most important of these is **external-input gating without starvation exit**:
a twin sits idle, then reacts when the world sends it something — and idleness must mean
*wait*, not *stop*. That one change reaches into the kernel's run loop; the rest layer on
top.

Crucially, all of this preserves determinism. The pacer changes wall-clock timing, never
simulation results; the input journal plus the RNG seed reproduce a run byte-for-byte;
and with nothing attached, the run loop is byte-identical to a plain batch simulation.
The twin layer is the only place the otherwise `!Send` core crosses a thread boundary,
and it does so through exactly two audited primitives: an mpsc inbox and a stop signal.

The rest of this part takes the three subsystems in turn, then builds a sensor twin that
uses all of them.

> **Go deeper:** design report §6f (what twins need beyond SystemC), §8 (determinism &
> replay).
