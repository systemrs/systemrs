# Events, notification, sensitivity

Processes do not poll; they are woken by **events**. An event is a kernel object
identified by a `Copy` `EventId`. A process becomes runnable when an event it is waiting
on (or statically sensitive to) *fires*.

## Allocating and notifying

Allocate an event during elaboration, then notify it from a process:

```rust
# use systemrs::prelude::*;
# let sim = Sim::new();
let ready = sim.alloc_event();
sim.add_thread("producer", &[], true, move |cx| {
    // ...produce something...
    cx.notify(ready);          // wake whoever waits on `ready`, next delta
});
sim.add_thread("consumer", &[], true, move |cx| {
    cx.wait_event(ready);      // suspend until `ready` fires
    // ...consume...
});
```

## Three flavours of notification, and how they collapse

SystemC distinguishes **immediate**, **delta**, and **timed** notifications, and
SystemRS reproduces the collapse rules exactly:

- An **immediate** notify makes sensitive processes runnable *within the current
  evaluate phase* (only valid there). It is the strongest.
- A **delta** notify (the default for `cx.notify`) fires at the start of the next delta
  cycle, after the update phase commits.
- A **timed** notify (`cx.notify_after(ev, dt)`) fires `dt` later.

When the same event is notified more than once before it fires, the *soonest* wins and
the rest are dropped — immediate beats delta beats timed. This collapse is part of what
makes equal-time behaviour deterministic; you do not have to reason about duplicate
notifications.

## Static vs dynamic sensitivity

- **Static** sensitivity is fixed at registration: the `&[events]` slice you pass to
  `add_method`/`add_thread`, or the `sensitive_to(ev)` builder call. The process is
  woken whenever any of those events fire, for the whole run.
- **Dynamic** sensitivity is a thread suspending on a *specific* event with
  `cx.wait_event(ev)` (or on time with `cx.wait(dt)`). Each `wait` chooses what wakes it
  next.

Channels expose events for you to be sensitive to — a `Signal`'s value-changed event, a
`Clock`'s posedge — which is the subject of the [next chapter](channels.md).

> **Go deeper:** design report §3.3 (events, notification & sensitivity), §6c (events &
> channels).
