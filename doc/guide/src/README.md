# The SystemRS Guide

**SystemRS** is a Rust framework for building **transaction-level models** and
**digital twins** of digital systems — a TLM-only equivalent of SystemC and TLM-2.0,
rebuilt on idiomatic Rust.

It faithfully ports SystemC's single-threaded, cooperative, three-phase delta-cycle
scheduler — the determinism contract on which all transaction-level behaviour rests —
and layers Rust on top: stackful coroutines for thread processes, an
arena-and-generational-id object store instead of a raw-pointer graph, sum types
instead of signed-integer conventions, and `Result` instead of thrown reports. It
deliberately drops the RTL-oriented machinery (resolved multi-driver signals, the
`sc_dt` datatype library, clocked threads) that a transaction-level tool does not need,
and it *adds* the subsystems a twin requires that SystemC lacks: real-time pacing,
external-input gating, deterministic replay, and an off-thread telemetry plane.

## Who this guide is for

Rust developers who want to model a digital system at transaction level — whether or
not you have used SystemC before. No prior discrete-event-simulation experience is
assumed; the [mental model](getting-started/mental-model.md) chapter builds it up from
scratch. If you *do* know SystemC, the [naming map](reference/naming-map.md) is your
fast path.

## How it is organised

The guide teaches by example, building one concept at a time and pulling real code from
SystemRS's reference models (every listing tagged "from the example source" is included
verbatim from a compiled, tested file):

- **Getting Started** — what SystemRS is, how to run it, and the mental model.
- **Core Concepts** — time, processes, events, channels, and module hierarchy.
- **Transaction-Level Modeling** — the generic payload, sockets, loosely- and
  approximately-timed transport, temporal decoupling, and custom datatypes.
- **Observability** — reporting, analysis ports, and tracing.
- **Digital Twins** — external input, real-time pacing, and deterministic replay.
- **Reference** — the architecture, the SystemC map, and the determinism guarantees.

Each chapter ends with a "go deeper" pointer into the authoritative
[design report](https://github.com/systemrs/systemrs/blob/master/doc/systemrs-design.md),
whose section numbers (`§…`) are cited throughout. The full API reference lives in the
rustdoc (`cargo doc --open`).
