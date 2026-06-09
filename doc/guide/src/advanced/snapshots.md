# Snapshot and restore

A long-running model — especially a [digital twin](../twins/needs.md) — sometimes needs
to be **checkpointed**: capture its state, then later resume from exactly that point
(fork a what-if branch, reproduce an incident, fast-forward and continue). SystemRS
supports a **bounded** snapshot/restore for this.

## What is captured, and what is not

A snapshot is taken at a **quiescent timestep boundary** — between `run_until` calls,
with nothing runnable and no pending update/delta work. `Sim::snapshot()` captures the
**kernel-visible scheduler state**: the determinism counters, the timed wheel, each
event's pending notification and its ordered subscriber lists, and each process's wait
state. It does **not** capture process bodies.

That last point is the whole "bounded" story. An `SC_THREAD` is a stackful coroutine;
its state lives on a native stack that cannot be portably serialized. So restore does not
resume a thread mid-body — it re-enters a process at its wait continuation on a freshly
rebuilt model. The model's *own* state (the values in its channels and services — the
"arena columns") is **yours** to save and restore around the snapshot. The kernel
checkpoints the scheduler; you checkpoint your serializable component state.

## What restores byte-identically

Because the rebuilt model's bodies are fresh, restore continues **bit-for-bit** when all
surviving model state lives in channels/services rather than in stack locals or closure
captures:

- **`SC_METHOD`s are ideal.** A method runs to completion on every trigger, so it has no
  "position" to lose — a fresh closure plus restored component state continues the
  original timeline exactly.
- **An `SC_THREAD` that holds live locals on its stack across a `wait` is out of scope**
  for this first cut (transparent native-stack capture is research-grade). A thread whose
  loop body is a pure function of restored channel/service state can be snapshottable; one
  that carries a half-finished computation across the `wait` cannot.

The practical rule: **to make a model snapshottable, keep its surviving state in channels
or services, not in closure captures or thread stack locals.** (This is the same
discipline the [spawned-body rule](../core/processes.md) already nudges you toward.)

## Doing it

`Sim::snapshot()` returns a `KernelSnapshot` (it errors if you are not at a quiescent
boundary). `Sim::restore()` applies it to a **freshly rebuilt** simulation — one
constructed with the *same sequence* of `alloc_event`/`add_method`/`add_thread`/channel
calls, so the generational ids line up — then you resume with `run_until`.

The `checkpoint` example is a self-clocking accumulator whose only state is a shared
`Cell` (its serializable component state):

```rust,ignore
{{#include ../../../../crates/systemrs-examples/src/checkpoint.rs:model}}
```

Snapshot it mid-run, restore onto a fresh rebuild, restore the model's `Cell`, and
continue — the trajectory past the split is identical to a straight run:

```rust,ignore
{{#include ../../../../crates/systemrs-examples/src/checkpoint.rs:checkpoint}}
```

Run it: `cargo run --example checkpoint`.

## Not yet (additive follow-ons)

The first cut is an in-memory checkpoint/restore, which is the load-bearing capture/apply
mechanism. Two extensions are additive: **automatic channel serialization** (a `Snapshot`
trait on every channel type, so the kernel captures channel values too rather than the
model doing it), and **on-disk persistence** (serializing a `KernelSnapshot` to bytes for
save-and-reload across runs or machines).

> **Go deeper:** design report §6f (what twins need beyond SystemC — the bounded snapshot
> model). Related: [processes](../core/processes.md) (why methods restore and stack-holding
> threads do not) and the [digital-twin layer](../twins/needs.md).
