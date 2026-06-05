# SystemRS

A Rust, **TLM-only** equivalent of SystemC for transaction-level digital twins.

SystemRS reproduces the parts of SystemC and TLM-2.0 needed to author digital
twins at transaction level, on a faithfully-ported single-threaded, cooperative,
**three-phase delta-cycle scheduler** — the determinism contract on which all TLM
behaviour rests. It layers idiomatic Rust on top: stackful coroutines for
`SC_THREAD`, an arena-and-generational-id object store instead of a raw-pointer
graph, sum types instead of signed-integer conventions, and `TypeId` maps instead
of RTTI.

The authoritative specification is [doc/systemrs-design.md](doc/systemrs-design.md).
It is **not** an RTL simulator: the `sc_dt` numeric library, resolved multi-driver
signals, and clocked threads (`SC_CTHREAD`) are out of scope.

## What is implemented

This workspace implements the deterministic core and the TLM-2.0 latency-timed
(LT) path needed to run the two reference examples — roughly milestones M0–M3 of
the design roadmap (§12), plus LT timing.

| Crate | Layer | What it provides |
|---|---|---|
| `systemrs-diag` | L0 | Reporting: severity, `Report`/`ReportError`, FATAL→abort. |
| `systemrs-time` | L0 | `SimTime` (integer u64 units), `Resolution`; `INF = u64::MAX`. |
| `systemrs-runtime` | L0 | Stackful coroutine backend (`corosensei`); depth-callable `suspend`. |
| `systemrs-kernel` | L1 | Three-phase delta cycle, events + notification collapse, processes, arenas, timed wheel, `Ctx`, the `Sim` builder/runner. |
| `systemrs-core` | L2 | Module/elaboration ergonomics: process & sensitivity builders, lifecycle trait. |
| `systemrs-channels` | L3 | `Signal`/`Buffer`/`Fifo`/`Clock` with the evaluate/update discipline. |
| `systemrs-tlm2` | L4 | Generic payload, sum-type command/response/sync, `TypeId` extensions, `Rc`+pool MM, sockets, a memory target. |
| `systemrs` | L6 | Facade + `prelude`. |
| `systemrs-examples` | L7 | The two examples + conformance/integration tests. |

The load-bearing invariants from the design are reproduced and tested:

- **Strict three-phase delta cycle** EVALUATE → UPDATE → DELTA-NOTIFY, with the
  empty-delta guard and the `change_stamp`/`delta_count` bump points.
- **Notification collapse** (immediate > delta > timed, earliest wins) and the
  verified `trigger()` subscriber ordering, plus the immediate self-notification
  guard.
- **Stackful `SC_THREAD`**: `wait()` is callable from arbitrary call depth —
  including from inside `b_transport` (demonstrated by the memory target modelling
  latency with `wait()`).
- **Arena + generational ids**: components refer to one another by `Copy` id;
  the synchronous core is `Rc`/`RefCell`/`Cell`, never `Arc`/`Mutex`.
- **Deterministic tie-breaks**: equal-time events ordered by insertion sequence.

Deferred per the roadmap (design §12, M4+): the AT four-phase non-blocking
protocol and PEQs, temporal-decoupling quantum keeper, TLM-1 analysis ports, the
tracing/VCD backends, the `#[module]` proc-macro, the SystemC `cxx` co-simulation
bridge, and the parallel (PDES) region orchestrator. The crate seams are designed
so these slot in without disturbing model-author code.

## Examples

### 1. Incrementing counter (`SC_METHOD` + clock + signal)

A `Clock` drives an `SC_METHOD` statically sensitive to its posedge; the method
increments a private count and writes it to an output `Signal`.

```sh
cargo run --example counter
```

### 2. Basic RV32I CPU hart (`SC_THREAD` + `b_transport`)

An `SC_THREAD` runs a fetch-decode-execute loop over the RV32I base integer
instruction set. **Every** memory access — fetch, load, store — is a `b_transport`
to a memory target over an initiator socket, with the modelled access latency
realized by `wait()` deep inside the transport call. The bundled program computes
`sum(1..=10) = 55` and stores it to memory.

```sh
cargo run --example rv32i_hart
```

The RV32I instruction semantics are decoupled from the kernel via a `Bus` trait,
so the ISA is unit-tested directly (see `crates/systemrs-examples/src/rv32i.rs`).

## Building and testing

This is a Cargo **workspace** (resolver 3, edition 2024, MSRV 1.90). From the repo
root:

```sh
cargo build                                   # debug build
cargo test --workspace                        # all tests
cargo run --example counter                   # run example 1
cargo run --example rv32i_hart                # run example 2
```

The full quality gate (matching the project's Rust skill):

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo build --release
cargo doc --no-deps --document-private-items
cargo deny check
cargo audit
```

The optional SystemC co-simulation path (`cosim` feature) is not yet wired up; it
will require `external/systemc` and a C++ toolchain (see the design doc §11).

## License

Apache-2.0, matching SystemC's lineage.
