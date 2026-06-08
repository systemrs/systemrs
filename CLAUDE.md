# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this project is

SystemRS is a Rust re-implementation of SystemC's modeling substrate, **restricted to the
transaction-level modeling (TLM) subset** and oriented toward long-lived, observable digital
twins. It is **not** an RTL simulator: the `sc_dt` numeric/bit/fixed-point library, resolved
multi-driver signals, and clocked threads (`SC_CTHREAD`) are explicitly out of scope.

The authoritative specification is [doc/systemrs-design.md](doc/systemrs-design.md) — a detailed
design report. Read the relevant section before implementing a subsystem; the section numbers
below point into it. The C++ SystemC reference implementation is vendored (gitignored) at
[external/systemc/](external/) and is the ground truth for behavioural fidelity.

The codebase has not been scaffolded yet (no `Cargo.toml`/`crates/` exist as of this writing) —
the design doc §10 defines the structure to build.

## Apply the Rust skill

**For any Rust work in this repo, follow the project's Rust skill at
[.claude/skills/claude-skill-rust/SKILL.md](.claude/skills/claude-skill-rust/SKILL.md).** It is
the source of truth for code style, module layout, error handling, documentation, and the
build-verification sequence. In particular:

- Use the modern `<module_name>.rs` module style, **not** `mod.rs`. (The design doc's directory
  tree in §10.3 uses one `socket/mod.rs` — prefer `socket.rs` + `socket/` instead.)
- No `.unwrap()`/`.expect()` in non-test code; propagate with `Result` + `?`. Errors surface as
  typed `Result`, not panics — `sc_report(ERROR)` maps to `Result`, `FATAL` aborts (§7).
- All public items get rustdoc; functions document `# Arguments`/`# Returns`/`# Errors`/`# Panics`.
- **No change is complete until `just ci` passes** — it runs the skill's full build-verification
  sequence (see Commands below). While iterating, lean on the faster individual recipes
  (`just clippy`, `just test`); run the full `just ci` before considering a change done or committing.

## Commands

This is a Cargo **workspace** (resolver 3, edition 2024). Run from the repo root.

| Task | Command |
|---|---|
| Build (debug) | `cargo build` |
| Build (release, as CI does) | `cargo build --release` |
| Format / check | `cargo fmt` &middot; `cargo fmt --check` |
| Lint (warnings are errors) | `cargo clippy --all-targets -- -D warnings` |
| Test (whole workspace) | `cargo test` (or `cargo nextest run`) |
| Test a single crate | `cargo test -p systemrs-kernel` |
| Test a single test fn | `cargo test -p systemrs-examples delta_order` |
| Docs (incl. private items) | `cargo doc --no-deps --document-private-items` |
| License/advisory gate | `cargo deny check` |
| Security audit | `cargo audit` |
| Reproduce MSRV leg | `cargo +1.90 build` / `cargo +1.90 test` |

The skill's **build-verification order** after a change is: `fmt` → `fmt --check` →
`clippy -D warnings` → `test` → `build --release` → `doc` → `deny check` → `audit`. Fix failures
before moving on. `just ci` runs exactly this sequence (using `fmt --check`, not the in-place
`fmt`, and additionally running the examples) — see the task runner below.

`cargo-deny`, `cargo-audit`, and `cargo-nextest` are preinstalled in the devcontainer.

### Task runner (`just`) — the canonical CI entrypoint

A [`justfile`](justfile) at the repo root exposes the tasks above as recipes that each delegate to a
script in [`scripts/`](scripts/) (e.g. `just clippy` → `scripts/clippy.sh`). **`scripts/` is the
single source of truth for what every CI/quality command actually runs.** The GitHub Actions workflow
([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) mirrors it: the `quality` job runs `just ci`
and the `msrv` job runs `scripts/msrv.sh`. A green `just ci` locally therefore means a green CI.

Because of this, **CI tasks must invoke the commands defined in `scripts/`, never duplicate them**:

- **`just ci` (alias `just check`) is the gate every change must pass — a change is not done until
  it is green.** Run the full pass before finishing or committing. While iterating, the faster
  individual recipes — `just fmt-check`, `just clippy`, `just test`, `just build-release`,
  `just examples`, `just doc`, `just deny`, `just audit`, `just msrv`, … — each wrap exactly one
  script; run `just` to list them all.
- When you change *what* a check does, edit the script under `scripts/`; for a new check, add its
  script, a thin recipe that calls it, and a line in `scripts/ci.sh`. Do **not** add a command only
  to the GitHub workflow or only to a recipe — keep `scripts/` authoritative so local and CI stay
  identical. The MSRV (`scripts/msrv.sh`) is derived from `rust-version` in `Cargo.toml`, not pinned.

### SystemC co-simulation (`cosim` feature)

`systemrs-ffi` bridges to C++ SystemC via `cxx` over a de-templated C++ shim, built against
`external/systemc`. This path is **feature-gated** and needs a C++ toolchain + CMake (both present
in the devcontainer). It is off by default — only enable `cosim` when `external/systemc` is
present, e.g. `cargo test -p systemrs-examples --features cosim`. Do not enable
`rust-analyzer.cargo.features = "all"`, as that would try to build the FFI/SystemC path
unconditionally.

## Architecture — the load-bearing invariants

These constraints are *why* the design is the way it is. Violating them breaks the product
(deterministic, replayable, SystemC-faithful TLM). See design doc §§5–9.

- **One single-threaded, cooperatively non-preemptive scheduler owns time** (§6a). Exactly one
  process runs at a time. The strict **three-phase delta cycle (evaluate → update → notify)** is
  ported bit-for-bit, along with the immediate > delta > timed notification-collapse rules and
  `change_stamp`/`delta_count` accounting. Determinism is the product: tie-breaks C++ leaves
  implementation-defined are made explicit (insertion sequence numbers).

- **`SC_THREAD` = stackful coroutines (`corosensei`), NOT `async fn`** (§6a). This is the central
  technical bet: `wait()` must be callable from arbitrary call depth (deep inside `b_transport`,
  helpers, library code) without "async colouring" spreading across the whole TLM forward path.
  `SC_METHOD` is a plain run-to-completion `FnMut`. The `Ctx`/`Suspend` API is identical under an
  optional `cfg`-gated async backend (Wasm/no-fiber), so keep that seam clean.

- **Arena + generational-id ownership** (§6a, §6b, §9). Processes, events, channels, and objects live
  in kernel-owned arenas keyed by `Copy` generational ids (`ProcessId`, `EventId`, `ChannelId`,
  `ObjectId`). Components refer to each other **by id, never by reference** — this dissolves
  SystemC's raw-pointer graph and sidesteps the borrow checker. The synchronous core uses
  `Rc`/`RefCell` + `Cell`-based double-buffering, **never `Arc`/`Mutex`**. Core crates are
  intentionally `!Send`; the only `Send` boundary is the foreign-thread path, behind a feature.

- **TLM-2.0 contracts preserved, mechanisms modernized** (§6d, §7). The generic payload,
  four-phase FSM, `tlm_sync_enum` semantics, timing-annotation convention, DMI, sockets, and PEQ
  delta-parity ordering are kept faithful; their *implementation* (raw pointers, RTTI, intrusive
  refcounts, `void*` trampolines) becomes Rust ownership, sum types, `TypeId` maps, and arena
  handles. Pooled payloads → `Rc<RefCell<GenericPayload>>`.

- **Determinism is the unit; the quantum is the unit of parallelism** (§8). Build the
  single-threaded core first as the golden reference. Optional conservative (barrier-synchronous)
  PDES runs disjoint regions in parallel and re-converges at quantum boundaries; `rayon` handles
  embarrassingly-parallel telemetry/memory work *off* the critical path. The whole parallel trust
  boundary is a single audited `unsafe impl Send for RegionHandle {}`. Ship `--verify-determinism`
  from day one of any parallel tier.

- **Tracing samples after update commits** (§3.12, §6e). Stage callbacks fire at `PreTimestep`
  (and `PostUpdate` for delta tracing); trace through signal **handles** (Copy/clone snapshots),
  never a long-lived `&T` into a mutated signal. The primary sink is transaction-centric; VCD/FST
  are optional backends; telemetry I/O is pushed to a writer thread.

- **Interop is phased** (§11). Phase 1 (first deliverable): Rust models run as *guests inside the
  C++ SystemC kernel* via `cxx`. Phase 2: C++ guests inside the Rust kernel once it is bit-faithful.
  Phase 3: out-of-process quantum-synchronized co-sim. Two live kernels in one process is rejected.
  The Rust↔C++ panic/exception firewall is **symmetric** (§11.2, §11.6): Rust panics are caught at
  every Rust `extern "C"` entry (`catch_unwind` → `SC_REPORT_FATAL`) — an escaping panic can land on
  a *suspended coroutine frame* of another process — *and* the C++ shim must `try`/`catch (...)`
  around every `sc_wait`/`b_transport`/`nb_transport_*` that re-enters Rust. That C++→Rust direction
  is the "under-covered" one: a C++ throw originates *below* the Rust entry, so `catch_unwind` does
  not cover it.

## Crate structure (design doc §10)

A 14-crate workspace under `crates/`, layered acyclically (L0 lowest → L7 highest). The RTL
`sc_dt` datatypes library is deliberately **not** a crate.

- **L0 leaves:** `systemrs-diag` (reporting), `systemrs-time` (`SimTime`), `systemrs-runtime`
  (coroutine backend, `corosensei`), `systemrs-macros` (proc-macros; `proc-macro2`/`quote`/`syn`).
- **L1:** `systemrs-kernel` (scheduler, queues, events, processes, arenas, stage callbacks).
- **L2:** `systemrs-core` (`Module`/`Object`, elaboration, sensitivity, `wait`/`next_trigger`).
- **L3:** `systemrs-channels` (interfaces/ports/exports/binding; `Signal`/`Fifo`/`Clock`/…).
- **L4:** `systemrs-tlm1` (put/get/peek, analysis ports), `systemrs-tlm2` (GP+MM+extensions,
  transport, phases, DMI, sockets).
- **L5:** `systemrs-tlm-utils` (quantum keeper, PEQs, convenience sockets, LT↔AT adapters),
  `systemrs-trace` (sampling, recorders, VCD/FST).
- **L6:** `systemrs` (facade/prelude re-exporting the public API; depends on all except ffi/examples).
- **L7:** `systemrs-ffi` (C ABI / `cxx` SystemC interop), `systemrs-examples` (examples +
  cross-crate conformance/integration tests; dev-deps `insta`, `criterion`).

The proc-macro crate has no workspace deps and emits path-qualified `::systemrs::…` code so the
facade re-exports macros without a dependency cycle.

## Conventions

- **Edition 2024, resolver 3, MSRV 1.90** (verified in CI alongside current stable). License
  **Apache-2.0** (matching SystemC's lineage).
- **Naming:** internal crates are `systemrs-*`; the umbrella is just `systemrs`. Drop the `sc_`
  prefix on user-facing names (`Signal`, not `sc_signal`); the SystemC→SystemRS map is design
  doc §14.
- **Lints (workspace-level):** `clippy::all` + `clippy::pedantic` = warn; `missing_docs` = warn;
  `unsafe_code = "warn"`, allowed only in `systemrs-ffi`/`systemrs-runtime` and only with a
  `// SAFETY:` comment. `cargo-deny` gates licenses/advisories/duplicate versions.
- Shared deps go in `[workspace.dependencies]`; add new deps with `default-features = false` and
  only the features you need (see the Rust skill).

## Roadmap (design doc §12)

M0 time/events/delta loop → M1 process model → M2 modules/hierarchy/elaboration → M3 channels +
first LT transaction → M4 temporal decoupling/AT/PEQ → M5 observability → M6 digital-twin layer.
Milestone 0 is "the riskiest 200 lines" — get the delta loop bit-faithful first.
