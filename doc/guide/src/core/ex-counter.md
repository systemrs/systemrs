# Worked example: the gated counter

This ties the Core Concepts together in a model you can run with
`cargo run --example counter`. It is an **enable-gated counter**: it increments on a
clock rising edge, but *only while an external `enable` line is high* — counting an
impulse, not bare clock cycles. That makes it a clean illustration of synchronous,
edge-triggered, externally-gated logic.

Here is the model, included verbatim from the tested example source:

```rust,ignore
{{#include ../../../../crates/systemrs-examples/src/counter.rs:build}}
```

What each piece is doing:

- **Two signals and a clock.** `enable` is a `Signal<bool>` (the gate), `count` is a
  `Signal<u32>` (the output), and `clock` provides the timing. All three are `Copy`
  handles the method captures by value.
- **A clocked method.** The method is `sensitive_to` the clock's posedge and
  `dont_initialize`d, so it runs *only* on rising edges.
- **Sampling the gate.** On each edge it reads `enable`. Because of the evaluate/update
  discipline, this sees the value committed *before* this delta — exactly the registered,
  one-cycle-latency behaviour of real synchronous logic. It increments and writes `count`
  only when the gate is high.

A testbench drives `enable` over a window and checks that only the enabled edges are
counted — for example, raising it between the 0 ns and 10 ns edges and lowering it after
the 20 ns edge yields a count of 2 (the edges at 10 and 20 ns). With `enable` never
driven, the counter stays at zero.

> **Where the "impulse" comes from.** Here an in-sim stimulus drives `enable`. For a
> genuinely *external* impulse — a line toggled from outside the simulation — you would
> drive `enable` from an `ExternalInput`, which is exactly what the
> [sensor twin](../twins/ex-twin.md) does.

> **Go deeper:** design report §6a (the scheduler core), §6c (channels). Full source:
> `crates/systemrs-examples/src/counter.rs`.
