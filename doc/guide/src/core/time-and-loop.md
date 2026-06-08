# Time and the simulation loop

## Simulation time

Time in SystemRS is a `SimTime` — a 64-bit count of resolution units, with integer-only
arithmetic so no floating-point accumulation can ever reorder the timeline. The default
resolution is one picosecond; the convenience constructors assume it (`SimTime::from_units` is the
explicit form for other resolutions):

```rust
# use systemrs::prelude::*;
assert_eq!(SimTime::from_us(1), SimTime::from_ns(1_000)); // 1 µs = 1000 ns
assert_eq!(SimTime::from_ns(5) + SimTime::from_ns(3), SimTime::from_ns(8));
assert!(SimTime::from_ns(5) > SimTime::from_ns(3));
```

`SimTime::ZERO` is the start of time and `SimTime::INF` is "forever" (used to run a
long-lived twin until it is stopped).

## Building, then running

A `Sim` has two lives. During **elaboration** you construct everything — channels,
processes, modules, services. Then `run_until` flips it into the **running** phase,
where the static structure is immutable. (This is the runtime-checked analogue of a
`Building → Running` typestate; the [modules chapter](modules.md) shows the compile-time
version.)

```rust
# use systemrs::prelude::*;
let sim = Sim::new();
sim.add_thread("worker", &[], true, |cx| {
    cx.wait(SimTime::from_ns(10));
});
sim.run_until(SimTime::from_us(1));
assert_eq!(sim.now(), SimTime::from_ns(10)); // stopped at starvation, not the deadline
```

## The loop, and starvation

`run_until(end)` runs delta cycles at the current instant until nothing is runnable,
then advances time to the next scheduled event, and repeats — until either time would
pass `end`, or there is **nothing left to do**. That second case is *starvation*: with
no runnable process and no pending event, a normal run **stops**, even before the
deadline. The example above ends at 10 ns (the worker's only `wait`), not at 1 µs.

This default — "stop when idle" — is exactly what a batch simulation wants. A digital
twin wants the opposite (park and wait for external input instead of exiting); that is a
policy you opt into, covered in [External input and parking](../twins/external-input.md).

You can inspect a finished run through the simulation: `sim.now()` is the time it
stopped, and `sim.ctx()` gives a `Ctx` for backdoor reads of channels and memories.

> **Go deeper:** design report §3.1 (kernel & scheduler), §6a (the scheduler core).
