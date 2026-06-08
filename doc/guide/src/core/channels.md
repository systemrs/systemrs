# Channels: signals, clocks, FIFOs

Channels are how processes communicate. They are kernel-owned, and they enforce the
evaluate/update discipline so that communication is deterministic.

## `Signal<T>` — a double-buffered value

A `Signal` holds a value of a `Copy` type. `write` *stages* a new value; `read` returns
the value committed at the previous update. The staged value becomes visible only in the
next delta — the rule from the [mental model](../getting-started/mental-model.md):

```rust
# use systemrs::prelude::*;
# let sim = Sim::new();
let sig: Signal<u32> = Signal::new(&sim, "s", 0);
sim.add_thread("driver", &[], true, move |cx| {
    assert_eq!(sig.read(cx), 0); // the initial value
    sig.write(cx, 7);
    assert_eq!(sig.read(cx), 0); // not yet — still the old committed value
    cx.wait(SimTime::from_ns(1)); // cross an update boundary
    assert_eq!(sig.read(cx), 7); // now committed
});
# sim.run_until(SimTime::from_ns(10));
```

A signal exposes `value_changed_event()` — fired the delta after a *changing* write — so
a process can be sensitive to it.

## `Clock` — a periodic event source

A `Clock` toggles on a period and offers `posedge_event()` (and the value via `read`).
It is the usual static-sensitivity source for clocked methods:

```rust
# use systemrs::prelude::*;
# let sim = Sim::new();
let clk = Clock::new(&sim, "clk", SimTime::from_ns(10));
# let _ = clk.posedge_event();
```

## `Fifo<T>` — a bounded blocking queue

A `Fifo` carries values between processes with back-pressure. `put`/`get` **block** the
calling *thread* until space/data is available; `try_put`/`try_get` are the non-blocking
forms. As with signals, a written value is readable only in the following delta.

```rust
# use systemrs::prelude::*;
# use std::sync::{Arc, Mutex};
# let sim = Sim::new();
let fifo: Fifo<u32> = Fifo::new(&sim, "f", 2); // capacity 2
let got = Arc::new(Mutex::new(Vec::new()));
let p = fifo;
sim.add_thread("producer", &[], true, move |cx| {
    for i in 0..3 { p.put(cx, i); } // put(2) blocks until the consumer drains
});
let g = Arc::clone(&got);
sim.add_thread("consumer", &[], true, move |cx| {
    for _ in 0..3 { g.lock().unwrap().push(fifo.get(cx)); }
});
sim.run_until(SimTime::from_ns(10));
assert_eq!(*got.lock().unwrap(), vec![0, 1, 2]);
```

`Buffer<T>` is the non-blocking, overwrite-on-write cousin when you want only the latest
value.

> **Go deeper:** design report §3.6 (primitive channels), §6c (events & channels).
