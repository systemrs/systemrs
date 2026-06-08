# Summary

[Introduction](README.md)

# Getting Started

- [Why SystemRS](getting-started/why.md)
- [Install & Run](getting-started/install.md)
- [Quickstart: a clock and a counter](getting-started/quickstart.md)
- [The mental model](getting-started/mental-model.md)

# Core Concepts

- [Time and the simulation loop](core/time-and-loop.md)
- [Processes: METHOD vs THREAD](core/processes.md)
- [Events, notification, sensitivity](core/events.md)
- [Channels: signals, clocks, FIFOs](core/channels.md)
- [Modules, hierarchy, elaboration](core/modules.md)
- [Worked example: the gated counter](core/ex-counter.md)

# Transaction-Level Modeling

- [The generic payload and sockets](tlm/payload-and-sockets.md)
- [Loosely-timed transport](tlm/lt.md)
- [Worked example: an RV32I hart](tlm/ex-rv32i.md)
- [A platform: hierarchy and binding](tlm/ex-platform.md)
- [Approximately-timed transport and the PEQ](tlm/at.md)
- [Worked example: a DMA engine](tlm/ex-dma.md)
- [Temporal decoupling](tlm/temporal-decoupling.md)
- [Bring your own datatypes](tlm/datatypes.md)
- [Worked example: a fixed-point reverb](tlm/ex-reverb.md)

# Observability

- [Reporting](obs/reporting.md)
- [Analysis ports](obs/analysis-ports.md)
- [Tracing](obs/tracing.md)

# Digital Twins

- [What a twin needs](twins/needs.md)
- [External input and parking](twins/external-input.md)
- [Real-time pacing](twins/pacing.md)
- [Deterministic replay](twins/replay.md)
- [Worked example: a real-time sensor twin](twins/ex-twin.md)

# Reference

- [Architecture and crate map](reference/architecture.md)
- [SystemC → SystemRS map](reference/naming-map.md)
- [Determinism guarantees](reference/determinism.md)
- [Where to go next](reference/next.md)
