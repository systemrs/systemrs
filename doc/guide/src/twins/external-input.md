# External input and parking

An ordinary run stops at starvation — no runnable process, no pending event, so nothing
left to do. A twin must instead **park**: block, wait for the world to send it something,
and resume. This is *suspend-on-starvation*, and it is the defining twin feature.

## The inbox

External input arrives on an mpsc inbox from a producer thread. `channel_input` gives you
the three pieces: the sim-side input, a `Send` sender for the producer, and a stop
signal:

```rust,ignore
// The injector turns each received value into simulation activity.
let (input, sender, stop) = channel_input::<u32, _>(move |cx, value| {
    cx.notify(sensor_ev);   // wake the model; never notify_now from here
});
attach_external_input(&sim, input, stop.clone());
```

`attach_external_input` sets the *suspend-on-starvation* policy and installs a gate. Now
`sim.run_until(SimTime::INF)` does not exit when the model goes idle — it parks. From
another OS thread, the producer sends values and finally stops:

```rust,ignore
let producer = std::thread::spawn(move || {
    for v in readings { sender.send(v).ok(); }
    stop.stop();           // clean shutdown
});
sim.run_until(SimTime::INF);
producer.join().ok();
```

Each `sender.send(v)` wakes the parked sim; the gate drains the inbox, the injector
fires `cx.notify(...)`, and the model resumes — processes the input — and parks again.
`stop.stop()` ends the run cleanly.

## Two rules

- **The core stays `!Send`.** Only the `ChannelInputSender` and the `StopSignal` cross to
  the producer thread; the model and its inbox receiver live entirely on the sim thread.
- **A finite `run_until(end)` still exits on starvation.** Parking is for the unbounded
  `run_until(SimTime::INF)` — the twin's long-lived service mode. A bounded run treats
  idleness as "done", so it never deadlocks waiting for input that cannot advance time to
  `end`.

> **Go deeper:** design report §6f (external-input gating, seeded in the kernel's
> `next_time`).
