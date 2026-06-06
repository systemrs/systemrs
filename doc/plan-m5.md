# Plan — Milestone 5: Observability, reporting, tracing, TLM-1 analysis

> Status: **✅ COMPLETE** (2026-06-06) — all four exit criteria proven, full skill
> sweep clean (`cargo test` **111 green** + doctests), M0–M4 (rv32i/platform/AT)
> bit-identical. The roadmap phase after M4 ([systemrs-design.md](systemrs-design.md)
> §12). Design refs: §3.7 (analysis ports), §3.12 (reporting/tracing), §6e
> (observability), §10.1 (`systemrs-tlm1` L4, `systemrs-trace` L5). See
> [STATUS.md](../STATUS.md).
>
> Planned by a 5-phase design workflow (parallel readers → 3 drafts → synthesize →
> adversarial critique → finalize). The critique surfaced several real blockers in
> the draft; this plan is the **corrected** design that closed them (see *Critique
> fixes* below). The workflow's auto-generated final text was degenerate, so this
> document is authored from the critique + understanding + the as-built result.

## Scope

Make the deterministic core **observable** without perturbing it:

1. **Reporting** (`systemrs-diag`) — action/verbosity resolution with the exact
   SystemC precedence as a *pure* function over a golden default table; ERROR→`Result`,
   FATAL→abort. The existing `report_*` free fns stay byte-identical.
2. **TLM-1 analysis sublayer** (new `systemrs-tlm1`, L4) — `AnalysisPort` (synchronous
   in-order fan-out), `AnalysisFifo` (unbounded stream), `AnalysisTriple` (timestamped).
3. **Tracing** (new `systemrs-trace`, L5) — stage-callback sampling driven at
   `PostUpdate`/`PreTimestep`, a transaction-centric record sink, and an **off-thread**
   telemetry writer (the one real `Send` boundary).
4. **Kernel** — one additive primitive: the `PreTimestep`/`PostUpdate` stage-callback
   hook (a true no-op when unused), plus `end_of_sim` becoming a hook **list**.

## Exit criteria (all met)

- **E1** — fan-out `write()` reaches N subscribers synchronously, in registration order.
  → `tlm1::AnalysisPort` (`fan_out_in_registration_order`, capstone `scoreboard_and_stream_fan_out`).
- **E2** — the analysis fifo never back-pressures. → `tlm1::AnalysisFifo`
  (`unbounded_no_back_pressure`: 10 000 writes in one delta, all readable next delta).
- **E3** — report action precedence matches a golden table. → `diag::ReportHandler`
  (`golden_default_action_table`, `resolution_precedence`, `verbosity_gates_info`).
- **E4** — telemetry-on vs -off traces identical. → `trace::Tracer` + the no-op stage
  hook (`active_sink_is_schedule_identical`, capstone `telemetry_on_off_identical`):
  an actively-sampling tracer leaves the `(now, delta_count)` trajectory byte-identical.

## Work items (as built)

| ID | Title | Result |
|---|---|---|
| M5-A | diag: `ActionFlags`/`Verbosity` + golden table + pure `ReportHandler::resolve` + `emit` | `action.rs`, `handler.rs`; free fns untouched; 5 tests (EC3) |
| M5-B | kernel: stage-hook `Vec` (no-op when empty) + `end_of_sim` `Vec` + wire `PreTimestep`/`PostUpdate` | `inner.rs`/`sim.rs`; `add_stage_hook`/`add_end_of_sim_hook`; `stage_hooks_fire_at_boundaries`; M0-M4 bit-identical |
| M5-C | tlm1 (L4): `AnalysisPort` (re-entrancy-safe fan-out) + `AnalysisFifo` (unbounded) + `AnalysisTriple` | new crate; EC1 + EC2; 5 tests |
| M5-D | trace (L5): `TxnRecord`/`TraceEvent` (owned, no serde) + `MemorySink` + `Tracer` (PostUpdate sampling) + off-thread `WriterSink` (end-of-sim flush) + LT capture | new crate; EC4; 2 tests |
| M5-E | facade re-exports + capstone scoreboard/identity tests + full skill sweep + STATUS/plan | `systemrs` lib/prelude; `examples/tests/observability.rs` (2 tests) |

## Critique fixes (what changed from the draft)

The adversarial review caught these; the as-built design resolves each:

- **No serde** — `TxnRecord`/`TraceEvent` carry *owned* `Send` data (`String`, `u64`,
  local `TraceCommand`/`TraceResponse` enums; `length` via `u32::try_from`) and are
  formatted to text. This sidesteps the slotmap-`ObjectId`/tlm2-enum serde gap entirely
  and keeps **tlm2 unchanged**.
- **Writer flush seam** — `end_of_sim_hook` (a single `Option` owned by the elaboration
  driver) became `end_of_sim_hooks: Vec`, so the trace `WriterSink` registers its own
  flush/join hook without clobbering core's teardown. `WriterSink::attach` wires it;
  flush is idempotent (`RefCell<Option<…>>` take).
- **Re-entrancy-safe fan-out** — `AnalysisPort::write` snapshots live subscribers
  (upgrade + reap under one brief borrow) *then* iterates, so a subscriber may legally
  bind/unbind/write on the same port during delivery with no `RefCell` double-borrow
  (`reentrant_bind_during_write_is_safe`).
- **Active-sink identity test** — EC4 is proven with a *real, actively-sampling* sink
  (not just the empty-list fast path): the traced and untraced `(now, delta_count)`
  trajectories are asserted equal.
- **PostUpdate timestamp pinned** — sampling fires after values commit but before the
  value-changed delta-notify and before `delta_count` increments; samples are tagged
  with the committing delta. Documented at the firing site.
- **Drain-all-per-wake** — `AnalysisFifo::drain` empties all currently-readable items
  per wake (a flood of N becomes readable together in N+1), so the consumer never lags.
- **AT capture vs capstone** — transaction capture is **LT-only** for M5; the capstone
  scoreboard uses `AnalysisPort` fan-out (not AT phase accumulation), removing the
  contradiction.

## Deliberately deferred beyond M5

- General `tlm_fifo` put/get/peek message channel (bounded blocking FIFO already exists
  as `channels::Fifo`; the analysis sublayer is the M5 observability deliverable).
- VCD/FST value-change backends (the transaction-centric text sink is shipped).
- AT four-phase `phases[]` accumulation in transaction records (LT capture shipped).
- `sc_report` LOG-to-file, `stop_after`/STOP/INTERRUPT actions (modelled as bits, inert).
- `sc_vector` hierarchy-aware child arrays.
