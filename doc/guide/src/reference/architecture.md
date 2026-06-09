# Architecture and crate map

You program against the `systemrs` facade and its prelude, but underneath it is a stack
of small, acyclically-layered crates. Knowing the layering helps when reading rustdoc or
tracking down where a type lives.

| Layer | Crate(s) | What it provides |
|---|---|---|
| L0 | `systemrs-diag`, `systemrs-time`, `systemrs-runtime`, `systemrs-macros` | Reporting, `SimTime`, the coroutine backend, the `#[module]` proc-macro |
| L1 | `systemrs-kernel` | The scheduler, events, processes, arenas — `Sim`, `Ctx`, the ids |
| L2 | `systemrs-core` | Modules, elaboration, sensitivity, the hierarchy |
| L3 | `systemrs-channels` | `Signal`, `Clock`, `Fifo`, `Buffer`, ports/exports |
| L4 | `systemrs-tlm1`, `systemrs-tlm2` | Analysis ports; the generic payload, transport, sockets, `Memory` |
| L5 | `systemrs-tlm-utils`, `systemrs-trace` | Quantum keeper, PEQs, adapters; tracing & sinks |
| L6 | `systemrs-twin`, **`systemrs`** | The digital-twin layer; the **facade** that re-exports the public API |
| L7 | `systemrs-pdes`, `systemrs-examples` | The parallel-PDES orchestrator (optional `rayon`); the reference models and integration tests |

Two structural invariants are worth internalising:

- **The core is single-threaded and `!Send`.** From the kernel up through the TLM and
  observability layers, state is `Rc`/`RefCell` — one process runs at a time, so there is
  no in-model data race to guard. This is a *feature*: it is what makes the schedule
  deterministic and the arenas cheap.
- **`Send` boundaries are few and audited.** The single-threaded core never crosses
  threads. `systemrs-twin` (L6) is where external input and the off-thread telemetry
  writer do, through two audited primitives (an mpsc inbox, a stop signal). The optional
  [parallel PDES](../advanced/parallel.md) tier adds a second, *feature-gated* boundary —
  a single `unsafe impl Send for Region`, compiled only under `rayon` — so with parallelism
  off the core is entirely `Send`-free.

The proc-macro crate has no workspace dependencies and emits fully-qualified
`::systemrs::…` paths, so the facade can re-export the macros without a dependency cycle.
The deferred `systemrs-ffi` crate (SystemC interop) is the one piece of the design not
yet built.

> **Go deeper:** design report §10 (crate structure), §10.2 (the dependency graph), §8a
> (the `!Send` core and the single audited `Send` boundary).
