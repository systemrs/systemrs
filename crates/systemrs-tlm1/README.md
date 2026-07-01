# systemrs-tlm1

TLM-1.0 for SystemRS: the analysis sublayer (ports/fifo/triple) for non-intrusive telemetry.

Part of **[SystemRS](https://github.com/londey/systemrs)** — a Rust re-implementation of SystemC's
modeling substrate, restricted to the transaction-level modeling (TLM) subset and oriented toward
long-lived, observable digital twins.

This is an internal layer of the workspace. Most users should depend on the umbrella crate
**[`systemrs`](https://crates.io/crates/systemrs)**, which re-exports the public API; this crate is
published so the facade (and advanced users) can depend on it directly. See the
[design report](https://github.com/londey/systemrs/blob/master/doc/systemrs-design.md) for the
architecture.

## License

Licensed under the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0)
(see `LICENSE-APACHE`).
