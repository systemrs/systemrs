# systemrs

[![crates.io](https://img.shields.io/crates/v/systemrs.svg?logo=rust)](https://crates.io/crates/systemrs)
[![docs.rs](https://img.shields.io/docsrs/systemrs?logo=docsdotrs)](https://docs.rs/systemrs)
[![CI](https://github.com/systemrs/systemrs/actions/workflows/ci.yml/badge.svg)](https://github.com/systemrs/systemrs/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/crates/l/systemrs.svg)](https://www.apache.org/licenses/LICENSE-2.0)

**SystemRS** is a Rust re-implementation of SystemC's modeling substrate, restricted to the
transaction-level modeling (TLM) subset and oriented toward long-lived, observable digital twins.
It is *not* an RTL simulator.

This is the umbrella crate: it re-exports the public API of the whole SystemRS workspace — the
deterministic discrete-event kernel, module/elaboration ergonomics, primitive channels, the TLM-1.0
analysis sublayer and the full TLM-2.0 LT+AT path, tracing/telemetry, the digital-twin layer, and the
`#[module]` proc-macro. Most users only need this crate.

```sh
cargo add systemrs
```

## Features

- `pdes-rayon` — enable the optional `rayon`-backed parallel execution of the Tier-1 PDES
  orchestrator. Off by default; the orchestrator is correct and deterministic without it.

## Documentation

- **User guide** (tutorial, example-driven) and the rendered **API reference**, published to
  GitHub Pages: <https://systemrs.github.io/systemrs/>
- API docs for this crate on docs.rs: <https://docs.rs/systemrs>
- Design report:
  <https://github.com/systemrs/systemrs/blob/master/doc/systemrs-design.md>

## License

Licensed under the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0)
(see `LICENSE-APACHE`).
