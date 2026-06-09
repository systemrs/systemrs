# SystemRS — Implementation Status

Tracks every feature in [doc/systemrs-design.md](doc/systemrs-design.md) against what is
actually built in `crates/`. Snapshot as of **2026-06-06**. The design's own
REPLICATE / SIMPLIFY / DEFER / DROP decisions (§4) are carried in the **Decision** column;
this file's **Status** column is the *as-built* reality.

> Regenerate by re-running the gap analysis against the design doc + `crates/` and editing
> the tables below. Keep it in sync as milestones land.

**Legend:** ✅ Done · 🟡 Partial · ⬜ Missing (planned, not built) · ⏸️ Deferred (intentionally post-MVP) · ❌ Dropped (explicitly out of scope, §4)

**Build health (2026-06-08):** `cargo test --workspace` → all passing (+ doctests incl. a multi-socket compile-fail and the PDES determinism suite); `fmt`/`clippy -D warnings`/`build --release`/`doc`/`book`/`deny`/`audit` all clean, plus a `pdes (rayon)` step that lints the single audited `unsafe` and runs the determinism tests with the parallel backend. **15 crates exist** (the new L7 `systemrs-pdes` added in M7 slice 1); `systemrs-ffi` is **deliberately not an in-tree crate** — SystemC interop is an out-of-tree bridge repo against this repo's pure-Rust `FwTransport` seam (M7 slice 3 packaging decision; revises §10).

**Feature tally (114 tracked, pre-M2 baseline):** ✅ 36 DONE  ·  🟡 17 PARTIAL  ·  ⬜ 39 MISSING  ·  ⏸️ 14 DEFERRED  ·  ❌ 8 DROPPED _(M2 rows below updated as Phase A lands)_

## Where we are

M0 (delta loop), M1 (process model), the M3 **LT TLM-2.0** slice, and **M2 (modules / hierarchy / ports / exports / elaboration)** are **complete and bit-faithful** ([doc/plan-m2.md](doc/plan-m2.md), all 7 M2 exit criteria proven). **M4 (temporal decoupling / AT protocol / PEQ / quantum keeper) has met all its exit criteria** — a new L5 crate `systemrs-tlm-utils` plus a single additive kernel primitive (`Ctx::spawn_thread`) and strictly-additive tlm2 socket extensions deliver: E1 quantum sync-on-grid (`QuantumKeeper`), E2 a full BEGIN_REQ→END_RESP exchange exercising all three `TlmSync` paths (the AT four-phase FSM over `nb_transport_fw/bw`), E3 LT→AT (`LtToAtAdapter`), E4 AT→LT (`AtToLtAdapter`, spawning a Send-safe per-transaction coroutine), and E5 PEQ delta-parity (`PeqWithGet`/`PhaseQueue`) — with the LT path (rv32i/platform) bit-identical throughout ([doc/plan-m4.md](doc/plan-m4.md)). The **M4 polish also landed**: convenience sockets with nb→b synthesis + multi/passthrough distinct-type stubs (M4-11), a SIMPLIFY DMI with the §3.9 re-entrancy guard (M4-12), and a reusable `AtMemory` (M4-13) — so **M4 is complete (14/14 work items)**. **M5 (observability / reporting / tracing / TLM-1 analysis) is complete** — two new crates (`systemrs-tlm1` L4: `AnalysisPort`/`AnalysisFifo`/`AnalysisTriple`; `systemrs-trace` L5: `Tracer`/`MemorySink`/`WriterSink`) plus reporting precedence in `systemrs-diag` (`ReportHandler` + golden `ActionFlags` table + `Verbosity`) and one additive kernel primitive (the `PreTimestep`/`PostUpdate` stage-callback hook, with `end_of_sim` now a hook **list**), all four exit criteria proven: E1 synchronous in-order fan-out (re-entrancy-safe `AnalysisPort`), E2 the unbounded analysis fifo never back-pressures (10k writes in one delta), E3 report action precedence matches the golden table, E4 telemetry-on == telemetry-off (an actively-sampling tracer leaves the `(now, delta_count)` trajectory byte-identical) — with rv32i/platform/AT bit-identical throughout ([doc/plan-m5.md](doc/plan-m5.md)). The off-thread `WriterSink` is the one real `Send` boundary, flushed/joined deterministically at end-of-sim. **M6 (digital-twin layer) is complete** — a new L6 crate `systemrs-twin` (depends on kernel + time only) plus two additive, no-op-when-unattached kernel seams (a starvation gate + a time-advance hook) deliver all three exit criteria: E1 a `RealTimePacer` paces wall clock to sim time at the time-advance hook (femtosecond-based, deltas instantaneous) and reports slip as a plain `PacerStats`; E2 an externally-driven model **parks (does not exit) and resumes on injection** via `ExternalInput`/`attach_external_input` (suspend-on-starvation, with the resume path running one `commit_and_notify` so delta injections aren't dropped); E3 a seeded SplitMix64 `Rng` + `Journal`/`JournalReplayer` replay a recorded run to a **byte-identical transaction trace** (seed proven load-bearing; empty-trace negative guard) — with rv32i/platform/AT/M5 bit-identical when nothing is attached ([doc/plan-m6.md](doc/plan-m6.md)). The core stays `!Send`; only the mpsc inbox + a `StopSignal` condvar cross threads. **The roadmap MVP (M0–M6) is complete.** **M7 slice 1 (Tier-1 conservative barrier-synchronous PDES) is complete** — a new L7 crate `systemrs-pdes` (regions wrapping Tier-0 kernels, deterministic quantum-boundary cross-region exchange sorted by `(deliver_at, dst_region, dst_link, src_seq)`, a single `rayon`-gated `unsafe impl Send for Region`, and `--verify-determinism` via Tier-0/Tier-1 trace equality) plus one additive kernel seam (`Sim::schedule_event_at`), with all exit criteria E1–E5 proven ([doc/plan-m7.md](doc/plan-m7.md)). **M7 slice 2 (bounded snapshot/restore) is also complete** — a `KernelSnapshot` captures the kernel-visible scheduler state (counters, timed wheel, per-event pending + ordered dynamic subscriptions, per-process wait state) at a quiescent boundary and `Sim::restore` applies it to a freshly-rebuilt model, so a run-to-completion method model snapshotted mid-run continues **byte-identically** (the bound per §6f: coroutine stacks are not serialized, so threads holding stack-local state across a `wait` are out of scope; the model saves/restores its own channel/service state). **M7 slice 3 is complete as the interop *seam* plus the smaller deferred items** — the packaging was revised so SystemC co-sim is **out-of-tree**: a separate bridge repo plugs into a pure-Rust seam (a swappable `Rc<RefCell<dyn FwTransport>>` forward-transport target via `TargetSocket::set_fw_transport` — which is also the bounded structural hot-swap of §6f — plus the generic-payload byte API), and `Ctx::kill` now **force-unwinds a parked thread's coroutine stack** (full throw/unwind kill semantics, §6a) rather than only cooperative cancellation. So the core stays pure Rust (no C++ build, no `cosim` feature). Remaining: the actual C++ SystemC bridge (its own repo), `reset` (needs a body factory), and the foreign-scheduler kernel-guest seam.

👉 **Next phase plan:** [doc/plan-m2.md](doc/plan-m2.md) — Milestone 2.

---

## Status by area

### Crates / Workspace structure

_§10 (14-crate plan)_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ✅ | Workspace (resolver 3, edition 2024, MSRV 1.90, Apache-2.0) | REPLICATE | §10.4 / §13 naming | Cargo.toml (resolver=3, edition 2024, rust-version 1.90, license Apache-2.0, workspace.lints + workspace.dependencies) — 10 of 14 crates present. Lints (clippy all+pedantic, missing_docs, unsafe_code=warn) and shared deps configured per skill. |
| ✅ | systemrs-diag (L0, reporting) | REPLICATE | §10.1 | crates/systemrs-diag/src/{lib,report,severity}.rs |
| ✅ | systemrs-time (L0, SimTime) | REPLICATE | §10.1 | crates/systemrs-time/src/{sim_time,resolution}.rs |
| ✅ | systemrs-runtime (L0, coroutine backend) | REPLICATE | §10.1 | crates/systemrs-runtime/src/stackful.rs (corosensei Fiber + suspend()) |
| ✅ | systemrs-kernel (L1, scheduler/queues/events/processes/arenas) | REPLICATE | §10.1 | crates/systemrs-kernel/src/{sim,inner,event,process,timed,ctx,ids,channel,phase}.rs |
| 🟡 | systemrs-core (L2, Module/Object, elaboration, sensitivity) | REPLICATE | §10.1 | crates/systemrs-core/src/{build,elaborate}.rs — Only the process-builder facade (Build/MethodBuilder/ThreadBuilder) and a default-empty Elaborate trait. No arena Object hierarchy, no naming/uniqueness, no Module type. Much thinner than §6b/M2. |
| 🟡 | systemrs-channels (L3, Signal/Fifo/Clock) | REPLICATE | §10.1 | crates/systemrs-channels/src/{signal,fifo,clock}.rs — Signal/Buffer/Fifo/Clock present; no ports/exports/binding, no mutex/semaphore, no signal posedge/negedge. |
| 🟡 | systemrs-tlm2 (L4, GP+MM+extensions, transport, phases, DMI, sockets) | REPLICATE | §10.1 | crates/systemrs-tlm2/src/{gp,mm,extension,protocol,socket,phase,memory}.rs — LT path (GP, MM pool, extensions, b_transport, transport_dbg, sockets) done; AT/nb_transport/DMI only as unused trait-default stubs. |
| ✅ | systemrs (L6, facade/prelude) | REPLICATE | §10.1 | crates/systemrs/src/{lib,prelude}.rs (re-exports all built crates + prelude) |
| ✅ | systemrs-examples (L7, conformance/integration tests) | REPLICATE | §10.1 | crates/systemrs-examples/src/{counter,rv32i,platform}.rs + tests/{integration,hierarchy,module_macro}.rs (counter + RV32I hart + two-level TLM platform capstone) — Dev-deps insta/criterion from §10.1 not yet used. |
| ✅ | systemrs-macros (L0, proc-macros / #[module]) | SIMPLIFY | §10.1, §4 modules | M2-11: `crates/systemrs-macros` (proc-macro2/quote/syn only); `#[module]` attribute emits `::systemrs::Module` (path-qualified, no facade cycle). Facade-routed test in `systemrs-examples`. |
| ✅ | systemrs-tlm1 (L4, put/get/peek + analysis ports) | REPLICATE | §10.1, §3.7 | M5: `AnalysisPort` (re-entrancy-safe in-order fan-out), `AnalysisFifo` (unbounded stream), `AnalysisTriple`. General `tlm_fifo` put/get/peek deferred (bounded blocking FIFO already in `channels::Fifo`). 5 tests. |
| ✅ | systemrs-tlm-utils (L5, quantum keeper, PEQs, convenience sockets, LT/AT adapters) | REPLICATE | §10.1, §3.11 | M4: `QuantumKeeper`/`GlobalQuantum`, `PeqWithGet`/`PhaseQueue`, AT FSM, `LtToAtAdapter`/`AtToLtAdapter`, `AtMemory`, `SimpleInitiator/TargetSocket` (nb→b synthesis), `Multi/PassthroughTargetSocket` stubs. 13 tests. |
| ✅ | systemrs-trace (L5, sampling, recorders, VCD/FST) | SIMPLIFY | §10.1, §3.12 | M5: `Tracer` (PostUpdate signal sampling via Copy handles), `MemorySink`, off-thread `WriterSink` (Send boundary, end-of-sim flush), `TxnRecord`/`TraceEvent`. VCD/FST + AT phase accumulation deferred. 2 tests. |
| ⏸️ | systemrs-ffi (SystemC interop) | REPLICATE, **out-of-tree** | §10.1, §11 | Deliberately not an in-tree crate (M7 slice-3 decision): a separate bridge repo against the in-core `FwTransport` seam. Core stays C++-free. |

### M0 — Kernel, time, events & the delta loop

_M0 (§12, §3.1, §3.3, §6a)_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ✅ | Three-phase delta cycle (evaluate/update/notify) | REPLICATE | §4 Kernel; §6a crunch loop | crates/systemrs-kernel/src/sim.rs crunch() (EVALUATE methods->threads, UPDATE, DELTA-NOTIFY high-index->0) |
| ✅ | Timed-event wheel + time advance + tie-break seq | REPLICATE | §4 Kernel; §6a | crates/systemrs-kernel/src/timed.rs (min-heap keyed (when,seq), lazy tombstone cancel) |
| ✅ | Immediate / delta / timed notification + collapse (earliest-wins) | REPLICATE | §4 Kernel; §3.3 | inner.rs notify_immediate/notify_delta/notify_timed/cancel + event.rs Pending state machine; tested notify_collapse_delta_beats_timed (tests.rs:54) |
| ✅ | change_stamp / delta_count counters | REPLICATE | §4 Kernel; §6a | inner.rs change_stamp/delta_count/delta_count_baseline_at_now; bumped only on non-empty deltas (sim.rs crunch empty-delta guard) |
| ✅ | triggered() within firing change-stamp window | REPLICATE | §4 Kernel (change_stamp underpins triggered) | event.trigger_stamp set in inner.rs trigger(); Ctx::triggered (ctx.rs:90); tested triggered_is_false_for_never_fired_event (tests.rs:97) |
| ✅ | SimTime (64-bit unit count) + resolution as construction param | REPLICATE (as construction param) | §4 Kernel; §6a time type | sim_time.rs (SimTime(u64), ZERO/INF/from_ns/...); resolution.rs (Resolution, Sim::with_resolution) — builder, not freeze-on-first-use global |
| ✅ | Empty-delta guard (empty evaluate advances no counter) | REPLICATE | §12 M0 exit | sim.rs crunch(): `if !ran { break }` before incrementing counters |
| 🟡 | sc_start / stop / pause typestate | SIMPLIFY | §4 Kernel | sim.rs run_until + ensure_started 'started' flag (runtime-checked Building->Running) — run_until drives to a time; no explicit stop()/pause() API or typestate type. Sufficient for examples. |
| 🟡 | Starvation policy (SC_RUN_TO_TIME vs SC_EXIT_ON_STARVATION) | REPLICATE | §3.1, §4 Kernel | phase.rs Starvation enum exists but is never consumed (grep: no use in inner.rs/sim.rs); run_until is implicitly run-to-time |
| 🟡 | Stage/phase callbacks (PreTimestep / PostUpdate) | SIMPLIFY | §4 Kernel; §3.12, §6e | phase.rs Stage enum defined; NOT wired — sim.rs:432 'callbacks would fire here once -trace lands'. Full bitmask taxonomy correctly dropped. |
| ⏸️ | sc_suspend_all / sc_unsuspend_all (with suspend hook in next_time) | DEFER | §4 Kernel | Not implemented; next_timed_when has no suspend hook yet. |
| ❌ | preempt_with nested execution | DROP (MVP), DEFER | §4 Kernel | Out of scope per §4; absent. |
| ❌ | Deprecated APIs | DROP | §4 Kernel | Intentionally not ported. |

### M0/Events — Events, notification & sensitivity

_M0 (§3.3, §4 Events)_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ✅ | sc_event + notify/cancel/trigger + collapse | REPLICATE | §4 Events | event.rs Event (4 subscriber lists), inner.rs trigger() fixed-order walk; Ctx::notify/notify_now/notify_after/cancel (ctx.rs:101-116) |
| ✅ | AND/OR lists, timeouts, wait(t, ev) | REPLICATE | §4 Events | process.rs WaitReq {Time,Event,EventTimeout,Or,OrTimeout,And}; Ctx::wait_any/wait_all/wait_event_timeout (ctx.rs:173-185); tested in adversarial_and_verify.rs (AND stale-subscription + completion) |
| ✅ | Immediate self-notification guard | REPLICATE | §4 Processes; §6a | tested immediate_self_notification_guard (kernel/src/tests.rs:78) |
| ⬜ | Expression-template &/\| syntax -> BitAnd/BitOr on event refs | SIMPLIFY | §4 Events | No BitAnd/BitOr impls; callers pass &[EventId] slices to wait_any/wait_all instead. Functional equivalent present, operator sugar absent. |
| ⬜ | sc_event_queue (lossless, as a channel) | REPLICATE | §4 Events | No EventQueue channel; PEQs (which need it) not yet built (M4). |
| ⏸️ | sc_event_finder (closure/selector at bind) | SIMPLIFY | §4 Events | No binding/finder machinery yet (ports/exports absent). |
| ✅ | sensitive << DSL -> explicit process builder | SIMPLIFY | §4 Events; §6b | core/build.rs MethodBuilder/ThreadBuilder.sensitive_to(); no hidden last-process state (matches §6b intent) |

### M1 — Process model & coroutines

_M1 (§3.2, §4 Processes, §6a)_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ✅ | SC_METHOD (run-to-completion FnMut) | REPLICATE | §4 Processes | process.rs ProcessBody::Method(Box<dyn FnMut(&Ctx)>); Sim::add_method (sim.rs:115); tested methods_run_before_threads_in_a_delta (tests.rs:34) |
| ✅ | SC_THREAD + wait() from arbitrary depth (stackful coroutine) | REPLICATE (stackful) | §4 Processes; §6a recommendation | process.rs ProcessBody::Thread(Fiber); runtime/stackful.rs corosensei Fiber + suspend(); rv32i.rs calls wait inside b_transport (3+ levels deep): Bus::b_transport -> isock.b_transport -> memory callback ctx.wait |
| ✅ | next_trigger() dynamic sensitivity | REPLICATE | §4 Processes | Ctx::next_trigger/next_trigger_event/next_trigger_any (ctx.rs:194-204) |
| ✅ | Single stackful backend (corosensei), drop OS-thread emulation | SIMPLIFY | §4 Processes | runtime/stackful.rs single backend; Cargo.toml corosensei with unwind feature |
| ✅ | Suspended-fiber force-unwind on drop (run destructors) | REPLICATE | §6a; Cargo.toml rationale | runtime/stackful.rs tested drop_suspended_fiber (line 235) |
| 🟡 | kill / reset / throw_it (cooperative cancellation only in MVP) | SIMPLIFY -> DEFER (full) | §4 Processes | process.rs has 'dead' flag + wait_gen lazy-cancel; no user-facing kill/reset API or synchronous-throw |
| ⬜ | sc_spawn / sc_spawn_options | SIMPLIFY | §4 Processes | Only elaboration-time add_method/add_thread; no runtime sc_spawn. |
| ⬜ | sc_join / fork-join (join_all) | SIMPLIFY | §4 Processes | No join_all helper. |
| ⏸️ | suspend/resume/disable/enable (testbench control) | DEFER | §4 Processes | Not implemented; testbench control deferred per §4. |
| ❌ | SC_CTHREAD (clocked threads) | DROP | §4 Processes | RTL construct, out of scope (CLAUDE.md scope statement); absent. |
| ❌ | Reset-signal machinery | DROP | §4 Processes | RTL concept; absent. |
| ⬜ | Stackful-vs-async decision memo + 10k-thread benchmark | n/a | §12 M1 exit | No committed decision memo or criterion benchmark found; M1 exit-criteria artifacts not present (functionality is implemented). |

### M2 — Modules, hierarchy, ports/exports, elaboration

_M2 (§3.4, §3.5, §4, §6b)_

> **✅ M2 COMPLETE** (phases A–F, M2-01…14). All seven exit criteria proven by the
> `systemrs-examples` platform capstone + unit tests: EC1 dot-joined names · EC2 socket bind via
> `complete_binding` · EC3 hierarchical port-to-port · EC4 binding-after-`build()` is a compile
> error (compile-fail doctest) + runtime `Err` · EC5 construction fixpoint · EC6 four callbacks in
> bucket order + `end_of_simulation` once · EC7 port-policy cardinality. Existing M0/M1/M3 tests
> bit-identical throughout. See [doc/plan-m2.md](doc/plan-m2.md).

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ✅ | Four lifecycle callbacks + construction fixpoint | REPLICATE | §4 Modules; §6b; §12 M2 | M2-06/07: `core/elaboration.rs` `drive` runs the per-bucket construction fixpoint + the four callbacks in bucket order (port→export→prim_channel→module) with the clone-Rc-out borrow-release discipline; wired into `Sim::run_until` via the dependency-inverted hook (`elaborate_once`), with init-commit pass + fire-once/`end_of_sim` latches. 5 tests (order, fixpoint, re-entrancy, init-commit, once). Existing examples bit-identical. |
| ✅ | #[module] / SC_MODULE / SC_CTOR macro | SIMPLIFY | §4 Modules | M2-11: `#[module]` attribute (`systemrs-macros`) generates the `Module` marker impl, path-qualified to avoid a facade cycle. |
| ✅ | Object hierarchy + naming + uniqueness | REPLICATE | §4 Modules; §6b | M2-02: `core/object.rs` `ObjectStore` (`SlotMap<ObjectId, ObjectMeta>` + name table + scope stack + implicit root); dot-joined unique names, sanitisation, deterministic suffixing. 9 unit tests. |
| ✅ | sc_module_name LIFO-stack -> cx.module(name, \|m\| {..}) scope closures | DROP mechanism, REPLICATE outcome | §4 Modules | M2-08: `core/module.rs` `module`/`module_with` + `Builder` (nested modules, `m.method`/`m.thread`); `core/hierarchy.rs` `ScopeGuard` RAII push/pop. `Kernel<Building/Running>` typestate front door (M2-10). 5 tests. |
| 🟡 | Orphan-children-to-root-on-drop via arena re-parent | REPLICATE | §4 Modules | M2-02: `ObjectStore::reparent_children_to_root` (pure-id reparent) + unit test present; full destruction-order integration deferred (§12 M7+). |
| ✅ | Interface/port/export + two-phase deferred bind + complete_binding | REPLICATE | §4 Ports; §12 M2 | M2-04/05: `channels/{interface,port,export,binding}.rs` — `Port<IF>`/`Export<IF>` Copy handles, id-keyed `PortRegistry`, two-phase `record` + `complete` (idempotent, cycle-guarded), auto-driven at the barrier; **consumed by the TLM sockets** (M2-09) and the platform capstone. 10 unit tests. |
| ✅ | Multiports + port-policy counting | REPLICATE | §4 Ports | M2-04/05: `PortPolicy` (`OneOrMore`/`AllBound`/`ZeroOrMore`) enforced at end of `complete`; multiport flatten preserves order. Tested. |
| ✅ | Hierarchical port-to-port binding | REPLICATE | §4 Ports; §12 M2 | M2-05: `complete` flattens parent forwards depth-first (borrow-safe id-threading); port→parent-port and port→export chains tested incl. 3-deep. |
| ✅ | Attributes (sc_attribute<T>) / AttributeStore | DEFER→done | §4 Modules; §6b | M2-12: `AttributeStore` lazy `get`/`set` (`core/attribute.rs`) + `ObjectStore::set_attribute`/`attribute` wiring on `ObjectMeta`. Tested. (DEFER per §4, but landed.) |

### M3 — Primitive channels + first end-to-end LT transaction

_M3 (§3.6, §3.8-3.10, §4, §6c, §6d)_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ✅ | prim_channel evaluate/update discipline (request_update + update phase) | REPLICATE | §4 Channels; §6c | kernel/channel.rs UpdatableChannel trait; Ctx::request_update; sim.rs UPDATE phase drains queue; channels stage-then-commit |
| ✅ | sc_signal / sc_buffer (bool/int) | SIMPLIFY -> keep | §4 Channels | channels/signal.rs Signal<T:Copy> + Buffer<T>; value_changed_event for next-delta notify |
| ⬜ | Signal posedge/negedge events | SIMPLIFY | §4 Channels | signal.rs has only value_changed_event; no posedge/negedge (grep: none). Note Clock provides posedge/negedge but Signal does not. |
| ✅ | sc_fifo (bounded blocking) | REPLICATE | §4 Channels | channels/fifo.rs Fifo<T> over VecDeque; blocking put/get yield the thread; try_put/try_get/num_available; tested (channels/src/tests.rs, 4 pass) |
| ✅ | sc_clock (self-scheduling) | SIMPLIFY -> DEFER | §4 Channels | channels/clock.rs Clock with posedge/negedge/value_changed events; used by counter example. Built despite DEFER classification. |
| ⬜ | sc_mutex / sc_semaphore | SIMPLIFY | §4 Channels | Not implemented (only std::sync::Mutex used inside tests). |
| ⬜ | Writer policy (runtime enum check, strict mode) | SIMPLIFY | §4 Channels | Signal::write has no writer-conflict / strict-mode check. |
| ❌ | Resolved signals | DROP | §4 Channels | RTL multi-driver; out of scope. |
| ✅ | TLM-2 Generic payload (owned buffer) | REPLICATE | §4 TLM2; §6d | tlm2/gp.rs GenericPayload (Command/ResponseStatus/ByteEnable sum types, owned Vec<u8>, dmi_allowed, extensions) |
| ✅ | MM acquire/release -> Rc<Payload> + pool | SIMPLIFY | §4 TLM2; §6d | tlm2/mm.rs TxnPool::acquire/recycle; tested txn_pool_recycles_and_resets (tests.rs:187) |
| ✅ | Extensions (TypeId-keyed map, no RTTI) | REPLICATE (idiomatic) | §4 TLM2; §6d | tlm2/extension.rs ExtensionMap (HashMap<TypeId,Box<dyn Extension>>), set/get/take/contains |
| ✅ | b_transport + timing annotation | REPLICATE | §4 TLM2; §12 M3 | protocol.rs FwTransport::b_transport(ctx,txn,delay); socket.rs InitiatorSocket::b_transport; memory.rs target; tested b_transport_write_then_read_roundtrip + rv32i integration |
| ✅ | transport_dbg (backdoor peek/poke) | REPLICATE | §4 TLM2 | protocol.rs transport_dbg; socket.rs InitiatorSocket::transport_dbg + register_transport_dbg; tested transport_dbg_peek_and_poke (tests.rs:126) |
| 🟡 | Sockets (initiator/target) + bind cycle | REPLICATE | §4 TLM2; §6d; §12 M3 | M2-09: `InitiatorSocket` *is* a forward `Port<BaseProtocol>`; `bind` is **deferred** (recorded, resolved at the barrier via `complete_binding`); unbound socket → FATAL at elaboration. Closure registry kept as resolved-interface storage. bw/nb path + target-side export hierarchy (passthrough/multi) deferred to M4. |
| ✅ | Convenience sockets (closure registration, no void* trampoline) | REPLICATE (adapters) | §4 TLM2; §6d | M4-11: `tlm-utils/simple_socket.rs` `SimpleInitiatorSocket`/`SimpleTargetSocket` (boxed closures + **nb→b synthesis** via a Send-safe spawned worker); `Multi/PassthroughTargetSocket` distinct-type stubs (illegal bind is a compile error — compile-fail doctest). Full multi/passthrough fan-out deferred to M5. |
| ✅ | Payload pool recycles with no leaks under stress (M3 exit) | REPLICATE | §12 M3 exit | tlm2/src/tests.rs txn_pool_recycles_and_resets verifies recycle + reset; reentrant shared target tested (shared_target_reentrant_b_transport) |
| ✅ | FIFO 'written in N, readable in N+1' rule (M3 exit) | REPLICATE | §12 M3 exit | channels/src/tests.rs exercises producer/consumer visibility discipline (4 tests pass) |

### M4 — Temporal decoupling / AT protocol / PEQ

_M4 (§3.9, §3.11, §4, §6d)_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ✅ | Quantum keeper + global quantum | REPLICATE | §4 TLM2; §6d temporal decoupling; §12 M4 | M4-03/04: `tlm-utils/{global_quantum,quantum}.rs` — `GlobalQuantum` Sim service, `QuantumKeeper` (`need_sync` `>=`, `sync` the only yield, integer-only `q - now%q`). E1 sync-on-grid proven. |
| ✅ | PEQ (peq_with_get then phase-aware, delta parity) | REPLICATE | §4 TLM2; §3.11; §12 M4 | M4-05/06: `PeqWithGet` (`BTreeMap<(SimTime,seq)>` + delta parity via `notify_after`) + `PhaseQueue` (SC_METHOD drain). E5 (one delta apart, FIFO) proven. No `sc_event_queue` needed (deferred to M5). |
| ✅ | nb_transport_fw / nb_transport_bw + 4-phase FSM + TlmSync | REPLICATE | §4 TLM2; §3.9; §12 M4 | M4-07/08: nb routed through the socket closure registry (crossed bw double-bind, `BwBaseProtocol`, keyed by `bw_export` id); `at.rs` `next_phase` FSM; E2 exercises all three `TlmSync` paths. |
| ✅ | b<->nb (LT<->AT) adapters | REPLICATE (explicit adapters) | §4 TLM2; §12 M4 | M4-09/10: `LtToAtAdapter` (blocks on per-`TxnId` event, E3) + `AtToLtAdapter` (spawns a Send-safe per-txn coroutine reaching the `Txn` via a service, E4). |
| ✅ | DMI (get_direct_mem_ptr / invalidate, arena handle/slice) | SIMPLIFY | §4 TLM2; §3.9 | M4-12: `Dmi` + `DmiAccess` (plain bools, no `bitflags`); socket `get_direct_mem_ptr` (fw) + `invalidate_direct_mem_ptr` (bw) wired through the closure registry; §3.9 re-entrancy guard (get-inside-invalidate → FATAL). Tested. |
| 🟡 | Extended phases (Phase::Extended(PhaseId) interned) | SIMPLIFY | §4 TLM2 | phase.rs Phase::Extended(PhaseId) variant defined; no interning registry, unexercised. |
| ⏸️ | Endianness helpers | DEFER | §4 TLM2 | Not implemented (deferred per §4). |
| ⏸️ | Instance-specific extensions | DEFER | §4 TLM2 | Not implemented (deferred per §4). |
| ✅ | Two same-time zero-delay notifications fire one delta apart FIFO (M4 exit) | REPLICATE | §12 M4 exit | M4-05: `PeqWithGet` `two_zero_delay_notifies_fire_one_delta_apart_fifo` test (E5) — released via `notify_delta`, one-per-delta drain. |

### M5 — Observability, reporting & tracing

_M5 (§3.7, §3.12, §4, §6e)_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| 🟡 | Reporting severity/action/verbosity; ERROR->Result, FATAL->abort | SIMPLIFY (essential) | §4 Support; §7; §12 M5 | diag/{severity,report,lib}.rs: Severity enum, Report, ReportError, report_info/warning + error()->ReportError + report_fatal()->! (aborts). No verbosity gating, no per-message action config. |
| ✅ | Action precedence (pure fn matching golden table) | REPLICATE (pure fn) | §4 Support; §12 M5 exit | M5: `diag::ReportHandler::resolve` (pure: type+severity > severity > golden `ActionFlags::default_for`) + `Verbosity` INFO gate; `emit` applies DISPLAY/THROW/ABORT/CACHE. EC3 tested; existing `report_*` free fns byte-identical. |
| ⏸️ | Cached report / per-process current-process cache | DEFER | §4 Support | Not implemented (deferred per §4). |
| ✅ | tlm_analysis_port / tlm_write_if fan-out | REPLICATE | §4 TLM1; §3.7; §12 M5 | M5: `tlm1::AnalysisPort` (`Weak` subscribers, snapshot-then-iterate → synchronous, in registration order, re-entrancy-safe) + `AnalysisWrite`. EC1 tested. |
| ✅ | tlm_analysis_fifo (unbounded decoupler) | REPLICATE | §4 TLM1 | M5: `tlm1::AnalysisFifo` — unbounded (`write` never back-pressures), put-N/readable-N+1 via the update discipline, drain-all-per-wake. EC2 tested (10k writes/delta). |
| ✅ | tlm_analysis_triple (timestamped telemetry) | REPLICATE (explicit conversions) | §4 TLM1 | M5: `tlm1::AnalysisTriple { time, delta, value }` + `AnalysisTriple::now(ctx, value)`. |
| ⬜ | tlm_fifo + put/get/peek (TLM-1 message passing) | REPLICATE | §4 TLM1; §3.7 | systemrs-tlm1 absent. (Note: a primitive-channel Fifo exists in systemrs-channels, but the TLM-1 tlm_fifo/peek API is separate and not built.) |
| ⬜ | tlm_transport_if (one required method + default) | SIMPLIFY | §4 TLM1 | Not implemented. |
| ✅ | Tracing via stage callbacks (sample after update commits) | REPLICATE (sampling discipline) | §4 Support; §3.12, §6e; §12 M5 | M5: kernel `add_stage_hook` fires `PostUpdate` (after update commits) / `PreTimestep` (before time advance) — a true no-op when empty (M0-M4 bit-identical); `trace::Tracer` samples signals via Copy handles. EC4 (telemetry on==off) tested. |
| 🟡 | VCD/FST -> transaction-centric sink | SIMPLIFY -> transaction sink | §4 Support; §3.12 | M5: transaction-centric `TxnRecord`/`TraceEvent` + `MemorySink` + off-thread `WriterSink` (text). VCD/FST value-change backends deferred. |
| ⬜ | Off-thread telemetry writer | REPLICATE | §12 M5; §6e | Not implemented. |
| ✅ | transport_dbg query API (twin inspection) | REPLICATE | §12 M5 | tlm2/socket.rs transport_dbg path + memory.rs register_transport_dbg; tested. Surfaced through the facade alongside the M5 observability layer. |
| ⬜ | sc_vector -> Vec<T> + scoped builder | SIMPLIFY | §4 Support | No sc_vector analogue / scoped builder (plain Vec used ad hoc). |
| ❌ | circular_buffer raw storage | DROP | §4 TLM1 | Use VecDeque per §4; FIFO uses VecDeque. |
| ❌ | tlm_tag<T> | DROP | §4 TLM1 | Unnecessary in Rust; absent. |
| ❌ | sc_dt datatypes (~58k LOC) | DROP | §4 Support | Out of scope (CLAUDE.md + §4); native ints + [u8] used instead. |

### M6 — Digital-twin layer

_M6 (§6f, §12)_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ✅ | RealTimePacer (wall-clock pacing + slip telemetry) | REPLICATE (twin layer) | §6f; §12 M6 | M6: `twin::RealTimePacer` on the kernel time-advance hook (femtosecond-based, deltas instantaneous); slip as plain `PacerStats` (no trace dep). EC1 tested. |
| ✅ | ExternalInput inbox + suspend-on-starvation (park, don't exit) | REPLICATE (twin layer) | §6f; §12 M6 | M6: `twin::ExternalInput`/`ChannelInput`/`attach_external_input` + kernel starvation gate (`SuspendOnStarvation`); parks on a `StopSignal` condvar, resume path runs `commit_and_notify` for delta injections. EC2 tested. |
| ✅ | Seeded RNG + input journal + deterministic replay | REPLICATE (twin layer) | §6f; §8; §12 M6 | M6: `twin::Rng` (SplitMix64 service, no `thread_rng`), `Journal`/`JournalRecorder`/`JournalReplayer` (replay-driver process, no live thread). EC3 byte-identical trace, seed load-bearing. |
| ⬜ | Seeded RNG service | DEFER | §6f; §12 M6 | Not implemented. (Sim::register_service plumbing exists and could host it.) |
| ⬜ | Input journal + replay (byte-identical trace) | DEFER | §6f; §12 M6 | Not implemented (grep: no journal/replay). |

### M7+ — Deferred

_M7+ (§12, §6f, §8a)_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ⏸️ | Snapshot/restore (bounded: arena columns + queues + resumable processes) | DEFER | §12 M7; §6f | Not implemented; arena+id design does not preclude it. |
| ⏸️ | Structural hot-swap | DEFER | §12 M7 | Not implemented. |
| ⏸️ | Full kill/reset throw semantics | DEFER | §12 M7; §4 Processes | Only cooperative cancellation present (dead flag/wait_gen). |
| ⏸️ | Tier-1 parallel region orchestrator + --verify-determinism + integer time | DEFER | §12 M7; §8a; §8 invariants | No parallel tier, no rayon, no RegionHandle, no --verify-determinism flag (grep: none). Single-threaded golden reference only, as mandated for M0. |
| ⏸️ | Endianness helpers / instance-specific extensions (M7 restatement) | DEFER | §12 M7; §4 TLM2 | Not implemented (see M4 rows). |
| 🟡 | ECS data architecture (columnar arena store) | DEFER (advisory) | §9 | kernel/inner.rs uses slotmap arenas + a TypeId-keyed services HashMap (the 'ECS-flavoured store' seam noted in inner.rs); not a full columnar SoA ECS. |

### §11 — SystemC interoperability

_§11 interop phases_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ⏸️ | Phase 1: Rust models as guests inside C++ SystemC kernel (cxx) | REPLICATE, **out-of-tree** | §11.2 | The in-core **seam** is provided (`TargetSocket::set_fw_transport` with `Rc<RefCell<dyn FwTransport>>` + the GP byte API); the C++ bridge implementing it (Phase 1 + the firewall) lives in a separate repo. |
| ⬜ | Payload marshaling & ownership across FFI | REPLICATE | §11.3 | No FFI marshaling code. |
| ⏸️ | Phase 2: C++ guests inside Rust kernel | DEFER | §11.4; §11 | Phased after Phase 1; not started. |
| ⏸️ | Phase 3: out-of-process quantum-synchronized co-sim | DEFER | §11.5 | Not started (and depends on quantum keeper, also absent). |
| ⬜ | Symmetric panic/exception firewall (catch_unwind at extern C; C++ try/catch around re-entry) | REPLICATE | §11.2, §11.6; CLAUDE.md interop | No catch_unwind / extern C entry points anywhere (grep: none); firewall is an FFI-path concern and FFI is absent. |
| ✅ | Single-scheduler invariant (reject two live kernels in one process) | REPLICATE (constraint) | §11.1 | Architecturally upheld: only the single-threaded systemrs-kernel scheduler exists; no second kernel can be instantiated. (Thread-local CURRENT_SIM in ctx.rs enforces one current sim per thread.) |

