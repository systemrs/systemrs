# Plan — Milestone 7: advanced subsystems (PDES Tier-1 first)

> Status: **🚧 IN PROGRESS** — slice 1 (Parallel PDES Tier-1) ✅ and slice 2
> (bounded snapshot/restore) ✅ complete; slice 3 (SystemC FFI) not started.
> Roadmap context: §12 "Milestone 7+". Design refs: §8a (parallelization), §6f
> (snapshot), §11 (interop). See [STATUS.md](../STATUS.md).

## Scope

M7 is the deferred *bucket* of advanced subsystems (§12). They are independent and
large, so M7 lands as sequenced slices, each shippable on its own:

- **Slice 1 — Parallel PDES Tier-1 (this slice).** Conservative, barrier-synchronous
  parallel discrete-event simulation: disjoint **regions** each running their own
  single-threaded kernel up to a **quantum boundary**, with deterministic cross-region
  message exchange at the barrier (§8a). Determinism is the product: a Tier-1 run is
  bit-identical to the serial Tier-0 run of the same model with the same quantum +
  partition, independent of thread count.
- **Slice 2 — Snapshot/restore (bounded).** Checkpoint kernel state at a timestep
  boundary (all threads at `wait()`); serialize arena columns + queues + counters +
  the resumable wait-state, never coroutine stacks (§6f).
- **Slice 3 — `systemrs-ffi` (SystemC interop Phase 1).** Rust models as guests in the
  C++ SystemC kernel via `cxx` + a de-templated shim + the symmetric panic/exception
  firewall (§11). Heaviest; sequenced last.
- **Smaller deferred items** — full kill/reset throw semantics, `suspend`/`resume`,
  structural hot-swap beyond the `Rc<dyn FwTransport>` callback swap — folded in as
  they're needed.

**Deliberately deferred from slice 1:** optimistic/Time-Warp execution (rejected by
design); automatic partitioning (start with modeler-declared regions); cross-region DMI
(banned); a single-source-of-truth Tier-0/Tier-1 model harness (the example provides
both lowerings for now).

## Crate placement

A new **`systemrs-pdes`** crate (L7 orchestration wrapper) wraps N Tier-0 kernels; the
intra-region kernel is unchanged ("parallelism lives in orchestration", §8a). Deps:
`systemrs-kernel` + `systemrs-time`. `rayon` is an **optional** `[features]` switch, not
a correctness dependency — the orchestrator is correct and deterministic running regions
**sequentially**; the `rayon` feature only swaps the run-to-boundary and commit phases to
`par_iter_mut`. The whole parallel trust boundary is a **single audited
`unsafe impl Send for RegionHandle`**, compiled only under the `rayon` feature — so CI's
default determinism runs are `unsafe`-free.

The kernel gains exactly **one seam**: `Sim::schedule_event_at(ev, when)` — arm a timed
event at an absolute time, usable between `run_until` calls (the orchestrator injects a
cross-region delivery at its exact `deliver_at`). It reuses the existing
`arm_timed`/timed-wheel `(when, seq)` order, so an injected delivery orders against
intra-region timed events exactly as a native one.

## Mechanism

- A **`Region`** wraps a `Sim` + a region-local **outbox** (`RegionOutbox` service) and
  **ingress** (`RegionIngress` service: per-`LinkId` delivery queues). Built exactly like
  a Tier-0 model.
- A **`BoundaryLink<T>`** is a `Copy` handle (src/dst `RegionId`, `LinkId`, `latency`,
  arrival `EventId`) — `Copy` so a `Send` thread body can capture it; send/recv reach the
  outbox/ingress via `cx.service()`. `send(cx, v)` enqueues an **owned** message
  `{ deliver_at = now + latency, dst_region, dst_link, src_seq, payload }` into the
  source region's outbox. Latency is the conservative-PDES lookahead and **must be ≥ the
  quantum** (rejected at `connect` otherwise) — so a message sent in quantum *k* arrives
  at ≥ the next boundary and can never need delivery within quantum *k*.
- The **`Orchestrator`** loop, per quantum: (1) **PARALLEL** — every region
  `run_until(boundary)` (sequential, or `par_iter_mut` under `rayon`); (2) **EXCHANGE**
  (sequential, deterministic) — drain all outboxes, `sort_unstable_by_key((deliver_at,
  dst_region, dst_link, src_seq))`, route to per-region inboxes; (3) **COMMIT** — each
  region pushes owned payloads into its ingress and `schedule_event_at`s the arrival
  event at `deliver_at`. Advance the quantum until `boundary ≥ end`.
- `global_quantum_boundary(q, quantum) = (q+1)·quantum`, **integer-only** (`SimTime` is
  `u64`); no `f64` ever touches a committed boundary or the sort key.

## Exit criteria (proposed)

- **E1** — a partitioned model produces a transaction trace **bit-identical** to its
  single-kernel Tier-0 run with the same quantum + partition → `pdes_determinism::tier0_equals_tier1`.
- **E2** — the result is **independent of region declaration/iteration order** (the
  EXCHANGE sort, not insertion order, decides) → `region_order_independent`.
- **E3** — a cross-region link with `latency < quantum` is rejected at construction → `lookahead_violation_rejected`.
- **E4** — messages in flight across **more than one** quantum still match Tier-0 → `latency_above_quantum`.
- **E5** — the `rayon` parallel backend yields the **same** trace as the sequential one → `rayon_parity` (run under `--features rayon`).

## Work items

| ID | Title | Where |
|---|---|---|
| M7-01 | `Sim::schedule_event_at` kernel seam | `crates/systemrs-kernel/src/{sim,inner}.rs` |
| M7-02 | `systemrs-pdes` crate: ids, message + canonical sort, region IO services | `crates/systemrs-pdes/src/{ids,message,region_io}.rs` |
| M7-03 | `BoundaryLink<T>` (Copy; service-reached send/recv) + `Region` | `crates/systemrs-pdes/src/{link,region}.rs` |
| M7-04 | `Orchestrator` + builder + the 3-phase loop + `global_quantum_boundary` | `crates/systemrs-pdes/src/orchestrator.rs` |
| M7-05 | `RegionHandle` + the single audited `unsafe impl Send` (rayon-gated) | `crates/systemrs-pdes/src/handle.rs` |
| M7-06 | `verify` (`assert_traces_match`) + facade/prelude re-export + workspace wiring | crate `verify.rs`, `crates/systemrs/*`, root `Cargo.toml` |
| M7-07 | Pipeline example (Tier-0 + Tier-1 from shared transforms) + runnable bin + determinism tests | `crates/systemrs-examples/{src/pipeline.rs,examples/pipeline.rs,tests/pdes_determinism.rs}` |

**Critical path:** M7-01 → M7-02 → M7-03 → M7-04 → M7-06/M7-07 (E1). M7-05 (rayon) is
orthogonal (E5).

## The riskiest correctness point

Aligning each region's independent timeline with Tier-0's single timeline so traces
match. Defense: because `latency ≥ quantum`, a message produced in quantum *k* has
`deliver_at ≥ boundary(k)`, so the orchestrator injects it at COMMIT (before quantum
*k+1*) at the *same* absolute `deliver_at` the Tier-0 kernel's wheel would fire it, via
the *same* `(when, seq)` timed-wheel ordering. `run_until` advances `now` to each event's
time before running it, so a process always observes the correct `now` at send time even
after idle quanta. `--verify-determinism` (E1) is the executable proof; the example
exercises both `latency == quantum` (tightest) and `latency == 2·quantum` (in-flight
across a barrier).

---

## Slice 2 — bounded snapshot/restore (✅ complete)

**Mechanism (design §6f).** A `KernelSnapshot` captures the **kernel-visible** scheduler
state at a *quiescent timestep boundary* — the determinism counters
(`now`/`delta_count`/`change_stamp`/`seq`), the timed wheel, each event's pending
notification + *ordered* dynamic subscriber lists, and each process's wait state — **not**
the process bodies. Restore applies it to a **freshly rebuilt model** (same sequence of
`alloc_event`/`add_method`/`add_thread`/channel calls, so generational ids align), marks
the sim started (so the initialize pass is not re-run), and clears all transient
runnable/update/delta work. The kernel checkpoints the *scheduler*; the *model* checkpoints
its own serializable state (channels/services — the design's "arena columns") by saving it
before the snapshot and restoring it after.

**The bound (why it is "bounded").** Coroutine (`SC_THREAD`) stack frames cannot be
serialized (transparent native-stack capture is research-grade, §6f). So byte-identical
restore is guaranteed for **run-to-completion `SC_METHOD`s whose mutable state lives in
channels/services** (a fresh closure + restored component state continues the original
timeline exactly), while a thread holding live locals on its stack across a `wait` is out
of scope. The single kernel seam is `snapshot.rs` (`Inner::is_quiescent`/`capture`/`apply`)
+ `Sim::snapshot`/`Sim::restore`; no change to the crunch loop.

**Deferred (additive):** automatic channel serialization (a `Snapshot` trait on every
channel type) and on-disk persistence (serialize `KernelSnapshot` to bytes/text); the
first cut is an in-memory checkpoint/restore, which is the load-bearing capture/apply
mechanism.

**Exit criteria (met).**

- **S1** — a method-based model snapshotted mid-run and restored onto a fresh rebuild
  continues to a **byte-identical** trajectory → `systemrs-kernel` unit test
  `snapshot::tests::snapshot_restore_continues_byte_identical` + the `checkpoint` example's
  `snapshot_restore::checkpoint_continues_byte_identical`.
- **S2** — snapshot is gated to a quiescent boundary (errors otherwise) →
  `Sim::snapshot` returns `SYSTEMRS/SNAPSHOT`; `snapshot_is_quiescent_after_a_settled_run`.

**Files:** `crates/systemrs-kernel/src/snapshot.rs` (+ `Sim::snapshot`/`restore` in
`sim.rs`, `KernelSnapshot` export); `crates/systemrs-examples/src/checkpoint.rs` +
`examples/checkpoint.rs` + `tests/snapshot_restore.rs`; facade/prelude re-export.

## Verification

`just ci` green, plus `cargo test -p systemrs-pdes --features rayon` (E5). The
determinism tests (E1–E4) run on the `unsafe`-free default build; the `rayon` parity test
(E5) compiles and audits the single `unsafe impl Send`.
