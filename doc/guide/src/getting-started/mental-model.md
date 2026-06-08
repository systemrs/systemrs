# The mental model

Four ideas explain almost everything about how a SystemRS model behaves. Hold these and
the rest follows.

## 1. One scheduler owns time, and runs one process at a time

SystemRS is **single-threaded and cooperative**. Exactly one process runs at any
instant; it runs until it finishes (a method) or voluntarily `wait`s (a thread). There
is no preemption and no in-model data race — which is *why* the core uses `Rc`/`RefCell`
rather than `Arc`/`Mutex`. The scheduler, not your code, decides who runs next.

## 2. The three-phase delta cycle

Simulation time advances in discrete steps, and within a single instant of time the
scheduler runs **delta cycles** in three strict phases:

1. **Evaluate** — runnable processes execute. When a process *writes* a channel
   (e.g. `signal.write`), the new value is **staged**, not yet visible.
2. **Update** — all staged channel writes commit at once.
3. **Notify** — events scheduled by those updates fire, making processes runnable for
   the next delta.

This evaluate/update split is the heart of determinism: because every reader in a delta
sees the *old* value until the update phase commits, the result does not depend on the
order the scheduler happened to run the writers. The practical consequence you will meet
constantly:

> A value written in one delta is not readable until the next. Cross a `wait` (or wait
> for the value-changed event) before expecting to read what you just wrote.

Time advances to the next scheduled event only when no more delta work remains at the
current instant.

## 3. Determinism is the product

Where SystemC leaves an ordering "implementation-defined", SystemRS pins it (equal-time
events break ties by an explicit insertion sequence number; no `HashMap` iteration order
ever feeds a result; all time arithmetic is integer). A run is therefore **reproducible
and replayable** — the foundation the [digital-twin layer](../twins/needs.md) builds on.
The price is small and worth it: don't write a model that depends on undefined order,
and you get byte-identical reruns for free.

## 4. Refer to things by id, not by reference

Processes, events, channels, and objects live in kernel-owned arenas, keyed by small
`Copy` generational ids. The handles you hold — `Signal<T>`, `Clock`, `EventId`, the
sockets — are those ids. You pass them by value into process closures and store them in
your structs; the kernel owns the underlying state. This is what dissolves SystemC's
raw-pointer graph, and it is why a process closure can `move`-capture a `Signal` without
borrowing anything.

---

With those four in hand, the [Core Concepts](../core/time-and-loop.md) part walks
through each piece — time, processes, events, channels, and hierarchy — and the
[gated counter](../core/ex-counter.md) ties them together.

> **Go deeper:** design report §5 (load-bearing principles), §6a (concurrency &
> scheduler core), §8 (determinism).
