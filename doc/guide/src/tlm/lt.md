# Loosely-timed transport

The **loosely-timed (LT)** coding style is the simplest and most common. An initiator
calls `b_transport` ("blocking transport"), passing a payload; the target services it
and returns. Timing is *loose*: the target models latency by `wait`ing inside the call,
and the whole transaction completes as one blocking call from the initiator's point of
view. This is the style you use to boot software fast.

Here is a full round-trip — write a word, read it back — over a bound socket:

```rust
# use systemrs::prelude::*;
let sim = Sim::new();
let mem = Memory::new(256, SimTime::from_ns(5));
let target = TargetSocket::new(&sim, "mem");
mem.connect(&sim, &target);

let isock = InitiatorSocket::new(&sim, "cpu");
isock.bind(&sim, &target);

sim.add_thread("cpu", &[], true, move |cx| {
    let mut delay = SimTime::ZERO;
    let mut wr = GenericPayload::write(0x40, 0xCAFEu32.to_le_bytes().to_vec());
    isock.b_transport(cx, &mut wr, &mut delay); // waits 5 ns inside
    let mut rd = GenericPayload::read(0x40, 4);
    isock.b_transport(cx, &mut rd, &mut delay);
    assert_eq!(u32::from_le_bytes(rd.data().try_into().unwrap()), 0xCAFE);
});
sim.run_until(SimTime::from_ns(100));
assert_eq!(mem.read_u32(0x40), 0xCAFE); // backdoor read, no modelled latency
```

Two things to notice:

- **`b_transport` blocks.** The `Memory` target `cx.wait`s for the access latency
  *inside* the call. That is why the initiator must be an `SC_THREAD`: the wait suspends
  the calling coroutine. This is SystemRS's central technical bet paying off — `wait`
  works from arbitrarily deep in the call stack, with no `async` colouring spreading up
  through your transport path. (A method may call `b_transport` only if the target never
  waits.)
- **`delay` is the timing annotation.** It accumulates modelled latency the initiator
  can choose to `wait` out (synchronising to the kernel) or carry forward
  ([temporal decoupling](temporal-decoupling.md)).

A custom target supplies its own behaviour with `register_b_transport`, which the
[DMA chapter](at.md) and the worked examples show. The next chapter builds a whole CPU
on this single primitive.

> **Go deeper:** design report §3.9 (transport interfaces & phases), §6d (TLM-2 API
> design).
