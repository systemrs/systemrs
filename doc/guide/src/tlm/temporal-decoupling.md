# Temporal decoupling

Synchronising to the kernel on *every* transaction is slow: each access pays a context
switch even when nothing else needs to run. **Temporal decoupling** lets an initiator run
*ahead* of simulation time, accumulating a local time offset and only synchronising at a
*quantum* boundary. It is the single biggest performance lever in loosely-timed
modelling.

## The quantum keeper

A `QuantumKeeper` tracks a per-initiator local time. The initiator adds modelled latency
to it (`inc`), checks whether it has reached the next quantum boundary (`need_sync`), and
only then actually `wait`s to catch the kernel up (`sync`). Between syncs, the initiator
issues many transactions without yielding:

```rust,ignore
let mut qk = QuantumKeeper::new();
qk.start(cx);
loop {
    // ...issue a transaction, with delay added to local time...
    qk.inc(access_latency);
    if qk.need_sync(cx) {
        qk.sync(cx);     // the only call that actually waits / yields
    }
}
```

The global quantum length is a simulation-wide setting:

```rust,ignore
set_global_quantum(&sim, SimTime::from_us(1)); // sync at most once per simulated µs
```

A larger quantum is faster but coarser (initiators see each other's effects only at
boundaries); a smaller quantum is more accurate but slower. The arithmetic is grid-
aligned and integer-only, so the *number and timing of syncs is deterministic* — the
same model always produces the same trace, independent of the quantum's performance
effect.

## Adapters

Temporal decoupling lives naturally on the LT path, while AT models the detailed timing
between phases. The `LtToAtAdapter` and `AtToLtAdapter` bridge the two: an LT initiator
can drive an AT target through an adapter (which spins the four-phase handshake for it),
and vice versa. The [reverb](ex-reverb.md) processes a block of audio per quantum — a
temporal-decoupling pattern applied to streaming DSP.

> **Go deeper:** design report §3.11 (temporal decoupling & PEQ), §6d (the quantum
> keeper and adapters).
