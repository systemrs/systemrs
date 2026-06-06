# SystemRS — Implementation Status

Tracks every feature in [doc/systemrs-design.md](doc/systemrs-design.md) against what is
actually built in `crates/`. Snapshot as of **2026-06-06**. The design's own
REPLICATE / SIMPLIFY / DEFER / DROP decisions (§4) are carried in the **Decision** column;
this file's **Status** column is the *as-built* reality.

> Regenerate by re-running the gap analysis against the design doc + `crates/` and editing
> the tables below. Keep it in sync as milestones land.

**Legend:** ✅ Done · 🟡 Partial · ⬜ Missing (planned, not built) · ⏸️ Deferred (intentionally post-MVP) · ❌ Dropped (explicitly out of scope, §4)

**Build health (2026-06-06):** `cargo test --workspace` → **71 passed, 0 failed**; `fmt`/`clippy -D warnings`/`build --release`/`doc`/`deny`/`audit` all clean. **10 of 14** planned crates exist (`systemrs-macros` added in M2-11).

**Feature tally (114 tracked, pre-M2 baseline):** ✅ 36 DONE  ·  🟡 17 PARTIAL  ·  ⬜ 39 MISSING  ·  ⏸️ 14 DEFERRED  ·  ❌ 8 DROPPED _(M2 rows below updated as Phase A lands)_

## Where we are

M0 (delta loop) and M1 (process model) are **done and bit-faithful**; the M3 **LT TLM-2.0** slice is **done** (generic payload, pool MM, extensions, `b_transport`, `transport_dbg`, id-keyed sockets). **M2 (modules / hierarchy / ports / exports / elaboration) is now in progress** — Phases A–E have landed (M2-01…11): the `ObjectStore` foundation + kernel hooks, the generic `Port`/`Export` two-phase binding + `complete_binding`, the elaboration driver wired into `run_until`, the user-facing front door (`module()` scope closures + `Builder`, the `Kernel<Building/Running>` typestate, the `#[module]` macro), and the TLM socket reconciliation onto the generic `Port` binding (deferred bind, unbound→FATAL). Only the **Phase-F polish** remains: AttributeStore get/set bodies (M2-12), a two-level platform example proving all 7 exit criteria (M2-13), and the final facade/sweep consolidation (M2-14) ([doc/plan-m2.md](doc/plan-m2.md)). M4 (AT/PEQ/quantum), M5 (observability/tracing/TLM-1), M6 (twin layer), M7+ and §11 interop are not started; their TLM-2 contract types exist only as inert trait-default stubs.

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
| ✅ | systemrs-examples (L7, conformance/integration tests) | REPLICATE | §10.1 | crates/systemrs-examples/{src/{counter,rv32i},examples,tests/integration}.rs (counter + RV32I hart; 3 integration + 10 unit tests pass) — Dev-deps insta/criterion from §10.1 not yet used. |
| ✅ | systemrs-macros (L0, proc-macros / #[module]) | SIMPLIFY | §10.1, §4 modules | M2-11: `crates/systemrs-macros` (proc-macro2/quote/syn only); `#[module]` attribute emits `::systemrs::Module` (path-qualified, no facade cycle). Facade-routed test in `systemrs-examples`. |
| ⬜ | systemrs-tlm1 (L4, put/get/peek + analysis ports) | REPLICATE | §10.1, §3.7 | Crate does not exist. |
| ⬜ | systemrs-tlm-utils (L5, quantum keeper, PEQs, convenience sockets, LT/AT adapters) | REPLICATE | §10.1, §3.11 | Crate does not exist; no quantum/PEQ code anywhere (only doc-comment mentions). |
| ⬜ | systemrs-trace (L5, sampling, recorders, VCD/FST) | SIMPLIFY | §10.1, §3.12 | Crate does not exist; sim.rs:432 comment notes stage callbacks would fire 'once -trace lands'. |
| ⬜ | systemrs-ffi (L7, C ABI / cxx SystemC interop) | REPLICATE | §10.1, §11 | Crate does not exist; cosim feature not wired. |

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

> **M2 in progress** — Phases A–E landed (M2-01…11): the `ObjectStore` arena + per-bucket registries; the generic `Interface`/`Port`/`Export` two-phase binding + `complete_binding`; the elaboration driver wired into `run_until`; the user-facing front door — `module(name, |m| {…})` scope closures with `Builder` + `ScopeGuard`, the `Kernel<Building/Running>` typestate (compile-time bind-after-start guard), the `#[module]` proc-macro; **and the TLM socket reconciliation** onto the generic `Port` (deferred bind, unbound→FATAL, rv32i bit-identical). The **only remaining work is the Phase-F polish**: AttributeStore bodies (M2-12), a two-level platform example proving all 7 exit criteria (M2-13), and the facade/sweep consolidation (M2-14). See [doc/plan-m2.md](doc/plan-m2.md).

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ✅ | Four lifecycle callbacks + construction fixpoint | REPLICATE | §4 Modules; §6b; §12 M2 | M2-06/07: `core/elaboration.rs` `drive` runs the per-bucket construction fixpoint + the four callbacks in bucket order (port→export→prim_channel→module) with the clone-Rc-out borrow-release discipline; wired into `Sim::run_until` via the dependency-inverted hook (`elaborate_once`), with init-commit pass + fire-once/`end_of_sim` latches. 5 tests (order, fixpoint, re-entrancy, init-commit, once). Existing examples bit-identical. |
| ✅ | #[module] / SC_MODULE / SC_CTOR macro | SIMPLIFY | §4 Modules | M2-11: `#[module]` attribute (`systemrs-macros`) generates the `Module` marker impl, path-qualified to avoid a facade cycle. |
| ✅ | Object hierarchy + naming + uniqueness | REPLICATE | §4 Modules; §6b | M2-02: `core/object.rs` `ObjectStore` (`SlotMap<ObjectId, ObjectMeta>` + name table + scope stack + implicit root); dot-joined unique names, sanitisation, deterministic suffixing. 9 unit tests. |
| ✅ | sc_module_name LIFO-stack -> cx.module(name, \|m\| {..}) scope closures | DROP mechanism, REPLICATE outcome | §4 Modules | M2-08: `core/module.rs` `module`/`module_with` + `Builder` (nested modules, `m.method`/`m.thread`); `core/hierarchy.rs` `ScopeGuard` RAII push/pop. `Kernel<Building/Running>` typestate front door (M2-10). 5 tests. |
| 🟡 | Orphan-children-to-root-on-drop via arena re-parent | REPLICATE | §4 Modules | M2-02: `ObjectStore::reparent_children_to_root` (pure-id reparent) + unit test present; full destruction-order integration deferred (§12 M7+). |
| 🟡 | Interface/port/export + two-phase deferred bind + complete_binding | REPLICATE | §4 Ports; §12 M2 | M2-04/05: `channels/{interface,port,export,binding}.rs` — `Port<IF>`/`Export<IF>` Copy handles, id-keyed `PortRegistry`, two-phase `record` + `complete` (idempotent, cycle-guarded), **auto-driven at the barrier** via `BindingElaborator::end_of_elaboration` (M2-06). 10 unit tests. Not yet used by TLM sockets (M2-09). |
| ✅ | Multiports + port-policy counting | REPLICATE | §4 Ports | M2-04/05: `PortPolicy` (`OneOrMore`/`AllBound`/`ZeroOrMore`) enforced at end of `complete`; multiport flatten preserves order. Tested. |
| ✅ | Hierarchical port-to-port binding | REPLICATE | §4 Ports; §12 M2 | M2-05: `complete` flattens parent forwards depth-first (borrow-safe id-threading); port→parent-port and port→export chains tested incl. 3-deep. |
| 🟡 | Attributes (sc_attribute<T>) / AttributeStore | DEFER | §4 Modules; §6b | M2-02: `AttributeStore` type present (`core/attribute.rs`) on `ObjectMeta`; lazy get/set bodies are M2-12. |

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
| 🟡 | Convenience sockets (closure registration, no void* trampoline) | REPLICATE (adapters) | §4 TLM2; §6d | socket.rs register_b_transport/register_transport_dbg store boxed closures (Rc<dyn Fn>); memory.rs Memory::connect. Only b_transport+transport_dbg convenience; no nb convenience socket. |
| ✅ | Payload pool recycles with no leaks under stress (M3 exit) | REPLICATE | §12 M3 exit | tlm2/src/tests.rs txn_pool_recycles_and_resets verifies recycle + reset; reentrant shared target tested (shared_target_reentrant_b_transport) |
| ✅ | FIFO 'written in N, readable in N+1' rule (M3 exit) | REPLICATE | §12 M3 exit | channels/src/tests.rs exercises producer/consumer visibility discipline (4 tests pass) |

### M4 — Temporal decoupling / AT protocol / PEQ

_M4 (§3.9, §3.11, §4, §6d)_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ⬜ | Quantum keeper + global quantum | REPLICATE | §4 TLM2; §6d temporal decoupling; §12 M4 | No QuantumKeeper anywhere (only doc-comment mentions in kernel/lib.rs). systemrs-tlm-utils absent. |
| ⬜ | PEQ (peq_with_get then phase-aware, delta parity) | REPLICATE | §4 TLM2; §3.11; §12 M4 | No PEQ; depends on sc_event_queue + tlm-utils, both absent. |
| 🟡 | nb_transport_fw / nb_transport_bw + 4-phase FSM + TlmSync | REPLICATE | §4 TLM2; §3.9; §12 M4 | protocol.rs FwTransport::nb_transport_fw / BwTransport::nb_transport_bw + phase.rs Phase {BeginReq..EndResp, Extended(PhaseId)} + TlmSync enum exist as trait defaults; never wired through sockets or called (grep confirms no call sites). Contracts present, mechanism inert. |
| ⬜ | b<->nb (LT<->AT) adapters | REPLICATE (explicit adapters) | §4 TLM2; §12 M4 | No LT/AT adapters; tlm-utils absent. |
| 🟡 | DMI (get_direct_mem_ptr / invalidate, arena handle/slice) | SIMPLIFY | §4 TLM2; §3.9 | protocol.rs Dmi struct + get_direct_mem_ptr/invalidate_direct_mem_ptr trait defaults (return false / noop); not wired to sockets, never granted. |
| 🟡 | Extended phases (Phase::Extended(PhaseId) interned) | SIMPLIFY | §4 TLM2 | phase.rs Phase::Extended(PhaseId) variant defined; no interning registry, unexercised. |
| ⏸️ | Endianness helpers | DEFER | §4 TLM2 | Not implemented (deferred per §4). |
| ⏸️ | Instance-specific extensions | DEFER | §4 TLM2 | Not implemented (deferred per §4). |
| 🟡 | Two same-time zero-delay notifications fire one delta apart FIFO (M4 exit) | REPLICATE | §12 M4 exit | Delta-FIFO ordering machinery exists in kernel (timed seq + delta-event vector) but the specific M4 zero-delay parity test is not present. |

### M5 — Observability, reporting & tracing

_M5 (§3.7, §3.12, §4, §6e)_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| 🟡 | Reporting severity/action/verbosity; ERROR->Result, FATAL->abort | SIMPLIFY (essential) | §4 Support; §7; §12 M5 | diag/{severity,report,lib}.rs: Severity enum, Report, ReportError, report_info/warning + error()->ReportError + report_fatal()->! (aborts). No verbosity gating, no per-message action config. |
| ⬜ | Action precedence (pure fn matching golden table) | REPLICATE (pure fn) | §4 Support; §12 M5 exit | No action-precedence function in diag (grep: none); only fixed severity->behavior mapping. |
| ⏸️ | Cached report / per-process current-process cache | DEFER | §4 Support | Not implemented (deferred per §4). |
| ⬜ | tlm_analysis_port / tlm_write_if fan-out | REPLICATE | §4 TLM1; §3.7; §12 M5 | systemrs-tlm1 absent; no analysis ports (grep: none). |
| ⬜ | tlm_analysis_fifo (unbounded decoupler) | REPLICATE | §4 TLM1 | Not implemented. |
| ⬜ | tlm_analysis_triple (timestamped telemetry) | REPLICATE (explicit conversions) | §4 TLM1 | Not implemented. |
| ⬜ | tlm_fifo + put/get/peek (TLM-1 message passing) | REPLICATE | §4 TLM1; §3.7 | systemrs-tlm1 absent. (Note: a primitive-channel Fifo exists in systemrs-channels, but the TLM-1 tlm_fifo/peek API is separate and not built.) |
| ⬜ | tlm_transport_if (one required method + default) | SIMPLIFY | §4 TLM1 | Not implemented. |
| 🟡 | Tracing via stage callbacks (sample after update commits) | REPLICATE (sampling discipline) | §4 Support; §3.12, §6e; §12 M5 | Stage enum (PreTimestep/PostUpdate) defined in kernel/phase.rs; sampling hook point identified (sim.rs:432) but no recorder/sink and not invoked. systemrs-trace absent. |
| ⬜ | VCD/FST -> transaction-centric sink | SIMPLIFY -> transaction sink | §4 Support; §3.12 | No trace sink of any kind; systemrs-trace absent. |
| ⬜ | Off-thread telemetry writer | REPLICATE | §12 M5; §6e | Not implemented. |
| ✅ | transport_dbg query API (twin inspection) | REPLICATE | §12 M5 | tlm2/socket.rs transport_dbg path + memory.rs register_transport_dbg; tested. (The transport_dbg primitive itself is present even though the broader M5 layer is not.) |
| ⬜ | sc_vector -> Vec<T> + scoped builder | SIMPLIFY | §4 Support | No sc_vector analogue / scoped builder (plain Vec used ad hoc). |
| ❌ | circular_buffer raw storage | DROP | §4 TLM1 | Use VecDeque per §4; FIFO uses VecDeque. |
| ❌ | tlm_tag<T> | DROP | §4 TLM1 | Unnecessary in Rust; absent. |
| ❌ | sc_dt datatypes (~58k LOC) | DROP | §4 Support | Out of scope (CLAUDE.md + §4); native ints + [u8] used instead. |

### M6 — Digital-twin layer

_M6 (§6f, §12)_

| | Feature | Decision | Design | Evidence / Notes |
|---|---|---|---|---|
| ⬜ | RealTimePacer (wall-clock pacing + slip telemetry) | DEFER (post-MVP feature) | §6f; §12 M6 | Not implemented (grep: no RealTimePacer/pace). |
| ⬜ | ExternalInput inbox + suspend-on-starvation (park, don't exit) | DEFER | §6f; §12 M6 | Not implemented (grep: no ExternalInput). |
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
| ⬜ | Phase 1: Rust models as guests inside C++ SystemC kernel (cxx) | REPLICATE (first deliverable) | §11.2; §10.1 ffi | systemrs-ffi crate absent; no cxx bridge. Dockerfile/devcontainer prep SSH/toolchain but cosim feature not wired (commit history: tlm2 transport landed, no ffi). |
| ⬜ | Payload marshaling & ownership across FFI | REPLICATE | §11.3 | No FFI marshaling code. |
| ⏸️ | Phase 2: C++ guests inside Rust kernel | DEFER | §11.4; §11 | Phased after Phase 1; not started. |
| ⏸️ | Phase 3: out-of-process quantum-synchronized co-sim | DEFER | §11.5 | Not started (and depends on quantum keeper, also absent). |
| ⬜ | Symmetric panic/exception firewall (catch_unwind at extern C; C++ try/catch around re-entry) | REPLICATE | §11.2, §11.6; CLAUDE.md interop | No catch_unwind / extern C entry points anywhere (grep: none); firewall is an FFI-path concern and FFI is absent. |
| ✅ | Single-scheduler invariant (reject two live kernels in one process) | REPLICATE (constraint) | §11.1 | Architecturally upheld: only the single-threaded systemrs-kernel scheduler exists; no second kernel can be instantiated. (Thread-local CURRENT_SIM in ctx.rs enforces one current sim per thread.) |

