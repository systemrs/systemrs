# Quickstart: a clock and a counter

Here is a complete SystemRS model. It builds a clock, a counter process that increments
on each rising edge, and an output signal carrying the count — then runs it:

```rust
use systemrs::prelude::*;

let sim = Sim::new();
let count: Signal<u32> = Signal::new(&sim, "count", 0);
let clock = Clock::new(&sim, "clk", SimTime::from_ns(10));

let mut n = 0u32;
sim.method("counter")
    .sensitive_to(clock.posedge_event())
    .dont_initialize()
    .finish(move |cx| {
        n += 1;
        count.write(cx, n);
    });

sim.run_until(SimTime::from_ns(45)); // posedges at 0,10,20,30,40 ns
assert_eq!(count.read(&sim.ctx()), 5);
```

Line by line:

- **`Sim::new()`** creates the simulation — an elaboration-time builder. Everything is
  constructed *before* the run; the structure is frozen once it starts.
- **`Signal::new` / `Clock::new`** create channels in the kernel's arena and hand you
  back small `Copy` handles (`count`, `clock`). You pass these handles around by value;
  the kernel owns the state behind them.
- **`sim.method("counter")…`** registers a **method**: a run-to-completion callback,
  here `sensitive_to` the clock's rising edge and told *not* to run at time 0
  (`dont_initialize`). On each posedge it bumps a private `n` and `write`s it to the
  signal.
- **`sim.run_until(…)`** runs the scheduler. The clock fires posedges at 0, 10, 20, 30,
  and 40 ns; the method runs five times.
- **`count.read(&sim.ctx())`** reads the committed value afterward, through a `Ctx`.

That is the whole shape of a SystemRS model: build channels and processes during
elaboration, then run. The next chapter explains *why* it behaves the way it does — the
delta-cycle mental model that makes the result deterministic.

> This snippet is the verified `# Examples` doctest on the `systemrs` facade, so it
> compiles and runs under `cargo test`.

> **Go deeper:** design report §3.1 (kernel/scheduler), §6a (the scheduler core).
