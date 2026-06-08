# Approximately-timed transport and the PEQ

Loosely-timed transport hides timing inside one blocking call. The **approximately-timed
(AT)** style exposes it: a transaction advances through a four-phase handshake, with
explicit timing between the phases, so you can model pipelining, contention, and
realistic latency.

## The four-phase handshake

A transaction walks a strict phase order over two calls:

- `nb_transport_fw` — the **forward** (request) path: the initiator drives `BEGIN_REQ`,
  later `END_RESP`.
- `nb_transport_bw` — the **backward** (response) path: the target drives `BEGIN_RESP`.

`BEGIN_REQ → END_REQ → BEGIN_RESP → END_RESP`. Each call returns a `TlmSync` telling the
caller what was done:

- `TlmSync::Accepted` — the callee is unchanged; await the opposite path.
- `TlmSync::Updated(phase)` — the callee advanced the phase synchronously; drive on from
  the new phase.
- `TlmSync::Completed` — the transaction ended early.

Unlike LT, the transaction is *shared* across phases as a `Txn` (an
`Rc<RefCell<GenericPayload>>` from a `TxnPool`), because both ends refer to the same
in-flight object as it progresses.

## The PEQ — timed responses

A target rarely responds instantly; it schedules `BEGIN_RESP` for one access-latency
later. That scheduling uses a **payload event queue (PEQ)** — `PhaseQueue` (callback) or
`PeqWithGet` (pull) — which fires the response at the right time with deterministic,
delta-accurate equal-time ordering. `AtMemory` is a ready AT target that services
reads/writes and returns `BEGIN_RESP` via a PEQ after its latency:

```rust,ignore
let mem = AtMemory::new(1024, SimTime::from_ns(2));
let target = TargetSocket::new(&sim, "mem");
mem.connect(&sim, &target);
let isock = InitiatorSocket::new(&sim, "dma");
isock.bind(&sim, &target);
```

## Driving it

An AT initiator drives `BEGIN_REQ` forward and completes the handshake from its backward
callback. Because writing that by hand is fiddly, SystemRS also ships ready-made
`LtToAtAdapter` and `AtToLtAdapter` to bridge an LT model to an AT one (and vice versa),
and convenience sockets that synthesise the missing direction. The
[DMA tutorial](ex-dma.md) shows an explicit hand-written AT engine; you will usually
reach for the adapters instead.

> **Go deeper:** design report §3.9 (transport & phases), §3.11 (temporal decoupling &
> PEQ), §6d (TLM-2 design).
