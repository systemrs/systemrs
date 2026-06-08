# Worked example: a DMA engine

`cargo run --example dma` is the AT protocol under load: a **DMA engine** that a CPU
programs over the loosely-timed path, which then copies a block of memory over the
approximately-timed four-phase handshake, raising a completion interrupt when done. It is
a two-master offload pattern — the CPU (LT) and the DMA (AT) are distinct initiators on
distinct paths.

## The backward path

The engine completes each transaction's handshake from its `nb_transport_bw` callback —
on `BEGIN_RESP` it drives `END_RESP` and wakes the copy loop:

```rust,ignore
{{#include ../../../../crates/systemrs-examples/src/dma.rs:nb-bw}}
```

## The copy engine

The engine itself is one `SC_THREAD`. It waits to be started (a register write from the
CPU notifies it), reads the descriptor from a service, then copies word by word — each
word an AT read followed by an AT write:

```rust,ignore
{{#include ../../../../crates/systemrs-examples/src/dma.rs:engine}}
```

A few patterns worth lifting:

- **`nb_transport_fw(BEGIN_REQ)` then `wait`.** `at_access` issues the request and
  suspends on the `done` event; the backward callback notifies it after the modelled
  latency. This is the AT handshake driven from a coroutine.
- **The descriptor is a service.** The thread body is `Send`, so it cannot capture the
  register state directly — it fetches `cx.service::<RefCell<DmaRegs>>()` at runtime,
  the [spawned-body rule](../core/processes.md) in action.
- **The interrupt is just an event.** `cx.notify(irq)` on completion; the CPU
  `wait_event(irq)`s for it. Modelling an interrupt needs nothing more.

The CPU programs the descriptor registers with ordinary LT writes (`b_transport` to the
control socket), writes the START register, and waits for the IRQ; the test then checks
that the block arrived at the destination and that time advanced by the modelled AT
latencies. The engine here is strictly sequential (one outstanding access); issuing
several requests before their responses — true AT pipelining — is the natural next step.

> **Go deeper:** design report §3.9, §3.11, §6d. Full source:
> `crates/systemrs-examples/src/dma.rs`.
