# Install & Run

## Add the dependency

SystemRS is a Cargo workspace; the umbrella crate is `systemrs`, a *facade* that
re-exports the public API of the layered crates. Add it to your model crate:

```toml
[dependencies]
systemrs = "0.1"
```

Then bring the public API into scope with the prelude — almost every model starts this
way:

```rust
use systemrs::prelude::*;
```

The prelude gives you `Sim`, `Ctx`, `SimTime`, the channels (`Signal`, `Clock`,
`Fifo`), the TLM-2.0 types (`GenericPayload`, `InitiatorSocket`, `TargetSocket`,
`Memory`), the observability and digital-twin layers, and more. You rarely need to name
the individual `systemrs-*` crates; the [architecture chapter](../reference/architecture.md)
explains how they are layered.

## Edition and toolchain

SystemRS is **edition 2024** with a minimum supported Rust version of **1.90**. A
recent stable toolchain (`rustup update stable`) is all you need.

## Running the reference examples

The `systemrs-examples` crate ships five runnable models — used as the worked tutorials
throughout this guide. From a clone of the repository:

```bash
cargo run --example counter   # an enable-gated counter (signals + a clocked method)
cargo run --example rv32i_hart # an RV32I CPU over loosely-timed transport
cargo run --example dma        # a DMA engine over the AT four-phase protocol
cargo run --example reverb     # a fixed-point guitar reverb streamed over TLM
cargo run --example twin       # a real-time sensor twin with deterministic replay
```

Each prints a short trace of what it does. Reading the example source alongside its
chapter is the fastest way to learn the framework.

## Building this guide

This guide is itself part of the repository. With [`mdbook`](https://rust-lang.github.io/mdBook/)
and [`just`](https://github.com/casey/just) installed:

```bash
just book        # render the guide (and check every link + code include)
mdbook serve book # live-preview at http://localhost:3000
```

`just book` is part of the project's `just ci` gate, so the guide can never drift out
of sync with the code it includes.

> **Go deeper:** design report §10 (crate structure), §10.4 (the workspace skeleton).
