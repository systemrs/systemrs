# Processes: methods and threads

Behaviour in SystemRS lives in **processes**. There are two kinds — *methods* and
*threads* — and choosing between them is the most common design decision you will make.

> **Coming from SystemC?** A method is what SystemC calls an `SC_METHOD`, and a thread is
> an `SC_THREAD`. SystemRS drops the macro names; the
> [naming map](../reference/naming-map.md) has the full translation.

## Methods — run to completion

A method is a plain `FnMut(&Ctx)` that runs from start to finish each time it is
triggered. It **cannot** `wait`. Use it for combinational logic, clocked register
updates, and short reactive handlers. Register it sensitive to one or more events:

```rust
# use systemrs::prelude::*;
# let sim = Sim::new();
# let clock = Clock::new(&sim, "clk", SimTime::from_ns(10));
# let count: Signal<u32> = Signal::new(&sim, "count", 0);
let mut n = 0u32;
sim.method("counter")
    .sensitive_to(clock.posedge_event())
    .dont_initialize()
    .finish(move |cx| {
        n += 1;
        count.write(cx, n);
    });
```

`dont_initialize()` skips the customary one-shot run at time 0. There is also a terse
form, `sim.add_method(name, &[events], initialize, body)`, when you do not need the
builder.

## Threads — stackful coroutines

A thread is a coroutine with its own stack. It **can** `cx.wait(...)` from *any* call
depth — even many frames deep inside a transport call. Use it for sequencers, CPUs,
drivers, DMA engines, and twins: anything that has its own flow of control over time.

```rust
# use systemrs::prelude::*;
# let sim = Sim::new();
sim.add_thread("driver", &[], true, move |cx| {
    loop {
        cx.wait(SimTime::from_ns(5));   // suspend for 5 ns, then resume here
        // ...do work, issue transactions, etc...
    }
});
```

The signature is `(name, static-sensitivity events, initialize?, body)`. The body is a
coroutine; when it `wait`s, the scheduler runs someone else and resumes the thread later
exactly where it left off — local variables intact.

> **The `Send` rule.** Thread bodies must be `Send + 'static`, so they may capture only
> `Copy` data (signals, sockets, `EventId`s) — never an `Rc`. To reach shared `!Send`
> state from a thread, register it as a service and fetch it at runtime with
> `cx.service::<T>()`. This matters once you build transactors and twins; the
> [`systemrs-modeling` skill] and the worked examples show the pattern.

## The `Ctx`

Both kinds of process receive a `Ctx` — the only handle into the running kernel:

- `cx.now()`, `cx.delta_count()` — the current time and delta.
- `cx.wait(dt)`, `cx.wait_event(ev)` — suspend a *thread* (a method may not call these).
- `cx.notify(ev)` — schedule an event (see the [events chapter](events.md)).
- `cx.service::<T>()` — fetch a registered shared service.

> **Go deeper:** design report §3.2 (processes & coroutines), §6a (the concurrency bet:
> stackful coroutines, not `async`).

[`systemrs-modeling` skill]: https://github.com/systemrs/systemrs-modeling-skill
