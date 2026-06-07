# Plan — Milestone 6: Digital-twin layer

> Status: **✅ COMPLETE** (2026-06-07) — all three exit criteria proven, full skill
> sweep clean (`cargo test` **123 green** + doctests), M0–M5 (rv32i/platform/AT)
> bit-identical when nothing is attached. The final roadmap MVP milestone
> ([systemrs-design.md](systemrs-design.md) §12). Design refs: §6f (twin needs), §8
> (determinism/replay). See [STATUS.md](../STATUS.md).
>
> Planned by a 5-phase design workflow (parallel readers → 3 drafts → synthesize →
> adversarial critique → finalize). The critique caught a headline correctness bug
> (delta injections dropped on the resume path) and several others; this plan is the
> **corrected** design that closed them (see *Critique fixes*).

## Scope

The subsystems a long-lived, observable, wall-clock-coupled twin needs that a batch
simulator lacks (§6f). **Snapshot/restore and structural hot-swap are deferred to
M7** (§12) — out of M6 scope.

1. **`RealTimePacer`** — paces wall clock to sim time at the kernel's time-advance
   hook (only time advance is paced; deltas stay instantaneous), with scale,
   tolerance, and slip telemetry.
2. **`ExternalInput` + suspend-on-starvation** — the critical twin feature: an mpsc
   inbox so an externally driven model **parks** (does not exit) when idle and resumes
   on injection.
3. **Seeded RNG service** — a deterministic SplitMix64 PRNG (no ambient `thread_rng`).
4. **Input journal + replay** — record injections + seed; replay byte-identically.

## Crate placement (the headline decision)

`systemrs-trace` depends on `systemrs-kernel`, so the kernel **cannot** depend on
trace (a cycle). Resolution:

- The **L1 kernel** gets only **two additive, no-op-when-unattached seams**: a
  *starvation gate* (consulted at `run_until`'s `None` arm under the new
  `Starvation::SuspendOnStarvation` policy) and a *time-advance hook* (fired in
  `do_timestep` before the advance commits), plus a `has_pending_delta()` accessor
  and `Ctx::resolution()`. None name trace, `std::time`, `std::thread`, or any twin
  type.
- A **new L6 crate `systemrs-twin`** (depends on kernel + time **only**) holds all
  concrete twin logic. Slip is exposed as a plain `Copy` `PacerStats` (never a
  `TraceEvent`), which dissolves the only reason a twin crate would need trace.
- RNG rides the existing `register_service`/`Ctx::service`; the journal handle is a
  plain `Rc<RefCell<Journal>>` read after the run.

## Exit criteria (all met)

- **E1** — paces to wall clock within tolerance + emits slip telemetry. →
  `twin::RealTimePacer` (`pacer_paces_to_wall_clock_and_reports_slip`: a 10 µs model
  takes ≈ 2 ms wall, sleeps, reports `advances`/`corrections`).
- **E2** — externally-driven model parks (not exits) when idle, resumes on injection.
  → `twin::attach_external_input` + the kernel gate
  (`external_input_parks_then_resumes_on_each_injection`: three delta injections wake
  the model exactly three times; `finite_run_until_with_input_attached_returns`).
- **E3** — journal + seed replays to a byte-identical transaction trace. →
  `twin::Rng`/`Journal`/`JournalReplayer`
  (`journal_and_seed_replay_byte_identically`, `seed_is_load_bearing`,
  `uninstrumented_model_yields_empty_trace`).

## Work items (as built)

| ID | Title | Result |
|---|---|---|
| M6-01..03 | Kernel seams: `GateOutcome` + `Starvation::SuspendOnStarvation`, starvation gate wired into `run_until` (Resume→`commit_and_notify` if delta pending; park only for `run_until(INF)`), time-advance hook in `do_timestep`, `has_pending_delta`, `Ctx::resolution` | `phase.rs`/`inner.rs`/`sim.rs`/`ctx.rs`; no-op when unattached |
| M6-04 | No-perturbation identity test | `twin_identity.rs` — deterministic + default starvation-EXIT preserved |
| M6-05..07,11,13 | `systemrs-twin` (L6): `Rng`, `ExternalInput`/`ChannelInput`/`ChannelInputSender`/`StopSignal`/`attach_external_input`, `RealTimePacer`/`PacerStats`, `Journal`/`JournalRecorder`/`JournalReplayer`, `TwinBuilder` | new crate; 5 unit tests |
| M6-15 | Facade re-exports + prelude | `systemrs` lib/prelude |
| M6-09,10,12,14 | EXIT tests (examples) | 7 integration tests (EC1/EC2/EC3 + guards) |

## Critique fixes (what changed from the draft)

- **Resume-path delta-drop bug (headline)** — a gate that injects via `cx.notify`
  (delta) arms `delta_events`, which only `commit_and_notify` drains; a bare re-crunch
  hits the empty-delta guard and drops the wake. **Fix:** the Resume arm runs one
  `commit_and_notify()` when `has_pending_delta()` before re-crunching. Proven by the
  three-wake EC2 test (which deliberately uses delta injection).
- **Parking vs `run_until(end)`** — park only for an unbounded `run_until(INF)` (the
  twin's service mode); a *finite* end exits on starvation (no deadlock waiting for
  input that can't advance time to `end`). Proven by the finite-end test.
- **Pacing math** — target wall-ns is computed from total femtoseconds
  (`units × fs_per_unit`), with `f64` used only to fold in `scale`, so sub-nanosecond
  resolutions don't round to zero. The EC1 floor assertion catches the rounding bug.
- **TxnRecord carried `delta`** — removed: per §6e the transaction record is *timed*,
  not delta-tagged. With it removed, a replayed run (whose injections arrive via a
  different scheduler path, so `delta_count` differs) is byte-identical on the
  transaction trace (time/command/address/length/response).
- **Vacuous EC3** — the model is *explicitly* instrumented with `record_transaction`;
  an `uninstrumented_model_yields_empty_trace` negative test rules out a vacuous pass,
  and `seed_is_load_bearing` proves the RNG seed actually drives the trace.
- **`!Send` core** — the kernel gate seam is a generic `Rc<dyn Fn(&Ctx)->GateOutcome>`
  closure; the kernel never names `ExternalInput`. Only `ChannelInputSender` +
  `StopSignal` (and `Send` payloads) cross threads. The `StopSignal` condvar is the
  second (and last) sanctioned cross-thread primitive, justified for OS-level wakeup +
  clean shutdown of a parked single-threaded sim.
- **Replay liveness** — the `JournalReplayer` spawns a real replay-driver `SC_THREAD`
  that waits to each record's sim time and injects, so the clock genuinely advances
  (not a tombstone event).

## Deliberately deferred (M7+)

- Snapshot/restore (bounded: arena columns + kernel queues + resumable-state-machine
  processes blocked at `wait` — not transparent native-coroutine-stack capture).
- Structural hot-swap; the optional parallel-region orchestrator (`--verify-determinism`).
- `tlm_fifo` generic put/get/peek (the analysis sublayer is M5's deliverable).
- Richer (non-`u64`) journaled input payloads; VCD/FST trace backends; `systemrs-ffi`.
