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
For a tutorial, example-driven introduction, see the **user guide** in
[`doc/guide/`](doc/guide/) (`just book`, then `mdbook serve doc/guide` to read it
locally). Once GitHub Pages is enabled, the guide and the API reference publish to
`https://londey.github.io/systemrs/` (`just site` assembles that bundle locally). It is
**not** an RTL
simulator: the `sc_dt` numeric library, resolved multi-driver signals, and clocked
threads (`SC_CTHREAD`) are out of scope.

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

This is a Cargo **workspace** (resolver 3, edition 2024, MSRV 1.90). Everyday tasks
are wrapped as [`just`](https://just.systems) recipes — each a thin wrapper around a
script in [`scripts/`](scripts/) — so the same commands run locally and in CI. Run
`just` with no arguments to list them all. From the repo root:

| Command | What it does |
|---|---|
| `just build` | Debug build of the whole workspace and all targets. |
| `just test` | Run the whole test suite (`just test <name>` filters). |
| `just examples` | Run every example (`just examples counter dma` runs a subset). |
| `just clippy` | Lint all targets; warnings are errors. |
| `just fmt` &middot; `just fmt-check` | Format in place &middot; check formatting only. |
| `just doc` &middot; `just open-docs` | Build the API docs &middot; build and open them. |
| `just book` | Build the user guide (mdBook); render + link/include check. |
| `just deny` &middot; `just audit` | License/advisory gate &middot; security audit. |
| `just msrv` | Build and test on the MSRV (Rust 1.90, installed if missing). |
| `just ci` (alias `just check`) | **The full quality gate — run this before committing.** |

If you don't have `just` (`cargo install just`), the recipes are thin wrappers, so
you can run the underlying cargo commands directly — e.g. `cargo test --workspace`
or `cargo run --example counter`.

### Before committing

Run the full quality gate and make sure it passes:

```sh
just ci
```

`just ci` runs, in order: `fmt --check` → `clippy -D warnings` → `test` →
`build --release` → examples → `doc` → `book` → `deny check` → `audit` — the project's
build-verification sequence. The GitHub Actions workflow
([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) mirrors it, adding the MSRV
leg (`just msrv`), so **a green `just ci` locally means a green CI.**

The optional SystemC co-simulation path (`cosim` feature) is not yet wired up; it
will require `external/systemc` and a C++ toolchain (see the design doc §11).

## License

Apache-2.0, matching SystemC's lineage.
