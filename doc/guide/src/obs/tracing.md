# Tracing

Where an analysis port is a tap you write to explicitly, **tracing** samples your model
for you. A `Tracer` registers a stage callback that fires *after each update phase
commits* and records the committed value of every signal you ask it to watch — sampling
through the signal's `Copy` handle, never a long-lived borrow into a value that is about
to change.

```rust
use systemrs_trace::{MemorySink, Tracer};
use systemrs_channels::Signal;
use systemrs_kernel::Sim;
use systemrs_time::SimTime;
use std::rc::Rc;

let sim = Sim::new();
let count: Signal<u32> = Signal::new(&sim, "count", 0);
let sink = MemorySink::new();
let tracer = Tracer::new(&sim, Rc::new(sink.clone()));
tracer.trace_signal(count, "count");   // keep `tracer` alive for the run

sim.add_thread("driver", &[], true, move |cx| {
    for i in 1..=3 {
        cx.wait(SimTime::from_ns(1));
        count.write(cx, i);
    }
});
sim.run_until(SimTime::from_ns(10));
assert!(sink.events().len() >= 3);      // the initial value plus each change
```

## Sinks

A tracer delivers `TraceEvent`s to a `TraceSink`:

- `MemorySink` collects them in-process (handy for tests and inspection).
- `WriterSink` hands them to an **off-thread** writer over a channel, so telemetry I/O
  never sits on the simulation hot path. It is the one place real concurrency exists,
  and it is flushed and joined deterministically at end-of-simulation.

For transactions, build a `TxnRecord` (a timed, transaction-centric record —
command, address, length, response) and emit it; the [reverb](../tlm/ex-reverb.md) and
[twin](../twins/ex-twin.md) record their activity this way.

## Telemetry on == telemetry off

The load-bearing guarantee: because sampling is read-only and the stage callback is a
true no-op when no tracer is attached, a *traced* run produces a byte-identical
`(now, delta)` trajectory to an untraced one. You can observe a model without changing
what it does — the precondition for trustworthy twin telemetry and deterministic replay.

> **Go deeper:** design report §3.12 (tracing), §6e (the observability plane).
