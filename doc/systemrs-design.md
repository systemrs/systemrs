# SystemRS: Design Report for a Rust, TLM-Only Equivalent of SystemC

**A discrete-event modeling framework in Rust for transaction-level digital twins, with first-class SystemC (C++) interoperability.**

> **Abstract.** SystemRS is a proposed Rust framework that reproduces the parts of SystemC and TLM-2.0 needed to author *digital twins* of digital systems at transaction level, while deliberately dropping the RTL-oriented machinery that a transaction-level tool does not need. It faithfully ports SystemC's single-threaded, cooperative, three-phase delta-cycle scheduler — the determinism contract on which all TLM behaviour rests — and layers idiomatic Rust on top of it: stackful coroutines for `SC_THREAD`-style processes, an arena-and-generational-id object store instead of a raw-pointer graph, sum types instead of signed-integer conventions, and `TypeId` maps instead of RTTI. It specifies the TLM-2.0 generic payload, the four-phase non-blocking transport handshake, sockets, and temporal decoupling, and it adds the subsystems a *twin* requires that SystemC lacks: real-time pacing, external-input gating without starvation exit, deterministic replay, snapshot/restore, and an off-thread telemetry plane. This document catalogues SystemC's features by subsystem, records an explicit REPLICATE / SIMPLIFY / DEFER / DROP decision for each, presents the concurrency core and the full crate decomposition, analyses parallelism (both of the running simulation and of the engineering effort) and an ECS-style data architecture, lays out a phased interoperability strategy with SystemC, and gives an MVP roadmap with exit criteria and a risk register. It is written for the engineers and stakeholders deciding whether and how to build SystemRS.

---

## 1. Executive summary

- **What it is.** SystemRS is a Rust re-implementation of SystemC's modeling substrate, restricted to the *transaction-level modeling* (TLM) subset and oriented toward long-lived, observable digital twins. It is *not* an RTL simulator: the ~58k-LOC `sc_dt` numeric/bit/fixed-point library, resolved multi-driver signals, and clocked threads (`SC_CTHREAD`) are out of scope.
- **Why.** A faithful Rust TLM kernel gives memory safety, fearless refactoring, an arena-based object model that eliminates whole classes of C++ destruction-order and use-after-free bugs, and a clean seam for the twin-specific features (pacing, replay, snapshot, telemetry) SystemC was never designed for.
- **The core technical bet — concurrency.** SystemRS keeps the scheduler **single-threaded and cooperatively non-preemptive** (exactly one process runs at a time) and ports the **strict three-phase delta cycle (evaluate → update → notify)** bit-for-bit. `SC_THREAD` processes are **stackful coroutines** (via `corosensei`), *not* `async fn`, so `wait()` is callable from arbitrary call depth — including deep inside `b_transport` — without "async colouring" infecting every user function. `SC_METHOD` is a plain run-to-completion closure.
- **Determinism is the product.** The evaluate/update split, the immediate > delta > timed notification-collapse rules, the subscriber-trigger ordering, and the `change_stamp`/`delta_count` accounting are reproduced exactly. Tie-breaks that C++ leaves implementation-defined are made *explicit* (insertion sequence numbers), so SystemRS runs are reproducible and replayable.
- **Ownership model.** All processes, events, channels, and objects live in kernel-owned **arenas keyed by generational ids** (`ProcessId`, `EventId`, `ChannelId`, `ObjectId`). Components refer to each other by `Copy` id, never by reference — dissolving SystemC's raw-pointer object graph and reference-counted handles, and sidestepping the borrow checker. Channel value buffers use `Cell`-based double-buffering; the synchronous core uses `Rc`/`RefCell`, never `Arc`/`Mutex`.
- **TLM-2.0 contracts preserved, mechanisms modernized.** The generic payload, four-phase FSM, `tlm_sync_enum` semantics, timing-annotation convention, DMI, sockets, and PEQ delta-parity ordering are kept faithfully; their *implementation* (raw pointers, RTTI, intrusive refcounts, `void*` trampolines) is replaced by Rust ownership, sum types, `TypeId` maps, and arena handles. Pooled payloads become `Rc<RefCell<GenericPayload>>`.
- **Internal data architecture is ECS-flavoured, not ECS-exposed.** The arena store is extended *internally* with columnar component storage and a conflict-checked deterministic scheduler — invisible to model authors, who keep an object-oriented surface. This is what makes snapshot, replay, and live introspection mechanical rather than impossible.
- **Parallelism follows the quantum.** The single-threaded delta cycle is the unit of determinism; the TLM quantum is the unit of parallelism. Optional conservative (barrier-synchronous) parallel discrete-event simulation runs disjoint regions in parallel and re-converges at quantum boundaries, preserving deterministic replay. `rayon` handles embarrassingly-parallel telemetry/memory work off the critical path.
- **Interop is phased.** Phase 1: Rust models run as *guests inside the proven C++ SystemC kernel* (one scheduler, native determinism) via a `cxx` bridge over a de-templated C++ shim. Phase 2: C++ models guest inside the Rust kernel once it is bit-faithful. Phase 3: out-of-process, quantum-synchronized co-simulation (shared memory or gRPC) for crash isolation and scaling. The two-live-kernels-in-one-process design is rejected.
- **Crate set.** A 14-crate Cargo workspace: leaf crates `systemrs-diag`, `systemrs-time`, `systemrs-runtime` (coroutine backend), `systemrs-macros`; the `systemrs-kernel`; modeling layers `systemrs-core`, `systemrs-channels`; `systemrs-tlm1`, `systemrs-tlm2`, `systemrs-tlm-utils`; `systemrs-trace`; the `systemrs` facade; `systemrs-ffi`; and `systemrs-examples`. The RTL datatypes library is deliberately *not* a crate.

---

## 2. Background: what SystemC is, and what "TLM-only" means

**SystemC** is an IEEE-1666 C++ class library and simulation kernel for modeling digital systems above the register-transfer level. Its core is a **discrete-event simulator**: a single-threaded, cooperative scheduler (`sc_simcontext`) drives an elaboration → simulation lifecycle, advancing logical time through *delta cycles* and *timed notifications*. On top of the kernel sit structural primitives (`sc_module`, `sc_port`, `sc_export`, `sc_signal`, `sc_fifo`, `sc_clock`) and two transaction-level layers: **TLM-1.0** (message-passing put/get/peek interfaces and analysis ports) and **TLM-2.0** (the memory-mapped bus modeling standard, built on the *generic payload*, *sockets*, a four-phase non-blocking protocol, blocking transport with timing annotation, DMI, and temporal decoupling via a global quantum).

**"TLM-only"** means SystemRS targets the transaction-level subset and abandons the RTL-oriented parts. Concretely, in-scope: the kernel and scheduler, processes and events, modules/ports/exports/interfaces, the deterministic primitive channels (signal/fifo/clock/mutex/semaphore), all of TLM-1 (especially the analysis fan-out used for telemetry) and TLM-2 (payload, transport, sockets, quantum, PEQ). Out-of-scope: the bit-true RTL machinery — the `sc_dt` library (`sc_int`/`sc_uint`/`sc_bigint`/`sc_biguint`/`sc_logic`/`sc_bv`/`sc_lv`/`sc_fixed`), resolved multi-driver signals, and clocked threads (`SC_CTHREAD`). TLM-2.0 payloads carry native integers plus raw byte buffers (`unsigned char*` data and byte-enable arrays), so none of the ~58k LOC of arbitrary-precision/4-valued arithmetic is needed for transaction modeling.

A **digital twin** at transaction level is a long-lived, observable, sometimes wall-clock-coupled, reconfigurable model of a real system. SystemC was designed as a *batch simulator*; a twin needs more: it must track or scale against real time, accept external inputs between timesteps without exiting on starvation, reproduce a run deterministically for forensic replay, snapshot and restore state, and stream high-volume telemetry without perturbing timing. These twin-specific needs (Section 6f) are first-class subsystems in SystemRS, layered on the same deterministic core.

---

## 3. Key features of SystemC, by subsystem

Each subsection states *what it is*, the *must-preserve semantics* that define trace-equivalence, and a *relevance verdict* for a TLM-only twin.

### 3.1 Simulation kernel & scheduler

**What it is.** `sc_simcontext` is the god-object owning all global simulation state: runnable queues, the delta-event vector, the timed-event priority queue, current time, the delta/change counters, the execution phase, and the simulation status. It runs the elaboration → start-of-simulation → repeated delta-cycle → end-of-simulation lifecycle via `crunch()` (one delta cycle) and `simulate()` (delta groups across time). Time is a 64-bit unsigned count of resolution units (default resolution 1 ps), with `sc_time::max()` as the infinity sentinel. That sentinel is *literally* all-ones: the `max_time_tag` constructor (`sc_time.h:254-256`) initialises `m_value( ~value_type{} )`, so it is bit-for-bit `u64::MAX` and SystemRS's `Time::INF = Time(u64::MAX)` is an exact (not merely semantic) match.

**Must-preserve semantics.**
- The **delta cycle is strictly three phases in order: (1) EVALUATE** — run all runnable processes to completion (methods first, then threads, via the `toggle_methods`/`toggle_threads` push/pop double-buffer); **(2) UPDATE** — apply pending primitive-channel updates; **(3) DELTA NOTIFY** — fire delta-notified events, queueing processes for the *next* delta. The evaluate-then-update separation is the determinism guarantee: signal writes during evaluate are buffered and only become visible after update, so process execution order within a delta cannot affect read values.
- **Three notification kinds** with distinct timing: *immediate* (`notify()`, legal only in evaluate phase, fires now), *delta* (`notify(SC_ZERO_TIME)`, fires at end of current delta), *timed* (`notify(t>0)`, fires after time advances). Priority **immediate > delta > timed**; an event holds at most one pending notification and the **soonest wins**.
- **`change_stamp` vs `delta_count`** are distinct counters with precise increment points. `m_change_stamp` is bumped in *two* places: at the top of every non-empty update phase, *before* `perform_update()` so `event_occurred()` sees the new stamp (`sc_simcontext.cpp:564-567`), **and** on every time advance in `do_timestep` (`sc_simcontext.cpp:986`). Both `m_change_stamp` and `m_delta_count` are guarded by `!empty_eval_phase`, so an empty evaluate phase advances *neither* counter (`sc_simcontext.cpp:566,614`). They underpin `triggered()`/`event_occurred()`.
- Time is monotonic; `do_timestep` asserts `m_curr_time < t` (`sc_simcontext.cpp:975`). Starvation policy (`SC_RUN_TO_TIME` vs `SC_EXIT_ON_STARVATION`) governs whether time advances to the requested point with no events.

**Verdict: essential — REPLICATE.** TLM is built directly on this kernel; the delta-cycle and notification semantics directly determine whether two TLM models produce identical traces.

### 3.2 Processes & coroutines

**What it is.** Three process kinds — `SC_METHOD` (run-to-completion callback on the kernel stack), `SC_THREAD` (stackful coroutine with its own stack that cooperatively yields at `wait()`), and `SC_CTHREAD` (clocked thread) — plus dynamic spawning (`sc_spawn`), reference-counted process handles, process control (suspend/resume/disable/enable/kill/reset/throw), join synchronization, and pluggable coroutine backends (QuickThreads, Win32 fibers, pthreads, std::thread).

**Must-preserve semantics.**
- **Exactly one process runs at a time.** Even the OS-thread backends serialize with a mutex+condvar handoff. This is cooperative, not parallel.
- `SC_METHOD`s run to completion on the kernel stack and never block; `next_trigger()` only records the next-invocation sensitivity. `SC_THREAD`s yield via `suspend_me()` and, on resume, check a throw-status state machine (`THROW_NONE`/`KILL`/`USER`/`ASYNC_RESET`/`SYNC_RESET`).
- The **immediate self-notification guard** is a *per-process* predicate, not a single kernel-level flag. In `trigger_static()`/`trigger_dynamic()` (`sc_thread_process.h:474-510`; the method-process analogue) a thread/method is added to the run queue only if it is runnable, not already queued, and still expecting this trigger — and crucially `if ( sc_get_current_process_b() == this ) { report_immediate_self_notification(); return; }`. So a process that immediately-notifies an event it is itself statically/dynamically sensitive to (including a thread suspended on that event via dynamic `wait`) is *not* re-made-runnable in the same evaluation; it is the *currently-running* process that is skipped, not merely "the running process by id equality" in the abstract.
- Kill/reset are delivered by throwing an unwind exception across the coroutine stack — the genuinely hard piece to port faithfully.

**Verdict: essential — REPLICATE (with the coroutine backend reduced to one and full kill/reset deferred).** The process/coroutine model is the execution substrate for all of TLM, including `b_transport` blocking and `nb_transport` phase progression.

### 3.3 Events, notification & sensitivity

**What it is.** `sc_event` is the fundamental synchronization object: four subscriber lists (static/dynamic × method/thread), a single pending-notification state machine, immediate/delta/timed notification, cancellation, and a `trigger()` that walks the subscriber lists making processes runnable. Static sensitivity is declared at elaboration via the `sensitive <<` DSL; dynamic sensitivity is `wait()`/`next_trigger()` with event AND/OR lists, timeouts, and event finders for late port binding.

**Must-preserve semantics.**
- **Notification collapse:** at most one pending notification per event, earliest wins; a pending delta makes a later timed `notify` a no-op; delta overrides timed.
- **Immediate notification is legal only in the evaluate phase.**
- **`trigger()` ordering** is observable and verified against `sc_event.cpp:378-458`: the four subscriber lists are walked in the fixed order **static methods → dynamic methods → static threads → dynamic threads**. The iteration *direction within each list* is also load-bearing: the two *static* lists are walked **high-index → 0** (`int i = size - 1; do {…} while( --i >= 0 )`), while the two *dynamic* lists are walked **0 → high-index** with consumed entries swapped in from the tail (`for (i = 0; i <= last_i; i++) { if (trigger_dynamic(this)) { l[i] = l[last_i]; last_i--; i--; } }` then `resize(last_i+1)`). Separately, the *delta-event vector itself* is drained high-index-to-low at the end of the delta (`sc_simcontext.cpp` notify phase). A faithful port must reproduce all of this for bit-exact behaviour.
- `triggered()` is `m_trigger_stamp == change_stamp` — true only within the firing change-stamp window.
- Dynamic-sensitivity state machine: EVENT (first fire), OR_LIST (first member fires, unsubscribe the rest), AND_LIST (decrement count, fire at zero), TIMEOUT variants (whichever fires first wins, cancel the sibling).

**Verdict: essential — REPLICATE.** Events and notification are the absolute core; TLM blocking transport, the nb four-phase sequence, and PEQs are all built on these primitives.

### 3.4 Modules, objects & elaboration

**What it is.** `sc_object` is the abstract base of every named entity (modules, processes, ports, events), carrying a hierarchical name and a parent back-pointer. `sc_module` is the hierarchical container users subclass. A strict two-phase lifecycle (elaboration builds the static hierarchy, then simulation runs) is marked by four virtual callbacks: `before_end_of_elaboration`, `end_of_elaboration`, `start_of_simulation`, `end_of_simulation`. Construction uses an `sc_module_name` LIFO stack so a module can discover its own name positionally.

**Must-preserve semantics.**
- Phase transitions fire in a fixed global order; the inter-registry order is port → export → prim_channel → module for each callback.
- `construction_done` is iterated to a fixpoint, so modules created inside `before_end_of_elaboration` also receive that callback.
- Hierarchical names are `.`-joined, sanitized, and de-duplicated with a warning.
- The static structural hierarchy is immutable once simulation starts.

**Verdict: essential — REPLICATE (the outcome; replace the name-stack mechanism).** Every TLM model is built from `sc_module` instances wired during elaboration; the lifecycle barrier is exactly when socket binding is validated.

### 3.5 Ports, exports & interfaces

**What it is.** `sc_interface` is the abstract base all channel interfaces inherit; `sc_port<IF>` is a required-interface endpoint resolving to one-or-more `IF*` and forwarding interface-method calls; `sc_export<IF>` re-publishes a provided interface upward. Binding is deferred to a two-phase elaboration resolution; event finders defer "which event of the channel behind this port" until binding completes.

**Must-preserve semantics.**
- **Two-phase deferred binding:** `bind()` only records a bind element; `complete_binding()` at end of elaboration resolves connectivity (recursively, depth-first, flattening parent multiports).
- Binding is legal only during elaboration.
- Interface ordering is preserved and deterministic; index 0 is the canonical interface.
- Port-policy enforcement (`SC_ONE_OR_MORE_BOUND`/`SC_ALL_BOUND`/`SC_ZERO_OR_MORE_BOUND`) happens at the end of `complete_binding`.

**Verdict: essential — REPLICATE.** TLM-2.0 initiator/target sockets *are* `sc_port`/`sc_export` pairs over the fw/bw transport interfaces; multi-sockets are unbounded multiports.

### 3.6 Primitive channels

**What it is.** `sc_prim_channel` is the base that defers side effects to the update phase via `request_update()`/`update()`. Concrete channels: `sc_signal`/`sc_buffer` (deterministic double-buffered value channels), `sc_fifo` (bounded blocking queue), `sc_clock` (self-driven periodic signal), resolved signals (multi-driver), plus the non-prim `sc_mutex`/`sc_semaphore`.

**Must-preserve semantics.**
- **Evaluate-then-update determinism:** `read()` returns the value committed at the previous update; `write()` only stages into a new-value buffer and calls `request_update()`. Reads in delta N see the value written through update at the end of delta N−1.
- `request_update()` is idempotent within a delta; updates requested *during* the update phase defer to the next delta.
- `value_changed_event` fires in the *next* delta (one-delta-delayed notification), with the change stamped.
- `sc_signal` skips the update if the value is unchanged; `sc_buffer` always fires an event — an observable distinction.
- `sc_mutex`/`sc_semaphore` use *immediate* notification (a hazard) and mutate state synchronously.

**Verdict: essential — REPLICATE (signal/fifo/clock/mutex/semaphore); DROP resolved signals.** The evaluate/update split and the `request_update`/`update` mechanism are the foundation of SystemC determinism and the template for any TLM channel.

### 3.7 TLM-1.0 message passing & analysis ports

**What it is.** A family of value-templated interfaces (transport, put/get/peek, blocking and non-blocking) plus a reference channel `tlm_fifo`, and an analysis sublayer (`tlm_write_if`/`tlm_analysis_port`/`tlm_analysis_fifo`) implementing one-way, non-blocking, fan-out broadcast.

**Must-preserve semantics.**
- `tlm_fifo` honours the evaluate/update discipline: a value put in delta N is not gettable until N+1; events are delta-deferred; blocking `get`/`put` are `while`-loops.
- **Analysis `write()` is synchronous, immediate, in-order, fan-out, with no back-pressure**: the producer's call synchronously delivers to every subscriber in registration order, same delta. `tlm_analysis_fifo` is unbounded so telemetry never stalls the model.

**Verdict: essential for twins — REPLICATE.** The analysis-port broadcast is exactly the mechanism a digital twin needs for non-intrusive telemetry/observability.

### 3.8 TLM-2.0 generic payload

**What it is.** The universal transaction object: command (read/write/ignore), address, a caller-owned data buffer + length, an optional byte-enable mask, streaming width, response status, DMI hint, a type-indexed extension array, and an optional memory manager with reference counting for pooling.

**Must-preserve semantics.**
- **Buffer ownership is split from object ownership.** The payload *never* allocates or frees the data or byte-enable buffers; the initiator owns them and they must outlive the call chain. The destructor frees only extensions.
- **Memory management is opt-in.** Without an MM the GP is a plain owned object; with an MM it is reference-counted and recycled via `free()` at count 0. The initiator must hold a reference for the whole transaction.
- Extension indices are assigned globally and deterministically by first-registration order; three removal semantics (`clear`/`release`/`free_all`) must stay distinct.
- Response status is a signed enum (`tlm_gp.h:96-103`), but it is **not** simply "positive = OK, non-positive = error." The exact mapping is: `TLM_OK_RESPONSE = 1` is the *sole* OK value; `TLM_INCOMPLETE_RESPONSE = 0` is the *initial, not-yet-processed* state and is **not** an error; the five error values are strictly negative — `TLM_GENERIC_ERROR (-1)`, `TLM_ADDRESS_ERROR (-2)`, `TLM_COMMAND_ERROR (-3)`, `TLM_BURST_ERROR (-4)`, `TLM_BYTE_ENABLE_ERROR (-5)`. The "is-error" test is therefore *discriminant `< 0`* (equivalently, `!OK && !INCOMPLETE`), which is exactly what SystemRS's `is_error()` computes — so the older shorthand "≤0 is error" was wrong about `INCOMPLETE = 0`. Byte-enables repeat modulo the byte-enable length.

**Verdict: essential — REPLICATE (idiomatic).** The generic payload IS the TLM-2.0 transaction object and the interoperability cornerstone.

### 3.9 TLM-2.0 transport interfaces & phases

**What it is.** Three transport flavours — blocking `b_transport` with timing annotation, non-blocking `nb_transport_fw`/`nb_transport_bw` with a four-phase handshake, untimed `transport_dbg` — plus DMI (`get_direct_mem_ptr`/`invalidate_direct_mem_ptr`) and the `tlm_sync_enum` return convention. The forward and backward interfaces are templated on a protocol-types traits struct.

**Must-preserve semantics.**
- **Phase ordering** is strictly BEGIN_REQ → END_REQ → BEGIN_RESP → END_RESP; any transition may be conveyed by either fw or bw depending on who advances it.
- **`tlm_sync_enum`:** `TLM_ACCEPTED` (callee unchanged, await opposite path), `TLM_UPDATED` (phase advanced synchronously), `TLM_COMPLETED` (early completion).
- **Timing annotation `t`** is a delay relative to `sc_time_stamp()`, not absolute; the callee may increase it; the initiator owes a `wait(t)`.
- The **same transaction object** is aliased forward and backward and mutated in place — no copy.
- `transport_dbg` must be side-effect-free, wait-free, callable outside the scheduler. DMI carries the hard rule that `get_direct_mem_ptr` must not be called inside `invalidate_direct_mem_ptr`.

**Verdict: essential — REPLICATE.** These are the central abstractions of any TLM-2.0 framework.

### 3.10 TLM-2.0 sockets

**What it is.** A socket bundles a forward and a backward transport path into one bind-able object. The core `tlm_initiator_socket`/`tlm_target_socket` templates pair an `sc_port` with an `sc_export`, carry a compile-time bus width and a protocol tag. The `tlm_utils` convenience sockets (simple/passthrough/multi) add ergonomic callback registration; `simple_target_socket` additionally synthesizes LT↔AT conversion.

**Must-preserve semantics.**
- **Bind is a crossed double-binding:** initiator.port → target.export (fw) *and* target.port → initiator.export (bw), wired atomically.
- Bus width is advisory metadata, not enforced; protocol compatibility is checked by type identity.
- Hierarchical binds (initiator→initiator, target→target) and multi-socket category rules (rejecting multi-target into non-multi-target).
- The b↔nb conversion uses spawned threads, per-transaction events, and a pending-transaction map keyed by transaction identity; `SC_ZERO_TIME` (delta) vs annotated-time notifications are load-bearing.

**Verdict: essential — REPLICATE.** Sockets are THE user-facing connection primitive.

### 3.11 TLM-2.0 temporal decoupling & PEQ

**What it is.** The quantum keeper (`tlm_quantumkeeper`) and global quantum (`tlm_global_quantum`) implement temporal decoupling for fast LT models: an initiator accumulates a private local time and runs ahead, syncing back to the kernel only at quantum boundaries. Payload event queues (`peq_with_cb_and_phase`, `peq_with_get`) provide kernel-integrated, time-ordered phase scheduling for AT models.

**Must-preserve semantics.**
- Local time and kernel time are distinct; folded together only at `sync()` (which calls `wait(local_time)`).
- `need_sync()` uses `>=`; sync points are grid-aligned via `q - (now % q)`.
- The PEQ routes a zero-time notify made during a phase callback into the *next* delta cycle (even/odd-parity bucketing), preserving evaluate-then-update determinism; a three-tier drain order (immediate → current-parity delta → timed-now) is observable.
- PEQs store raw pointers and do not own the payload.

**Verdict: essential — REPLICATE.** Temporal decoupling is THE LT performance mechanism; PEQs are the standard AT phase-sequencing idiom.

### 3.12 Support: reporting, tracing, vectors, datatypes

**What it is.** `sc_report`/`sc_report_handler` (severity/action/verbosity diagnostics, and `sc_report` is also a thrown exception); tracing (VCD/WIF value-change dumps driven by stage callbacks); `sc_vector` (hierarchy-aware named child arrays); and the `sc_dt` datatypes library.

**Must-preserve semantics.**
- Report action resolution has a strict precedence (per-type-per-severity > per-type > per-severity, then suppress, then force); default actions per severity are load-bearing (ERROR throws, FATAL aborts).
- Verbosity gates INFO reports.
- Tracing samples are driven by stage callbacks at `SC_PRE_TIMESTEP` (and `SC_POST_UPDATE` for delta tracing) — *after* the update phase commits new values.

**Verdict: mixed.** Reporting is **essential — SIMPLIFY** (needed from day one, far simpler in Rust). Tracing is **useful — SIMPLIFY** to a transaction-centric sink. `sc_vector` is **useful — SIMPLIFY**. The `sc_dt` datatypes library is **SKIP/DROP** — ~58k LOC of RTL arithmetic the generic payload does not use.

---

## 4. Feature-coverage decisions for SystemRS

Legend: **REPLICATE** = reproduce semantics faithfully (trace-equivalence target); **SIMPLIFY** = keep the capability in a reduced/idiomatic form, divergence documented; **DEFER** = out of MVP, design must not preclude it; **DROP** = explicitly out of scope.

### Kernel & scheduling

| Feature | Decision | Rationale |
|---|---|---|
| Three-phase delta cycle (evaluate/update/notify) | **REPLICATE** | The determinism contract. |
| Timed-event wheel, time advance, starvation policy | **REPLICATE** | LT decoupling and AT scheduling ride on timed notifications. |
| Immediate / delta / timed notification + collapse | **REPLICATE** | Trace-equivalence requires the exact state machine. |
| `change_stamp` / `delta_count` counters | **REPLICATE** | Underpin `triggered()`, signal `event()`, PEQ parity. |
| `sc_time` (64-bit unit count) + resolution | **REPLICATE (as construction param)** | Integer time mandatory; replace freeze-on-first-use global with a builder/const value. |
| `sc_start`/`stop`/`pause` | **SIMPLIFY** | Keep run/stop/pause via typestate; drop dual stop-mode subtlety initially. |
| `sc_suspend_all` / `sc_unsuspend_all` | **DEFER** | Useful for cosim gating; design `next_time()` to allow a suspend hook. |
| Stage/phase callbacks | **SIMPLIFY** | Keep `PreTimestep`/`PostUpdate` (tracing needs them); drop the full bitmask taxonomy. |
| `preempt_with` nested execution | **DROP (MVP), DEFER** | Only needed for synchronous kill/reset; not required for clean TLM. |
| Deprecated APIs | **DROP** | Dead weight. |

### Processes & coroutines

| Feature | Decision | Rationale |
|---|---|---|
| `SC_METHOD` (run-to-completion) | **REPLICATE** | Plain `FnMut`; no stack. |
| `SC_THREAD` + `wait()` from arbitrary depth | **REPLICATE (stackful coroutine)** | `b_transport` must call `wait` deep in the call tree. |
| `next_trigger()` dynamic sensitivity | **REPLICATE** | Core method idiom. |
| `SC_CTHREAD` (clocked threads) | **DROP** | Pure RTL/clocked construct. |
| `sc_spawn` / `sc_spawn_options` | **SIMPLIFY** | `Box<dyn FnMut()>` + options; drop `sc_bind`/`sc_ref`/`sc_cref`. |
| `sc_join` / fork-join | **SIMPLIFY** | Counter + event, exposed as `join_all`. |
| suspend/resume/disable/enable | **DEFER** | Testbench control, not core. |
| kill / reset / throw_it | **SIMPLIFY → DEFER (full semantics)** | MVP: cooperative cancellation; synchronous-throw deferred and documented. |
| Reset-signal machinery | **DROP** | RTL concept. |
| Multiple coroutine backends | **SIMPLIFY** | One stackful backend (`corosensei`); drop OS-thread emulation. |
| Immediate self-notification guard | **REPLICATE** | Cheap; prevents real non-determinism. |

### Events & sensitivity

| Feature | Decision | Rationale |
|---|---|---|
| `sc_event` + notify/cancel/trigger + collapse | **REPLICATE** | Foundation. |
| AND/OR lists, timeouts, `wait(t, ev)` | **REPLICATE** | Needed by AT and FIFO/PEQ idioms. |
| Expression-template `&`/`\|` syntax | **SIMPLIFY** | `BitAnd`/`BitOr` on event refs returning owned lists. |
| `sc_event_queue` (lossless) | **REPLICATE (as a channel)** | PEQs rely on "every notify observable". |
| `sc_event_finder` | **SIMPLIFY** | Closure/selector enum resolved at bind. |
| `sensitive <<` DSL | **SIMPLIFY** | Explicit process builder; no hidden last-process state. |

### Modules, hierarchy, objects

| Feature | Decision | Rationale |
|---|---|---|
| Object hierarchy + naming + uniqueness | **REPLICATE** | Names are the introspection key. |
| Four lifecycle callbacks + construction fixpoint | **REPLICATE** | Sockets validate binding here. |
| `sc_module_name` LIFO-stack discovery | **DROP mechanism, REPLICATE outcome** | Use `cx.module("name", \|m\| {…})` scope closures. |
| `SC_MODULE`/`SC_CTOR` macros | **SIMPLIFY** | `#[module]` proc-macro. |
| Attributes (`sc_attribute<T>`) | **DEFER** | `AttributeStore` when needed. |
| Orphan-children-to-root-on-drop | **REPLICATE via arena** | Re-parent of indices. |

### Ports, exports, channels

| Feature | Decision | Rationale |
|---|---|---|
| Interface/port/export + two-phase bind | **REPLICATE** | Sockets are port+export pairs. |
| Multiports, port-policy counting | **REPLICATE** | Multi-passthrough sockets are unbounded multiports. |
| Hierarchical port-to-port binding | **REPLICATE** | Nested TLM platforms. |
| `sc_signal`/`sc_buffer` | **SIMPLIFY → keep** | Reset/IRQ glue lines; keep `bool`/`int` only. |
| Signal posedge/negedge events | **SIMPLIFY** | Keep edge events (IRQ); drop clocked-reset hookup. |
| `sc_clock` | **SIMPLIFY → DEFER** | Self-scheduling process if needed; twins use the quantum. |
| `sc_fifo` | **REPLICATE** | Canonical bounded blocking channel. |
| `sc_mutex`/`sc_semaphore` | **SIMPLIFY** | Keep; note immediate-notify hazard. |
| Resolved signals | **DROP** | RTL multi-driver bus. |
| Writer policy | **SIMPLIFY** | Runtime enum check in strict mode. |

### TLM-2.0

| Feature | Decision | Rationale |
|---|---|---|
| Generic payload | **REPLICATE** | The interoperability object. |
| MM (acquire/release/`tlm_mm_interface`) | **SIMPLIFY** | `Rc<Payload>` + pool. |
| Extensions | **REPLICATE (idiomatic)** | Anymap keyed by `TypeId`. |
| `b_transport` + timing annotation | **REPLICATE** | LT workhorse. |
| `nb_transport_fw/bw` + 4-phase + sync enum | **REPLICATE** | AT base protocol. |
| `transport_dbg` | **REPLICATE** | Backdoor peek/poke for twin inspection. |
| DMI | **SIMPLIFY** | Keep interface; model backdoor as arena handle/slice. |
| Sockets (initiator/target/hierarchical/multi) | **REPLICATE** | User-facing connection primitive. |
| Convenience sockets | **REPLICATE (as adapters)** | Define the ergonomics modelers touch. |
| LT↔AT conversion | **REPLICATE (explicit adapters)** | Distinguishes correct TLM from a toy. |
| Endianness helpers | **DEFER** | Initiator convenience, re-express with const generics later. |
| Instance-specific extensions | **DEFER** | Secondary utility. |
| Extended phases | **SIMPLIFY** | `Phase::Extended(PhaseId)` interned. |

### TLM-1.0 & analysis

| Feature | Decision | Rationale |
|---|---|---|
| `tlm_fifo` + put/get/peek | **REPLICATE** | Evaluate/update visibility discipline. |
| `circular_buffer` raw storage | **DROP** | Use `VecDeque<T>`. |
| `tlm_transport_if` | **SIMPLIFY** | One required method + default. |
| `tlm_analysis_port`/`tlm_write_if` fan-out | **REPLICATE** | Telemetry backbone. |
| `tlm_analysis_fifo` | **REPLICATE** | Unbounded decoupler. |
| `tlm_analysis_triple` | **REPLICATE (explicit conversions)** | Timestamped telemetry. |
| `tlm_tag<T>` | **DROP** | Unnecessary in Rust. |

### Support

| Feature | Decision | Rationale |
|---|---|---|
| Reporting (severity/action/verbosity) | **SIMPLIFY (essential)** | `tracing` + typed `Result`; ERROR→`Result`, FATAL→abort. |
| Action precedence | **REPLICATE (pure fn)** | Compatibility; testable. |
| Cached report / per-process | **DEFER** | Task-local current-process cache later. |
| Tracing via stage callbacks | **REPLICATE (sampling discipline)** | Sample after update commits. |
| VCD bit-level format | **SIMPLIFY → transaction-centric sink** | Twins want transaction records; optional VCD/FST later. |
| `sc_vector` | **SIMPLIFY** | `Vec<T>` + scoped builder. |
| `sc_dt` datatypes | **DROP (~58k LOC)** | Native ints + `[u8]` suffice. |

---

## 5. From catalogue to design: the load-bearing principles

The catalogue (§3) and the REPLICATE/SIMPLIFY/DEFER/DROP decisions (§4) do not translate into a design one feature at a time; they converge on a small set of cross-cutting principles that the rest of the report (§6 onward) elaborates. This section names them explicitly so the design that follows is read as one coherent system rather than eight independent ports.

1. **The determinism contract is the product, and it is non-negotiable.** Everything marked REPLICATE in §4 exists to make two runs trace-identical: the three-phase evaluate→update→delta-notify cycle, the immediate>delta>timed notification collapse, the `change_stamp`/`delta_count` accounting (including the empty-delta guard and the time-advance bump, §3.1), and the verified `trigger()` subscriber ordering (§3.3). These are reproduced *bit-for-bit* and are the one place SystemRS deviates *zero* from SystemC (§7).

2. **Where C++ leaves order implementation-defined, SystemRS pins it.** Heap-defined equal-time ordering becomes explicit insertion-`seq` FIFO order; HashMap iteration never feeds a tie-break. This *strengthens* determinism beyond IEEE-1666 rather than weakening it (§6a, §8a Tier-0/1).

3. **Ownership is dissolved into arenas keyed by `Copy` generational ids.** SystemC's raw-pointer object graph and intrusive refcounts (§3.2, §3.4, §3.10) all become `SlotMap`-backed arenas with id handles. This single decision neutralises the borrow-checker fight, the initiator/target bind cycle, destruction-order hazards, and the snapshot problem at once — and is stated once in §6a, then *referenced* (not re-derived) by the ECS (§9), parallelization (§8a), and interop (§11) sections.

4. **Sum types and `TypeId` maps replace signed-int conventions and RTTI.** `ResponseStatus`, `Command`, `TlmSync`, and `Phase` (§3.8–3.9) become enums whose invariants are structural; extensions become a `TypeId`-keyed map. `match` totality replaces "remember that 0 means incomplete."

5. **All deterministic-path time arithmetic is integer-only.** `Time(u64)` add/compare/advance are pure integer operations; `f64` is used *only* for one-shot conversions (resolution scaling, the rounding `Mul<f64>` convenience), never for any per-step or per-region accumulation that feeds a committed result. This is what lets Tier-1 parallel runs stay bit-exact to Tier-0 (§8a) — floating-point non-associativity can never enter the deterministic timeline.

6. **The unit of determinism is the delta cycle; the unit of parallelism is the quantum.** The single-threaded kernel owns the delta cycle; conservative PDES and `rayon` data-parallelism only ever act at or above quantum boundaries (§8a). The same quantum grid is the interop synchronization barrier (§11).

The remainder of the report is structured around these principles: the concurrency/scheduler core and the rest of the design (§6a–§6f), how far to push idiomatic Rust without breaching principle 1 (§7), parallelization in both senses (§8), the internal ECS-flavoured store (§9), the crate decomposition that lets the effort parallelize (§10), the phased interop strategy (§11), the roadmap and exit criteria (§12), the risk register (§13), the naming map (§14), and the source references (§15).

> **A note on section numbering.** This report has fourteen substantive sections plus this bridge. The major design content (concurrency, modules, channels, TLM-2, observability, twin needs) is *deliberately* carried as the six sub-sections §6a–§6f rather than split into separate top-level numbers, so §5 is a short synthesis section rather than a heavyweight one. We do not claim "fifteen equally-weighted sections"; we claim complete coverage of the requested topics in the requested order.

---

## 6. SystemRS design

### 6a. Concurrency & scheduler core (the centrepiece)

This is the heart of SystemRS: the single-threaded cooperative discrete-event kernel and the process model. Everything else in a TLM framework — `b_transport` annotating delay, `nb_transport` walking the phase FSM, the quantum keeper, PEQs — is a *client* of these primitives. Two sub-problems dominate: (1) how an `SC_THREAD` that calls `wait()` from arbitrary call depth maps to Rust, and (2) how processes mutate shared state without fighting the borrow checker.

#### The non-negotiable invariants

- **Exactly one process runs at any instant** (cooperative, not parallel).
- **Strict three-phase delta cycle:** EVALUATE (methods to completion, then threads one at a time) → UPDATE (commit channel `request_update`s) → DELTA-NOTIFY (fire delta events, queue next-delta processes). The evaluate/update split is the determinism guarantee.
- **Three notification kinds, priority immediate > delta > timed**, one pending per event, earliest wins.
- **Method-then-thread ordering** inside evaluate; the push/pop double-buffer defines delta-batch boundaries.

#### The three candidate mappings for `SC_THREAD`

SystemC's coroutine contract is tiny: `create(stack, fn, arg)`, `yield(next)`, `abort(next)`, `get_main()`. `yield(next)` means "block me, resume `next`."

**(a) async/await + custom single-threaded executor.** Processes become `Future`s; `wait()` becomes `.await`; an `sc_event` is a wakeable token; the kernel `crunch()` loop *is* the executor (never `tokio`). The fatal ergonomic flaw is **function colouring**: `wait()` must be callable from anywhere on the call stack — deep inside `b_transport`, inside helpers, inside library routines. With async, every function on that path becomes `async fn` and every call site `.await`s. The colour spreads virally across the entire TLM forward path. The ownership upside is real (you cannot hold a non-`'static` borrow across `.await`, which structurally prevents aliasing bugs), but the colouring cost is disqualifying for a SystemC-compatibility tool.

**(b) Stackful coroutines (corosensei).** The literal analogue of SystemC. `wait()` is an ordinary synchronous function — no colouring:

```rust
use corosensei::Coroutine;

// Yielder<Resume, Yield>: resume carries the wake reason in, yield carries the wait request out.
type Fiber = Coroutine<WaitResult, WaitRequest, ()>;

impl Ctx {
    /// Callable from ANY depth inside an SC_THREAD body — exactly like SystemC.
    pub fn wait_event(&self, ev: EventId) -> WaitResult {
        self.yielder().suspend(WaitRequest::Event(ev)) // hand control back to crunch()
    }
    pub fn wait_time(&self, t: Time) -> WaitResult {
        self.yielder().suspend(WaitRequest::Time(t))
    }
}
```

`b_transport` stays a plain `fn b_transport(&self, txn: &mut Gp, t: &mut Time)` and can call `ctx.wait_time(...)` from inside it. `tlm_quantumkeeper::sync()` → `wait(local_time)` works verbatim. This is the decisive ergonomic win and exactly why SystemC uses stackful coroutines.

**(c) OS threads + condvar (the `sc_cor_pthread` port).** Each process is a real `std::thread` lock-stepped by a per-process mutex+condvar. Faithful but performance-collapses at scale (MiB stacks, two syscalls per resume), determinism is fragile (you fight the OS scheduler with mutexes), and ownership gets worse (everything looks like it needs `Send + Sync + Arc<Mutex>`). Rejected except as a last-resort backend on exotic targets.

#### Recommendation

> **Primary: stackful coroutines via `corosensei` for `SC_THREAD`, plain `FnMut` for `SC_METHOD`, on a bespoke single-threaded discrete-event executor. Fallback: async/await behind the identical `Ctx` API, `cfg`-gated, for Wasm/no-fiber targets.**

The value proposition of SystemRS is trace-faithful, low-friction porting; colouring the entire TLM forward path with async imposes a tax on every model author for the life of the project, to buy a memory saving that small tunable stacks neutralize. `corosensei` installs guard pages (so SystemC's `stack_protect` becomes free), is maintained, and supports passing values both directions through resume/yield. The `Ctx`/`Suspend` API is identical under both backends, so the choice is an implementation detail behind one trait:

```rust
/// The single suspension surface. Backed by corosensei (primary) or futures (fallback).
pub trait Suspend {
    fn wait_event(&self, ev: EventId) -> WaitResult;
    fn wait_time(&self, t: Time) -> WaitResult;
    fn wait_or(&self, list: &EventOrList) -> WaitResult;
    fn wait_and(&self, list: &EventAndList) -> WaitResult;
    fn wait_event_timeout(&self, ev: EventId, t: Time) -> WaitResult; // wait(t, ev)
}
```

#### The ownership model: arena + `Ctx` handle

| Strategy | Verdict |
|---|---|
| `&mut` threading | Impossible — a running process must call back into the kernel that owns it; self-referential. |
| `Rc<RefCell<_>>` graph | Rejected — reference cycles between events and processes, runtime borrow panics when `trigger()` mutates an event list while a callback re-enters, pointer instability after death. |
| **Central arena + generational IDs** | **Chosen** — mirrors SystemC's "manager owns everything"; ids are `Copy`; "dead process" is a stale generation; dissolves the entire refcount-for-pointer-stability rationale. |
| Interior mutability for channel buffers | **Chosen locally** — the double-buffer is `Cell`/two cells, not `RefCell`, so no runtime borrow check on the hot path. |

```rust
pub struct Kernel {
    procs:  SlotMap<ProcId, Process>,
    events: SlotMap<EventId, Event>,
    chans:  SlotMap<ChanId, Box<dyn UpdatableChannel>>,

    // runnable double-buffers (mirror sc_runnable toggle_methods/toggle_threads)
    method_pop:  VecDeque<ProcId>,   // being drained this evaluate-batch
    method_push: VecDeque<ProcId>,   // scheduled DURING this batch -> next toggle, same phase
    thread_pop:  VecDeque<ProcId>,
    thread_push: VecDeque<ProcId>,

    update_queue: Vec<ChanId>,       // request_update enqueues; idempotent via per-chan flag
    delta_events: Vec<EventId>,      // drained high-index..0 each delta (faithful order)
    timed:        BinaryHeap<Reverse<TimedEntry>>, // (when, seq) min-heap

    now: Time,
    delta_count: u64,
    change_stamp: u64,
    delta_count_baseline_at_now: u64, // mirrors m_initial_delta_count_at_current_time; reset in do_timestep
    phase: Phase,                    // Initialize | Evaluate | Update | Notify
    seq:   u64,                      // monotonic tie-breaker for timed events
    running: Option<ProcId>,
}
```

A running fiber reaches the kernel through a thread-local `Ctx` set for the duration of one process's execution (exactly like `sc_get_curr_simcontext()`). The fiber **never** holds `&mut Kernel` across a suspension; it borrows re-entrantly only for the duration of each `ctx.method()` call, which returns before the next `suspend()`.

#### Time type

```rust
/// Count of time-resolution units. `INF = u64::MAX` is bit-for-bit `sc_time::max()`,
/// whose `max_time_tag` ctor is literally `~value_type{}` (sc_time.h:254-256) — an exact
/// match, not merely a semantic sentinel.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Time(u64);
impl Time {
    pub const ZERO: Time = Time(0);
    pub const INF:  Time = Time(u64::MAX);
}
// DETERMINISTIC PATH: integer only (principle 5, §5). add/compare/advance never touch f64.
impl core::ops::Add for Time { type Output = Time;
    fn add(self, o: Time) -> Time { Time(self.0.saturating_add(o.0)) } }
// NON-deterministic-path ONLY: a one-shot delay derivation (e.g. "0.5 * period") rounded
// to an integer BEFORE it is committed. Never used inside a per-step/per-region accumulation,
// so f64 non-associativity can never enter the committed timeline (see §8a Tier-1).
impl core::ops::Mul<f64> for Time { type Output = Time;            // integer scaling, +0.5 round
    fn mul(self, k: f64) -> Time { Time((self.0 as f64 * k + 0.5) as u64) } }
```

Resolution is a field of the elaborating `Kernel`, frozen by the `Building → Running` typestate, replacing SystemC's process-wide mutable singleton.

#### `sc_event`: notify / cancel / trigger

```rust
enum Pending { None, Delta { idx: usize }, Timed { seq: u64, when: Time } }

impl Kernel {
    pub fn notify_immediate(&mut self, ev: EventId) {
        assert!(self.phase == Phase::Evaluate, "SC_ID_IMMEDIATE_NOTIFICATION_");
        self.cancel(ev);                 // immediate cancels any pending delta/timed first
        self.trigger(ev);                // fires NOW, in the current evaluate phase
    }
    pub fn notify_delta(&mut self, ev: EventId) {
        match self.events[ev].pending {
            Pending::Delta { .. } => {}                                  // already delta: no-op
            Pending::Timed { .. } => { self.cancel_timed(ev); self.arm_delta(ev); } // delta wins
            Pending::None => self.arm_delta(ev),
        }
    }
    pub fn notify_timed(&mut self, ev: EventId, after: Time) {
        let when = self.now + after;
        match self.events[ev].pending {
            Pending::Delta { .. } => {}                       // delta always sooner: no-op
            Pending::Timed { when: w, .. } if w <= when => {} // keep the sooner one
            _ => { self.cancel(ev); self.arm_timed(ev, when); }
        }
    }
}
```

`trigger()` reproduces the observable subscriber ordering verified against `sc_event.cpp:378-458` — static methods, then dynamic methods, then static threads, then dynamic threads — and additionally reproduces the *iteration direction* within each list: the **static** lists are walked high-index→0, the **dynamic** lists are walked 0→high-index with consumed entries swapped in from the tail and then truncated. It consumes dynamic subscribers (swap-and-shrink) and applies the **immediate-self-notification guard** as the *per-process* predicate from `sc_thread_process::trigger_static` (`sc_thread_process.h:474-510`): a process is added to the runnable set only when it is runnable, not already queued, and still expecting this trigger, and is skipped entirely when it is the currently-running process — i.e. `if cur == p { report_self_notification(); return; }`, where `cur` is `self.running` (the analogue of `sc_get_current_process_b()`). This covers a thread that is dynamically waiting on the very event it immediately-notifies, not just statically-sensitive methods. `triggered()` is `event.trigger_stamp == self.change_stamp`. Signals and events share this one mechanism (the `change_stamp`-based `posedge()`/`event()`).

The timed heap uses an explicit insertion `seq` so equal-time ordering is **deterministic** (SystemC leaves it heap-defined; SystemRS pins FIFO-by-insertion and documents it). Delta cancel is O(1) `swap_remove` + index fix-up; timed cancel is lazy (tombstone, skipped on pop via a `seq` mismatch).

#### The crunch loop

```rust
impl Kernel {
    fn crunch(&mut self) {
        loop {
            // ---- EVALUATE: methods to completion, then threads one at a time ----
            self.phase = Phase::Evaluate;
            let mut ran = false;
            loop {
                self.toggle(Kind::Method);                 // swap push->pop when pop empty
                while let Some(p) = self.method_pop.pop_front() {
                    self.procs[p].queued = false;
                    self.run_method(p); ran = true;        // may push more (immediate notify)
                }
                self.toggle(Kind::Thread);
                if let Some(p) = self.thread_pop.pop_front() {
                    self.procs[p].queued = false;
                    self.resume_thread(p); ran = true;     // corosensei resume; returns at wait()
                    continue;                              // re-drain methods first
                }
                if self.runnable_empty() { break; }
            }
            // EMPTY-DELTA GUARD (sc_simcontext.cpp:566,614): an empty evaluate phase
            // advances NEITHER counter. SystemC computes `empty_eval_phase` and guards
            // both `m_change_stamp++` and `m_delta_count++` with it.
            let empty_eval = !ran;
            if empty_eval { break; }

            // ---- UPDATE ----
            self.phase = Phase::Update;
            self.change_stamp += 1;                        // bump BEFORE updates, non-empty only
                                                           // (sc_simcontext.cpp:564-567; signal::event())
            for c in std::mem::take(&mut self.update_queue) {
                self.chans[c].update(/* &mut Ctx via thread-local */);
            }

            // ---- DELTA NOTIFY ----
            self.phase = Phase::Notify;
            let evs = std::mem::take(&mut self.delta_events);
            for ev in evs.into_iter().rev() { self.trigger(ev); }  // high-index..0, faithful
            self.delta_count += 1;                         // non-empty only (guarded above)

            if self.runnable_empty() { break; }            // quiescent at this time
        }
    }

    /// Time advance, mirroring sc_simcontext::do_timestep (sc_simcontext.cpp:972-988).
    /// Called by `simulate()` between delta groups when the runnable set is empty and the
    /// next timed event lies strictly in the future. CRUCIAL: change_stamp is ALSO bumped
    /// here, on EVERY time advance — not only in the update phase — so that triggered()/
    /// event_occurred() observe a fresh stamp across a time step (sc_simcontext.cpp:986).
    /// Omitting this bump breaks triggered()/event_occurred() across a time advance.
    fn do_timestep(&mut self, t: Time) {
        debug_assert!(self.now < t, "time must advance monotonically (sc_simcontext.cpp:975)");
        self.fire_stage_callbacks(Stage::PreTimestep);     // SC_PRE_TIMESTEP, before commit
        self.now = t;
        self.change_stamp += 1;                            // bump on time advance (sc_simcontext.cpp:986)
        self.delta_count_baseline_at_now = self.delta_count;
    }
}
```

`resume_thread` resumes the fiber, receives the `WaitRequest` it yields, installs the corresponding `Sensitivity`, arms timeouts; on the next satisfying trigger the kernel sets a `wake_reason` and re-queues the fiber. Kill/reset are delivered as `WaitResult::Killed`/`Reset` that the body propagates with `?` (or a sentinel `panic!` caught at the fiber entry, so `Drop` runs during unwind matching SystemC RAII). The first cut implements kill/reset as **deferred** (mark dead, reap between deltas); full synchronous `preempt_with` semantics are a later, optional fidelity upgrade.

#### Sensitivity state machine

```rust
enum Sensitivity {
    Static,                                            // wake on any static_sens event / clock edge
    Event(EventId),
    Or(EventOrList),
    And { list: EventAndList, remaining: usize },      // fire only when remaining hits 0
    EventTimeout { ev: EventId, deadline: Time },
    OrTimeout  { list: EventOrList,  deadline: Time },
    AndTimeout { list: EventAndList, remaining: usize, deadline: Time },
}
```

`next_trigger` (method) and `wait` (thread) install a `Sensitivity` and, for timeouts, push a per-process entry into the timed heap keyed by `ProcId` (no hidden timeout-event object). `make_runnable` enforces "cannot queue twice" via a `queued` flag and routes methods/threads to the push buffer so processes made runnable *during* the current evaluate batch are picked up by the next toggle in the same phase.

#### Elaboration/run barrier as a typestate

```rust
pub struct Kernel<S> { /* fields */ _s: PhantomData<S> }
pub struct Building; pub struct Running;

impl Kernel<Building> {
    pub fn set_time_resolution(&mut self, fs_per_unit: u64) { /* only here */ }
    pub fn module<M: Module>(&mut self, name: &str, build: impl FnOnce(&mut Builder<M>)) -> ModId;
    pub fn build(self) -> Kernel<Running> { /* construction_done fixpoint, elaboration cbs */ }
}
impl Kernel<Running> {
    pub fn run(&mut self, until: Time, policy: Starvation);  // sc_start
    pub fn stop(&mut self, mode: StopMode);
    pub fn now(&self) -> Time { self.now }
}
```

This turns SystemC's runtime "simulation running" checks (`SC_ID_INSERT_MODULE_`, resolution-freeze) into compile errors.

#### Decision summary

| Concern | Decision |
|---|---|
| `SC_THREAD` | Stackful coroutine via `corosensei`; `wait()` sync at any depth, no colouring. |
| `SC_METHOD` | Plain `FnMut(&Ctx)`, run-to-completion, `next_trigger` installs next `Sensitivity`. |
| Fallback | async/await behind the identical `Ctx`/`Suspend` API, `cfg`-gated. |
| Threading | Strictly single-threaded; `Rc`/`Cell`, never `Arc<Mutex>` in the core. Foreign-thread `async_request_update` is the only `Send` boundary (mpsc drained at update-phase top). |
| Shared state | Central arenas keyed by `Copy` generational ids; processes/events/channels never hold references to each other. |
| Time | `Time(u64)` units; `u64::MAX` = ∞, *bit-for-bit* equal to `sc_time::max()` (`~value_type{}`); **integer-only** deterministic-path math (`f64` only for one-shot conversions); resolution frozen by `Building→Running`. |
| Event notify | Per-event `Pending {None,Delta,Timed}`; collapse rules ported verbatim; `triggered()` = `trigger_stamp == change_stamp`; `change_stamp` bumped in non-empty update phase *and* on every time advance (`do_timestep`), both counters skipping empty deltas. |
| Determinism | Bespoke executor only; faithful method-then-thread + verified subscriber ordering and iteration direction (`sc_event.cpp:378-458`). |
| Kill/reset | First cut deferred; full `preempt_with` documented as a later upgrade. |

### 6b. Modules, objects & elaboration in Rust

The `sc_module_name` LIFO-stack trick (a hidden global used so a ctor can discover its own name) is the most un-idiomatic-for-Rust part and is **not** reproduced. Construction is explicit via a scoped closure that pushes hierarchy scope, runs the body, and pops on return:

```rust
ctx.module("cpu", |m| {                       // pushes hierarchy scope, runs body, pops on return
    let isock = m.initiator_socket::<32, _>("isock");
    m.method("tick", tick_fn).sensitive_to(&clk);   // explicit process builder, no hidden state
})?;
```

The four lifecycle callbacks map to a trait with default-empty methods, driven in the fixed registry order with the construction-done fixpoint ("iterate until no new modules registered this pass"):

```rust
pub trait Elaborate {
    fn before_end_of_elaboration(&mut self) {}
    fn end_of_elaboration(&mut self) {}
    fn start_of_simulation(&mut self) {}
    fn end_of_simulation(&mut self) {}
}
```

Objects live in an arena (`SlotMap<ObjectId, ObjectMeta>`) with parent/child links as `Vec<ObjectId>` and a central name table (`HashMap<String, ObjectId>` tagged by origin). The orphan-children-to-root-on-drop rule becomes a re-parent of indices — sidestepping the C++ destruction-order hazards entirely. The `sensitive <<` "last-created-process" coupling is replaced by an explicit process builder (`.sensitive_to(&ev).dont_initialize()`). A `#[module]` proc-macro generates the registration boilerplate; `SC_HAS_PROCESS` is a deprecated no-op needing no analog. Attributes become an `AttributeStore` (`HashMap<TypeId, Box<dyn Any>>`) with lazy allocation.

### 6c. Events & channels

Events are covered by the kernel (§6a). Channels implement an `UpdatableChannel` trait the kernel knows about, plus an update-queue of `ChanId`s, reproducing `sc_prim_channel_registry` without the kernel knowing concrete channel types:

```rust
pub trait UpdatableChannel { fn update(&self, ctx: &Ctx); } // &self + interior mutability
```

`Signal<T, P: WriterPolicy>` double-buffers via split `Cell`s; `write()` stages into `new` and pushes the `ChanId` (idempotent via an `update_pending` flag); `update()` commits and posts the value-changed event for the *next* delta, stamping `change_stamp`. `Buffer<T>` fires an event on every write; `Signal` skips if unchanged — the observable distinction is preserved. `Fifo<T>` is backed by `VecDeque<T>` (dropping the hand-rolled circular buffer) with per-delta counters (`num_readable`/`num_read`/`num_written`) preserving the "written in delta N, readable in N+1" rule:

```rust
pub enum Capacity { Bounded(usize), Unbounded, Zero }

impl<T> Fifo<T> {
    pub fn try_put(&self, ctx: &Ctx, v: T) -> Result<(), FifoFull<T>> {
        if self.is_full() { return Err(FifoFull(v)); }   // returns the value, no ownership loss
        self.buf.borrow_mut().push_back(v);
        ctx.request_update(self.id());                    // event fires NEXT delta, faithful
        Ok(())
    }
    pub fn put(&self, ctx: &Ctx, mut v: T) {
        while let Err(FifoFull(returned)) = self.try_put(ctx, v) {
            v = returned;
            ctx.wait_event(self.data_read);               // while-loop, cooperative yield
        }
    }
}
```

`Clock` is a self-scheduling process (`Signal<bool>` + a process that toggles, `request_update`s, and schedules the next edge via a timed notification), not special-cased in the kernel. `Mutex`/`Semaphore` keep the immediate-notification semantics, backed by an event plus an owner id. An initialization update pass runs at simulation start so values written during elaboration are committed before the first evaluate.

### 6d. TLM-2 API design

The governing commitment: SystemRS processes are stackful coroutines, so the transport surface is **synchronous** (`wait()` is an ordinary call inside `b_transport`), not `async`. SystemRS keeps SystemC's transport *contracts* bit-for-bit while replacing its memory-management and type-identity *mechanisms*.

#### Generic payload — buffer ownership

The decisive decision: the data buffer is **owned by default** (`Vec<u8>`), not borrowed, because in the AT flow the payload outlives the call that created it and a borrowed `&mut [u8]` cannot survive a coroutine yield. SystemC's "payload never frees data" rule becomes "the payload *does* own and free its data" — strictly safer and indistinguishable from the model's point of view. A borrowing `PayloadView<'a>` is offered for the *pure-LT synchronous* path only.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command { Read, Write, Ignore }

/// SystemC (tlm_gp.h:96-103): OK = 1 (sole OK), INCOMPLETE = 0 (initial, NOT an error),
/// errors are strictly negative (-1..-5). SystemRS: a sum type; `is_error()` is total and
/// excludes both Ok and Incomplete, matching "discriminant < 0".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseStatus {
    Incomplete, Ok, GenericError, AddressError, CommandError, BurstError, ByteEnableError,
}
impl ResponseStatus {
    pub fn is_ok(self) -> bool { matches!(self, ResponseStatus::Ok) }
    pub fn is_error(self) -> bool { !matches!(self, ResponseStatus::Ok | ResponseStatus::Incomplete) }
}

/// SystemC: parallel 0xff/0x00 byte array, repeats modulo be_length. The modulo rule lives here.
#[derive(Debug, Clone)]
pub enum ByteEnable { All, Mask(Vec<u8>) }
impl ByteEnable {
    pub fn enabled(&self, i: usize) -> bool {
        match self { ByteEnable::All => true, ByteEnable::Mask(m) => m[i % m.len()] != 0x00 }
    }
}
```

#### Memory management — `Rc` + pool

Since the kernel is single-threaded by spec, `Rc` is correct and `Arc` would be a category error.

```rust
pub type Txn = Rc<RefCell<GenericPayload>>;   // pooled, shareable handle; replaces GP* + acquire/release

/// Holds a ref-count for the guard's lifetime; mirrors acquire()/release() without asserts.
pub struct TxnPool { free: RefCell<Vec<GenericPayload>> }
impl TxnPool { pub fn acquire(&self) -> Txn { /* pop or default, FULL reset, wrap in Rc */ } }
```

| SystemC rule | SystemRS equivalent |
|---|---|
| no MM: plain owned object, caller deletes | a bare `GenericPayload` moved by value down a synchronous call |
| MM set: refcounted, recycle at count 0 | `Txn` cloned into the spawned coroutine/PEQ entry; recycle via `TxnPool` |
| initiator holds a reference for the whole transaction | the initiator holds its `Rc` clone — premature-recycle bugs (C++ asserts) become impossible |

One faithfulness caveat is deliberately *fixed*: SystemC's `reset()` leaves scalar fields stale; `TxnPool::acquire` performs a full reset, because reusing stale fields is a real source of SystemC bugs.

#### Extensions — typed map, no RTTI

```rust
pub trait Extension: Any {
    fn clone_ext(&self) -> Option<Box<dyn Extension>>;   // None == "not clonable" (no null ptr)
    fn copy_from(&mut self, other: &dyn Extension);
    fn as_any(&self) -> &dyn Any;
}
#[derive(Default)]
pub struct ExtensionMap {
    auto: HashMap<TypeId, Box<dyn Extension>>,   // set_auto_extension: freed on recycle
    norm: HashMap<TypeId, Box<dyn Extension>>,   // set_extension: owned by the map, Drop frees
}
```

This collapses SystemC's three removal semantics onto Rust ownership: `set` (map owns, `Drop` frees — safer than C++ which leaks unless the caller frees), `set_auto` (cleared by `TxnPool::acquire`), `take` (returns ownership), `free_all` (drop both maps). For speed, the `TypeId → HashMap` lookup can later be swapped for a `Vec<Option<Box<dyn Extension>>>` indexed by an interned id.

#### Transport traits

```rust
pub trait Protocol: 'static { type Payload; type Phase: Copy + PartialEq; }
pub struct BaseProtocol;
impl Protocol for BaseProtocol { type Payload = GenericPayload; type Phase = Phase; }

/// nb return value. Folds the advanced phase into Updated so "phase only meaningful when advanced"
/// is structural, not convention.
pub enum TlmSync { Accepted, Updated(Phase), Completed }

pub trait FwTransport<P: Protocol> {
    /// `t` is delay relative to now. May yield the coroutine (cx.wait). The only blocking method.
    fn b_transport(&self, cx: &mut Cx, txn: &mut P::Payload, t: &mut SimTime);
    fn nb_transport_fw(&self, cx: &mut Cx, txn: &Txn, phase: Phase, t: &mut SimTime) -> TlmSync;
    fn get_direct_mem_ptr(&self, txn: &P::Payload, dmi: &mut Dmi) -> bool;
    /// No Cx, &mut Payload only: STRUCTURALLY forbids waits/notifications, callable off-scheduler.
    fn transport_dbg(&self, txn: &mut P::Payload) -> u32 { 0 }
}
pub trait BwTransport<P: Protocol> {
    fn nb_transport_bw(&self, cx: &mut Cx, txn: &Txn, phase: Phase, t: &mut SimTime) -> TlmSync;
    /// HARD RULE: must not call get_direct_mem_ptr (guarded by a RefCell<bool> reentrancy lock).
    fn invalidate_direct_mem_ptr(&self, cx: &mut Cx, start: u64, end: u64);
}
```

The `&mut`-aliasing problem is split by flow. **LT/`b_transport`** is synchronous, so a plain `&mut P::Payload` is sound and zero-copy (control is linear; no competing borrow). **AT/`nb_transport`** parks the txn between calls, so it passes `&Txn` (`= &Rc<RefCell<GenericPayload>>`); each side does a short-lived `borrow_mut()` within one phase boundary, and a double-borrow is a clean panic rather than UB. This is the deepest impedance mismatch, and SystemRS pays for it with `RefCell` checks **only on the AT path**.

#### Sockets — the bidirectional cycle, neutralized

A literal `Rc` cycle between initiator and target would leak. The resolution is a kernel-owned **socket arena with `SocketId` handles**: a cycle of indices is not a memory-management cycle.

```rust
pub struct InitiatorSocket<const BUSWIDTH: usize = 32, P: Protocol = BaseProtocol> { id: SocketId, _p: PhantomData<P> }
pub struct TargetSocket<const BUSWIDTH: usize = 32, P: Protocol = BaseProtocol>   { id: SocketId, _p: PhantomData<P> }

impl<const BW: usize, P: Protocol> InitiatorSocket<BW, P> {
    pub fn bind(&self, el: &mut Elaboration, target: &TargetSocket<BW, P>) {
        el.registry.connect(self.id, target.id);  // fw: self -> target
        el.registry.connect(target.id, self.id);  // bw: target -> self  (crossed double-bind)
    }
}
```

Bus width is a const generic, so mismatched binds are **compile errors** (better than SystemC's runtime convention). Multi-target-into-non-multi-target is rejected by making `MultiTargetSocket` a distinct type, so the illegal bind does not compile. Convenience sockets register **boxed closures** (`Box<dyn FnMut(...)>`), replacing the `void*` trampoline + pointer-to-member; "callback already registered" becomes `debug_assert!(field.is_none())`; missing callbacks use `Option` defaults (`transport_dbg` → 0, DMI → false).

#### Phases, LT↔AT adapters, PEQ

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase { Uninitialized, BeginReq, EndReq, BeginResp, EndResp, Extended(PhaseId) }
```

LT↔AT conversion ships as first-class library adapters: `AtToLtAdapter` (spawns a per-transaction coroutine on `BeginReq`) and `LtToAtAdapter` (pushes into a PEQ, blocks on a per-transaction event keyed by a `TxnId`, not a raw pointer). The PEQ uses the pull model (`peq_with_get`) as the recommended shape for both PEQs, with a `BTreeMap<(SimTime, seq), Entry>` and explicit insertion sequence for deterministic equal-time ordering. The delta-parity "fire on next delta" behaviour is preserved by routing through the kernel's `notify_delta` (which always lands in the next delta round), eliminating the manual even/odd arithmetic.

#### Temporal decoupling

```rust
pub struct QuantumKeeper { local_time: SimTime, next_sync_point: SimTime }
impl QuantumKeeper {
    pub fn need_sync(&self, cx: &Cx) -> bool { cx.now() + self.local_time >= self.next_sync_point } // >=
    pub fn sync(&mut self, cx: &mut Cx) { cx.wait(self.local_time); self.reset(cx); }  // the only yield
    pub fn reset(&mut self, cx: &Cx) {
        self.local_time = SimTime::ZERO;
        self.next_sync_point = cx.now() + cx.global_quantum().compute_local_quantum(cx); // q - now%q
    }
}
```

The global quantum lives inside the runtime context, not a true singleton — enabling multiple simulations per process and avoiding shared-mutable-global borrow pain.

#### Where Rust wins and where it costs

| Facet | Verdict |
|---|---|
| Response status / command / sync enum | **Rust strictly better** — sum types replace signed-int conventions; `match` is total. |
| Extensions | **Rust strictly better** — `HashMap<TypeId>` eliminates RTTI + `static_cast`. |
| Payload memory management | **Rust strictly better** — `Rc<RefCell<>>` + pool; premature-recycle impossible. |
| Bus-width / hierarchical / double bind | **Rust better** — const generics + typestate lift runtime errors to compile errors. |
| Convenience-socket callbacks | **Rust better** — closures replace `void*` trampolines. |
| `transport_dbg` purity | **Rust better** — no-`Cx` signature structurally forbids waits. |
| LT `b_transport` aliasing | **Even** — synchronous `&mut` is sound and zero-copy. |
| **AT shared-mutable transaction** | **Rust harder** — resolved with `Rc<RefCell<>>` (runtime borrow checks on the AT path only). |
| **Bidirectional initiator/target cycle** | **Rust harder, then neutralized** — kernel-owned socket arena with `SocketId`. |
| Stackful `wait()` from depth | **Rust harder (kernel cost)** — stackful backend not async, paid once in the kernel. |

### 6e. Observability & analysis for digital twins

The analysis port is the telemetry backbone. Two faces are offered. The **faithful synchronous face** is an observer list using `Weak` to encode "the port does not own subscribers", delivering `&T` in registration order within the same delta:

```rust
pub struct AnalysisPort<T> { subs: RefCell<Vec<Weak<dyn AnalysisWrite<T>>>> } // single-threaded: RefCell
impl<T> AnalysisPort<T> {
    pub fn write(&self, txn: &T) {                       // synchronous, in-order, no back-pressure
        self.subs.borrow_mut().retain(|w| w.strong_count() > 0);
        for w in self.subs.borrow().iter() { if let Some(s) = w.upgrade() { s.write(txn); } }
    }
}
```

The **ergonomic `Stream` face** wraps an unbounded `AnalysisFifo` (so `write()` never back-pressures) that a consumer drains as an iterator/stream — `recorder.next()` yields the consumer's coroutine until the next delta that fed the fifo. Tracing is driven by stage callbacks at `PreTimestep` (and `PostUpdate` for delta tracing) — *after* the update phase commits values — and traces through signal **handles** (reading a `Copy`/clone snapshot each sample), never a long-lived `&T` into a mutated signal. The primary sink is **transaction-centric** (`{ t_start, t_end, initiator, target, command, address, length, phases[], response }`), with optional VCD/FST backends. Telemetry I/O is pushed off the simulation hot path onto a writer thread via a `Send` boundary, so telemetry-on and telemetry-off traces are identical.

### 6f. What twins need beyond SystemC

SystemC is a batch simulator; a twin is a long-lived, observable, sometimes wall-clock-coupled, reconfigurable service. These are first-class subsystems on the deterministic core.

| Twin need | SystemC status | SystemRS plan | Priority |
|---|---|---|---|
| Real-time pacing | absent | `RealTimePacer` on the time-advance hook; only time-advance is paced, deltas stay instantaneous | High |
| External-input gating | partial (`async_*`) | `ExternalInput` mpsc inbox drained at update-phase top + suspend-on-starvation (never exit when idle) | **Critical** |
| Deterministic replay | implicit only | explicit tie-breaks (insertion `seq`) + seeded RNG service (no ambient `thread_rng`) + input journal | High |
| Snapshot/restore | absent | arena serialization at timestep boundaries when all threads are blocked at `wait` — *with the explicit coroutine caveat below*; DMI as arena handles | Medium |
| Live telemetry | in-process only | off-thread telemetry plane + `transport_dbg` query API | High |
| Hot-swap | forbidden | replaceable callback target (`Rc<dyn FwTransport>`) at a quiescent point; full structural hot-swap out of scope | Low |

```rust
pub trait ExternalInput: Send {
    /// Drained on the simulation thread at update-phase start. Returns true if it injected activity.
    fn poll(&mut self, cx: &mut SimContext) -> bool;
}
// kernel: if runnable empty && no timed events && a suspending input is attached -> park, don't stop.
```

The single most important twin-specific feature is **external-input gating without starvation exit**, architecturally seeded in the kernel's `next_time()`.

**The hard part of snapshot/restore: a *suspended* `SC_THREAD`.** Serializing the arenas, the event/timed/delta queues, and the scalar counters is mechanical (§9 benefit 3) — *all of that is plain id-keyed data*. The genuinely hard object is a `corosensei` coroutine that is **parked mid-`b_transport` at a `wait()`**: its live state is a native machine stack (saved SP, callee-saved registers, and a frame chain holding the model's local variables and the resume point), which is **not portable** and cannot be `serde`-serialized — neither across machines nor across binary rebuilds, and not even reliably across the same binary if stack layout shifts. SystemRS does **not** claim to capture a raw suspended stack. The supported model is narrower and stated honestly:

1. **Snapshot is taken only at a timestep boundary with every `SC_THREAD` blocked at a `wait()`** — never mid-evaluate, never with a fiber on the kernel stack. This is the only point at which a thread's *kernel-visible* state is fully externalised: its installed `Sensitivity` (which event(s)/timeout it is waiting on, §6a) and its `ProcessId` live in the arena, not on the native stack.
2. **What is serialized is that kernel-visible waiting state, not the native frame.** A restored process is *not* a resumed raw stack; it is a process re-entered at its `wait()` continuation. For the MVP this requires the thread body to be expressible as a **resumable state machine whose resume points are the `wait()` calls** — i.e. the model author (or a future codegen layer) provides a re-entry that, given the saved `Sensitivity` and any saved model-owned component state (which lives in arena columns, *not* in stack locals), reconstructs the logical continuation. Local variables that must survive a snapshot therefore have to live in serializable component state, not in raw stack locals across the `wait`.
3. **Anything that cannot meet (2) is out of MVP scope** and documented as such: a `corosensei` thread with non-trivial live locals held purely on its native stack across a `wait` is *not* snapshottable in M7's first cut. Full transparent stack capture (raw `corosensei` resume-point + register-file serialization) is a research-grade extension, not a promise.

This makes the §9 "first-class snapshot" benefit precise: it is first-class for *columnar component state and kernel queues*, and conditional (resumable-state-machine processes) for *suspended threads* — the report does not pretend a parked native coroutine stack serializes for free.

---

## 7. Making SystemRS idiomatically Rust

This section is opinionated about how far to deviate from SystemC's synchronous interface-method-call (IMC) idiom toward Rust's grain.

### The load-bearing constraint: one cooperative scheduler still owns time

Channels in SystemRS are an *ergonomic re-shaping of synchronous IMC over a single-threaded executor* — they are **not** `std::sync::mpsc`/`crossbeam`/`flume` carrying work between OS threads. Reaching for a real concurrent channel + multi-threaded runtime destroys the determinism that is the entire point. We therefore distinguish:

| Term | What it is | Backed by |
|---|---|---|
| **Sim-channel** | A deterministic delivery primitive (`AnalysisPort`, `Fifo<T>`, `PhaseQueue`) | The single-threaded kernel's event/update/delta queues |
| **OS-channel** | A real `crossbeam`/`flume`/`mpsc` channel | Actual OS threads, used **only** at the foreign-thread boundary |

OS-channels appear in exactly one architecturally-sanctioned place (mirroring `async_request_update`): the bridge from a foreign thread (real hardware twin, network socket, GUI) into the sim, drained at the top of each update phase.

### When channels fit vs when synchronous IMC is more faithful

Use **synchronous IMC** (a borrowed trait-method call, possibly yielding the coroutine) when the caller needs a synchronous answer on this stack frame: `b_transport`, `transport_dbg`, `get_direct_mem_ptr`, TLM-1 `transport`. Modeling `b_transport` as "send a message, await a reply" doubles queue traffic and obscures the call stack for zero benefit — keep it a trait method and let the suspension be the coroutine yield it already is.

Use a **sim-channel** (message-passing shape) when delivery is fan-out fire-and-forget (analysis ports), decoupled producer/consumer with back-pressure (`tlm_fifo`), or deferred to a future time/delta and pulled later (AT phase sequencing via PEQ). The dividing line: *does the caller need a synchronous answer on this stack frame?* Yes → IMC trait method. No → sim-channel.

An **actor-style module** (a module is a task with a mailbox, transactions are messages) is offered as a *documented pattern* built on the sim-channel primitives, not the substrate — because actors imply "reply later" while `b_transport` implies "reply now", and forcing actors reintroduces the b↔nb adapter complexity for cases that did not need it.

### Typed protocol enums and type-state binding

`Command`, `ResponseStatus`, `TlmSync`, and `Phase` are all loosely-typed integers in C++ and become enums; `TlmSync::Updated(Phase)` carries data so "phase is only meaningful when Updated" is structural. Newtypes for `SimTime`, `Addr`, and a const-generic `BUSWIDTH` make time/width mismatches type errors. The headline improvement is **type-state socket binding**: `b_transport` exists only on `InitiatorSocket<Bound, …>`, so calling it on an unbound socket is a *compile* error, not the runtime `m_bind_info == 0` check.

```rust
impl<const W: usize, P: Protocol> InitiatorSocket<Unbound, W, P> {
    pub fn bind(self, t: &mut TargetSocket<Unbound, W, P>)
        -> Result<InitiatorSocket<Bound, W, P>, ElaborationError> { /* crossed double-bind */ }
}
impl<const W: usize, P: Protocol> InitiatorSocket<Bound, W, P> {
    pub fn b_transport(&self, txn: &mut P::Payload, delay: &mut SimTime) { /* unbound use won't compile */ }
}
```

For multi-sockets (unbounded `N`), full type-state is impractical, so the `bind() → index; build() → resolve` two-phase pattern with `Result`/panic on pre-`build()` access is the honest fallback.

### Error handling: `Result` and typed errors, not `sc_report` throw

`sc_report` is both a record and a thrown exception in C++. Rust splits these by phase and **never uses `catch_unwind` as the primary control-flow mechanism**:

| SystemC behaviour | Phase | SystemRS mechanism |
|---|---|---|
| Binding error, unbound port, type mismatch | Elaboration | `Result<_, ElaborationError>` (many become compile errors via type-state) |
| Protocol error (bad response, illegal phase) | Runtime | `Result<_, ProtocolError>` — recoverable (testbenches inject errors) |
| TLM response status | Runtime data | a sum type *in the payload*, not an error |
| SC_FATAL / `sc_assert` / invariant corruption | Runtime, unrecoverable | `panic!` (aborts with a diagnostic) |
| INFO/WARNING (verbosity-gated) | Any | the `tracing`/`log` crate + a thin `ReportConfig` for severity→action precedence |
| `sc_unwind_exception` (kill/reset) | Runtime | cooperative cancellation: `wait()` resolves to `Err(ProcessControl::{Kill,Reset})`, propagated by `?` |

The rule: elaboration mistakes are programmer errors discovered before time advances — make them unrepresentable or `Result`; runtime protocol violations are recoverable and must be `Result`; only invariant corruption is `panic!`.

### RAII, ownership, iterators, builders

Drive structural scope via **closures, not `Drop`** (`ctx.module(.., |m| ..)`), so push/pop is structural and `Drop` never has to return errors or panic during unwind — replacing SystemC's `sc_hierarchy_scope` `noexcept(false)` contortions. Binding lifetimes use the **arena + generational index** pattern (no `Rc<RefCell<>>` graph of mutually-referencing kernel objects; ids instead). Transaction sequences (analysis fifo, PEQ, TLM-1 fifo) are `Iterator`/`Stream`. Socket callbacks and `sc_spawn` bodies are **closures** (`impl Fn`/`Box<dyn FnMut>`), not pointer-to-member trampolines.

### The strategic two-layer-API question

> **Recommendation: two layers — a faithful, id-based deterministic *core*, and an ergonomic idiomatic *facade* — shipping the facade as the blessed front door and treating the faithful surface as an interop/compat shim.**

| Option | Interop | SystemC-user learning curve | Idiomatic for Rust users | Verdict |
|---|---|---|---|---|
| One idiomatic API only | Hard (no recognizable shapes for FFI) | High | Excellent | Rejected — kills interop |
| One faithful (mirror) API only | Easy | Low | Poor (transliterated C++, fights the borrow checker) | Rejected — sells none of Rust's value |
| **Two layers** | Easy at the core seam | Low at core, moderate facade | Excellent at facade | **Chosen** |

**Deviate maximally on API surface and error model; zero on scheduling semantics.** Deviate hard (and declare it): kill the name-stack, the `sensitive <<` coupling, pointer-to-member trampolines, signed-int capacity hacks, `sc_report`-as-exception, freeze-on-first-use resolution, the circular buffer, intrusive linked-list queues, and the `sc_dt` library; add deterministic tie-breaks where C++ left order implementation-defined. Do **not** deviate on: the three-phase delta cycle and counters, notification collapse, immediate-notify-evaluate-phase-only and self-notification skip, the evaluate/update split, PEQ next-delta semantics, b↔nb adapter behaviour, GP memory ownership, the DMI invalidate re-entrancy ban, and the elaboration→run lifecycle.

---

## 8. Parallelization

Two orthogonal meanings: **(8a)** running the *simulation* on multiple cores, and **(8b)** running the *engineering effort* on multiple developers.

### 8a. Parallel execution of the simulation

#### The constraint we are fighting

The kernel's determinism is purchased with serialization: exactly one process runs at a time, and the evaluate/update split, immediate-notification handling, and global change-stamp are intrinsically sequential at the delta-cycle level. You cannot parallelize *within* a delta cycle without abandoning the observable orderings the corpus flags as "must reproduce for bit-exact compatibility." **Therefore the only sane unit of parallelism is the region/domain, synchronized at logical-time boundaries coarser than a delta cycle — and the TLM quantum is the natural boundary.**

#### PDES: conservative vs optimistic

| Axis | Conservative (Chandy–Misra–Bryant) | Optimistic (Time Warp) |
|---|---|---|
| Core idea | A region advances to `T` only once it can prove no message arrives with timestamp `< T` (lookahead + barriers/null messages). | Regions run speculatively; a straggler triggers state rollback + anti-messages. |
| State cost | Low — no checkpointing. | High — snapshots + event logs + GVT. |
| Determinism | Naturally deterministic given fixed lookahead and a total tie-break order. | Result deterministic but execution nondeterministic; rollback order must be canonicalized. |
| Fit for TLM twins | **Strong** — TLM's quantum *is* lookahead. | **Weak** — rolling back a payload with caller-owned buffers, DMI pointers, and side-effectful extensions is unsound by default. |

**Recommendation: conservative PDES, with the TLM quantum as the lookahead.** Temporal decoupling already gives every LT initiator a guaranteed lookahead equal to its local quantum, and `compute_local_quantum()`'s `q - (now % q)` aligns every initiator onto a common grid — exactly a CMB barrier we did not have to invent. **Optimistic execution is out of scope** for the default kernel because the generic-payload memory model makes rollback both expensive and semantically treacherous.

#### The quantum as the unit of parallelism

Within one global quantum, weakly-coupled initiators are causally independent iff every cross-region interaction has a latency ≥ the time remaining to the next sync point — a property LT models satisfy by construction. Each region runs its *own single-threaded kernel* up to the boundary, buffering **cross-region** messages into per-edge outboxes; at the barrier we join all region threads, deliver buffered messages in a canonical order, and advance the quantum. Intra-region traffic keeps full delta-cycle fidelity; only boundary channels are deferred.

```rust
struct RegionKernel { /* a sharded sc_simcontext: own queues, own local_time */ outbox: Vec<BoundaryMessage>, inbox: Vec<BoundaryMessage> }

fn run_quantum(regions: &mut [RegionKernel], quantum: SimTime, q_index: u64) {
    let boundary = global_quantum_boundary(q_index, quantum);

    // 1. PARALLEL: each region runs its delta/timed loop to the boundary. No shared mutable state.
    regions.par_iter_mut().for_each(|r| r.run_until(boundary));

    // 2. EXCHANGE (sequential, deterministic): canonical total order so delivery is reproducible
    //    regardless of which region thread finished first.
    let mut routed: Vec<BoundaryMessage> = regions.iter_mut().flat_map(|r| r.outbox.drain(..)).collect();
    routed.sort_unstable_by_key(|m| (m.deliver_at, m.dst_region, m.dst_channel, m.seq));
    for m in routed { regions[m.dst_region.0].inbox.push(m); }

    // 3. COMMIT (parallel): inject inbox into each region's timed/delta queues for the NEXT quantum.
    regions.par_iter_mut().for_each(|r| r.commit_inbox());
}
```

Correctness obligations: no message may be scheduled before the current barrier (enforced by `deliver_at >= boundary_at_send`); boundary payloads must be **owned snapshots** (`deep_copy_from`), not aliases; DMI cannot cross a region boundary (the partitioner disables cross-region DMI).

#### Model-graph partitioning

A **region** is a subgraph owning its own kernel instance. Partitioning is an elaboration-time decision (frozen before `start_of_simulation`). Cut only across channels with non-zero minimum latency (the lookahead); immediate-notification edges (signals same-delta, mutex/semaphore) and shared clocks must stay co-resident. Start with **modeler-declared regions** before attempting automatic min-cut partitioners — a twin author knows their domains better than a heuristic.

#### Data-parallel work (rayon)

Embarrassingly-parallel within a single logical instant, needing no clock reasoning: batched telemetry serialization (record on the ordered hot path, serialize off it), large memory models sharded by address range (`transport_dbg` is side-effect-free and trivially parallel), and stateless/partitioned target batches. All feed back through sorted, deterministic merge points, so they cost nothing in reproducibility.

#### Rust enablers and the determinism contract

| Mechanism | Use for | Avoid for |
|---|---|---|
| `Rc<RefCell<T>>`, arena indices | all intra-region state (the `!Send` core) | anything crossing a region boundary (won't compile — good) |
| `rayon` scoped `par_iter_mut` | per-region quantum execution; data-parallel telemetry/memory/dbg | inside a delta cycle |
| `Arc<...>` (immutable) | shared read-only config, ROM images | mutable shared state |
| `Arc<Mutex>` / atomics | telemetry fan-in, foreign-thread injection only | the simulation hot path |

`RegionKernel` is `!Send` internally; only `BoundaryMessage` and the region *handle* are `Send`. `rayon::par_iter_mut` over `&mut [RegionKernel]` gives disjoint `&mut` with no `unsafe`. A single audited `unsafe impl Send for RegionHandle {}` (justified by exclusive, move-only, never-aliased ownership) is the entire trust boundary of the parallel kernel — a single reviewable line rather than the diffuse raw-pointer aliasing of a C++ PDES.

**Deterministic replay is non-negotiable for twins.** Three tiers:

| Tier | Guarantee | How |
|---|---|---|
| **Tier 0 — bit-exact serial** | identical event trace, run-to-run and machine-to-machine | single-thread core; canonical trigger ordering; total order `(SimTime, Seq, EntityId)` on all queues. The golden reference. |
| **Tier 1 — deterministic parallel** | same committed result and cross-region message trace as Tier 0, independent of thread count/timing | barrier-synchronous quantum; fixed partition map; exchange sorts by `(deliver_at, dst_region, dst_channel, src_seq)`; no tie-break depends on address/HashMap order/completion order |
| **Tier 2 — throughput mode** | correct results, no trace reproducibility | reserved for any future optimistic/lock-based fast path; off by default |

A Tier-1 run is bit-exact to a Tier-0 run *only with the same quantum and partition* — i.e. Tier 1 parallelizes an already-quantized timing model rather than introducing new nondeterminism. We record `(quantum, partition_map, seed)` in the run header and ship a `--verify-determinism` mode (run Tier 1, re-run Tier 0 with the same quantum/partition, assert trace equality) as the CI gate.

**Integer-only time arithmetic is a precondition of Tier-1 = Tier-0 (principle 5, §5).** Floating-point addition is *not associative*, so if any committed quantity on the deterministic timeline were accumulated in `f64` — for example a per-region `local_time` summed as `f64` and then compared across regions, or `Time::Mul<f64>` applied repeatedly inside the hot loop — a different thread-interleaving or partition could reorder the additions and produce a different last bit, silently breaking the Tier-1 bit-exactness this section promises. SystemRS therefore mandates: **`Time` is `u64` units and every operation that advances or compares simulation time, accumulates a region's local time, computes a quantum boundary (`q - now % q`), or feeds the exchange sort key is integer arithmetic only.** `f64` appears in exactly two non-deterministic-path places — the one-shot resolution/time-unit conversion at construction, and the rounding `Time::Mul<f64>` convenience used to *derive a delay once* before it is committed as an integer — and never inside a per-step or per-region accumulation. `--verify-determinism` would catch a violation, but the invariant is asserted in the type design so the violation cannot be written: there is no `+=` on an `f64` time anywhere in the kernel or the region orchestrator.

#### Pragmatic recommendation

Build the deterministic single-thread core first (Tier 0) as the golden reference; add opt-in parallel regions gated on the quantum (Tier 1) as a *wrapper* around N copies of the Tier-0 core (parallelism lives entirely in orchestration; the intra-region kernel is unchanged); layer rayon data-parallelism independently; do not build optimistic Time-Warp; ship `--verify-determinism` from day one of Tier 1.

### 8b. Parallelizing the implementation effort

The goal is to let multiple developers build SystemRS concurrently by carving at **trait seams**. The crate decomposition (§10) is the mechanism.

#### The critical path

The critical path is exactly two things: `systemrs-kernel` (scheduler + process + event/notification + arena IDs) and `systemrs-tlm2` (GP + transport traits + protocol types). Everything else is downstream of a *trait* these publish. The strategic move is to **split each into a tiny `*-api` crate (traits + types) that stabilizes fast, and an `*-impl` crate** — then freeze the `*-api` surfaces early (gated behind a short RFC), so every other team can compile and test against the contract while the implementations are still being built.

#### Interface-first contracts and stubbing

| Seam (trait) | Unblocks | Mocked by |
|---|---|---|
| `Scheduler` / `wait`/`notify` | channels, tlm-utils, tlm1, examples | a `MockScheduler` running methods inline, recording notifies |
| `CoroutineRuntime` | thread-using channels, AT sockets | a `BlockingMockRuntime` (panics on yield) for method-only tests |
| `UpdatableChannel` | parallel orchestrator, tracing | a `NullChannel` no-op `update()` |
| `FwTransport`/`BwTransport` | sockets, PEQs, examples, conformance | a `LoopbackTarget` returning `Completed` |
| `Extension` | endianness, instance-ext, user code | `NoopExtension` |
| `RegionKernel::run_until` | `systemrs-parallel` barrier work | a `OneStepRegion` advancing by the quantum with a scripted outbox |

Worked example: the LT↔AT adapter team (deep in the graph) starts on day 1 against a `MockTarget`, validating the phase FSM and exclusion rules with no real kernel and no coroutine backend; when the runtime lands, only the `wait`/spawn wiring changes, not the FSM logic. The hardest concurrency code — the deterministic barrier merge — is developed and fuzz-tested against a `OneStepRegion` stub independently of the real kernel.

**The one rule that makes 8b work:** stabilize `systemrs-kernel-api` and `systemrs-tlm2-api` before writing any implementation, and gate changes behind RFCs. Get them wrong or leave them fluid, and the seams collapse into a single critical path — exactly the serialization, at the org level, that 8a fights at the kernel level.

---

## 9. Entity-Component-System (ECS) data architecture

### The question

We are already committed to an arena/generational-index store (every subsystem analysis recommends it). ECS is the industrial generalization of exactly that pattern: a generational-index entity store + columnar component storage + a scheduler running functions over queries. So the question is **how far up the ECS ladder to climb**, and **whether to expose it to model authors**.

### Conceptual mapping

| SystemRS concept | ECS concept | Notes |
|---|---|---|
| `sc_module`/`sc_object` instance | **Entity** | stable `ObjectId`; replaces raw back-pointers |
| Module's user state (registers, RAM) | **Component(s)** | plain data, columnar per type |
| Sockets/ports | **Components** on the owning module, not their own entities | sockets share their module's lifecycle |
| Port binding target | a `Binding` component holding peer `ObjectId`s | not `Arc<dyn IF>` |
| Static sensitivity | a `Sensitivity` component | resolved at `complete_binding` |
| `sc_event` | **Entity** or arena entry keyed `EventId` | first-class enough to be an entity; a flat `SlotMap` is equally fine |
| `SC_METHOD` | **System-like run-to-completion callback** | the only clean fit — stackless, run-to-completion, re-armed via `next_trigger` |
| `SC_THREAD`/`SC_CTHREAD` | **NOT an ECS system** | stackful coroutine yielding mid-function; see the mismatch |
| Primitive channels | **Entity + components** (`SignalValue<T>`, `UpdatePending`) | evaluate/update = a dirty component + an update system |
| TLM generic payload | **Transient data, NOT an entity** | pooled, ref-counted, short-lived; flows *through* systems |

Sockets/ports are **components, not entities** — they share their module's lifecycle and `complete_binding` resolution; treating each as its own entity would shred locality.

### Benefits for a TLM twin

1. **Cache locality for large homogeneous fleets.** A twin is dominated by many instances of few types (thousands of memory blocks, identical IP cores, NoC routers). Columnar storage lets "step every memory model this delta" stream over a contiguous array rather than pointer-chase a `Box<dyn Module>` graph. For LT under temporal decoupling — where the hot loop is `b_transport` dispatch + quantum bookkeeping — this is a measurable win.
2. **Automatic parallelism — the tie-in to §8 (a *bounded*, future-work claim).** In principle the scheduler can run systems with disjoint component access concurrently, because SystemC's determinism guarantee is the **evaluate/update separation**, not literal single-threading: within one evaluate phase, two `SC_METHOD`s that (a) touch disjoint components, (b) write *only* via the staged `request_update`/`perform_update` queue, and (c) perform *no* immediate notification and *no* direct mutation of module state or peer components cannot observe each other regardless of order. That is the property a disjointness analysis would prove. **But the general `SC_METHOD` does not satisfy (b)–(c).** A method may legally call `notify()` (immediate notification, which `trigger()`s a peer *synchronously, in-phase* — an observable cross-method side effect), and a method may mutate its own module's plain fields directly rather than through a staged channel. Either makes two methods order-dependent and unsafe to run concurrently. So the honest, *bounded* claim is: **intra-delta parallelism is sound only for the subset of `SC_METHOD`s statically proven to (i) write all observable state through the staged update queue, (ii) issue no immediate notifications, and (iii) not mutate state shared with another concurrently-scheduled method.** The static analyzer that certifies a method into that subset (and conservatively excludes everything it cannot prove) **does not exist yet and is explicitly future work**; until it ships, methods run serially in the canonical order like Tier-0. ECS gives a *principled route* to this parallelism — it does not deliver it for free, and the default remains serial-and-order-fixed (commit + notification drain always serial).
3. **First-class snapshot/restore/replay/introspection — the biggest twin win.** If all state is components in columns, a world snapshot is "serialize every column + the entity-generation table + the scheduler queues + the scalar counters" — mechanical with id-keyed columns, near-impossible with a raw-pointer graph. This directly enables time-travel debugging, checkpoint/resume, deterministic replay, and live "observe component X across all entities each `PreTimestep`" telemetry.
4. **Structural hot-swap.** Replacing a model = remove components, attach new ones, keep the same `ObjectId` so bindings stay valid — natural in ECS, awkward in OO.

### Trade-offs and mismatches

1. **(The big one) `SC_THREAD` coroutines are not run-to-completion systems.** An ECS system returns; it cannot suspend mid-function. But `b_transport` with internal `wait(t)`, `sc_fifo::read`, `tlm_quantumkeeper::sync()`, and the nb adapter threads all yield from arbitrary call depth and resume with their stack intact. Expressing this as a sequence of run-to-completion systems requires manual state-machine rewrites of every model — destroying SystemC compatibility. **Reconciliation:** an `SC_THREAD` is an **entity whose continuation (the coroutine handle) is a component**, resumed *serially* by the coroutine driver; it is never parallelized by disjointness (it can touch anything across a yield). Only `SC_METHOD`s, channel `update()`s, and read-only observers are scheduled like ECS systems with disjointness-based parallelism.
2. **Hierarchical naming & dynamic sensitivity vs flat queries.** ECS queries are flat; SystemC needs hierarchical `.`-joined names and a per-process `Wait` state machine. Keep a *separate* name table and parent/child link components, and drive sensitivity by the event subsystem's id-based subscriber lists — not by queries. ECS columns are for bulk state and bulk stepping; the graph/naming/sensitivity layer rides on top by id.
3. **Determinism of system execution order.** A general ECS scheduler picks *some* valid order and often does not guarantee stability across runs/versions. SystemC requires a specific reproducible order in the serial parts. Any parallelism must be opt-in, conflict-checked, and order-deterministic at commit — which most off-the-shelf schedulers do not promise.
4. **Author cognitive burden & interop.** SystemC users think in OO modules + `SC_METHOD; sensitive << ev`. Forcing components + systems + queries is an alien paradigm that hurts adoption.
5. **Crate churn/semver.** `bevy_ecs` has rapid breaking releases; `legion` is effectively unmaintained. Exposing a third-party ECS type in SystemRS's *public* API couples our semver and our users' code to that crate's churn — unacceptable for a long-lived SystemC analogue.

### Recommendation

> **Build a bespoke generational-index / lightly-archetyped store INTERNALLY, and do NOT force ECS on model authors. Do not adopt a third-party ECS crate in the public API; do not stop at a bare slotmap either.**

- **Concurrency model:** an *internal* columnar store + a conflict-checked *deterministic* scheduler is the natural extension that unlocks §8's parallelism while preserving the bespoke-single-threaded-determinism contract. A third-party scheduler would forfeit determinism.
- **Interop/authoring:** authors keep an OO surface (`#[module]`, `SC_METHOD`-like registration, `wait()`, sockets); the ECS-ness is a kernel implementation detail, invisible at the model boundary, protecting compatibility and a flat learning curve.
- **Twin needs:** the internal id-keyed, column-friendly representation is what makes snapshot/restore/replay/introspection/hot-swap mechanical without exposing a churning `World`.

Treating `SC_METHOD`s/channel `update()`s/observers as ECS-style systems (safe because the evaluate/update split makes within-delta order irrelevant) and `SC_THREAD`s as entities-with-continuations resumed serially is the synthesis. Start at a `SlotMap` core and design the type boundaries so the columns/scheduler slot in *behind* the kernel API without touching model-author code.

```rust
pub enum ProcessBody {
    /// SC_METHOD: stackless. The ONLY kind the system-scheduler may run in parallel
    /// (subject to component disjointness), because all writes are STAGED.
    Method(Box<dyn FnMut(&mut MethodCtx)>),
    /// SC_THREAD: stackful. The continuation is stored as a component; the kernel RESUMES
    /// it serially. Never parallelized by disjointness — it can touch anything across a yield.
    Thread(Coroutine),
}
```

---

## 10. Proposed crate structure

A 14-crate Cargo workspace under `crates/`, layered acyclically. The RTL `sc_dt` datatypes library is deliberately *not* a crate.

### 10.1 Crate-by-crate

| Crate | Responsibility | Depends on | Key external deps |
|---|---|---|---|
| `systemrs-diag` | Reporting: severity, actions, verbosity, message-type registry, `Report`/`ReportError` | — | `bitflags`, `log`/`tracing` |
| `systemrs-time` | `SimTime` newtype, resolution, `ZERO`/`INF` | `-diag` | — |
| `systemrs-runtime` | `CoroutineRuntime` trait + stackful backend | — | `corosensei` |
| `systemrs-kernel` | Scheduler, delta/timed queues, events, notification, processes, arenas, stage callbacks | `-diag`, `-time`, `-runtime` | `slotmap`, `bitflags`, `smallvec` |
| `systemrs-core` | `Module`/`Object`, elaboration, sensitivity builder, `wait`/`next_trigger`, attributes, `ModuleVec` | `-kernel`, `-diag`, `-time` | `slotmap` |
| `systemrs-channels` | Interfaces/ports/exports/binding; `Signal`/`Buffer`/`Fifo`/`Clock`/`Mutex`/`Semaphore` | `-kernel`, `-core`, `-diag`, `-time` | — |
| `systemrs-tlm1` | put/get/peek interfaces, `TlmFifo`, analysis ports/fifo/triple | `-channels`, `-core`, `-kernel`, `-time` | — |
| `systemrs-tlm2` | GP + MM + extensions, transport traits, phases, DMI, sockets | `-channels`, `-core`, `-kernel`, `-time`, `-diag` | `bitflags`, `inventory` |
| `systemrs-tlm-utils` | quantum keeper, global quantum, PEQs, convenience/multi sockets, LT↔AT adapters | `-tlm2`, `-kernel`, `-channels`, `-core`, `-time` | — |
| `systemrs-trace` | stage-callback sampling, `TraceSink`/`Traceable`, transaction recorder, VCD/FST | `-kernel`, `-channels`, `-time`, `-tlm2`, `-diag` | `serde`, `bincode`, opt. `vcd`/`fst` |
| `systemrs-macros` | `#[module]`, `#[derive(Module)]`, `#[derive(Extension)]` | none (path-qualified codegen) | `proc-macro2`, `quote`, `syn` |
| `systemrs-ffi` | C ABI / `cxx` interop & SystemC co-sim | `systemrs` facade, `-tlm2`, `-kernel` | `cxx`, `cc`, `libc` |
| `systemrs` | Facade/prelude re-exporting the public API | all except `-ffi`/`-examples` | — |
| `systemrs-examples` | Examples + cross-crate integration/conformance tests | `systemrs` (+ `-ffi` under cosim) | dev: `insta`, `criterion` |

### 10.2 Dependency graph

```
            systemrs-diag      systemrs-time      systemrs-runtime      systemrs-macros
                  ^               ^   ^                  ^                 (proc-macro,
                  |               |   |                  |                  no ws deps)
                  +-------+-------+   |                  |
                          |          |                  |
                     systemrs-kernel-+------------------+
                          ^   ^   ^
                          |   |   |
                systemrs-core  |   +-------------------+
                     ^   ^      |                       |
                     |   +------+--- systemrs-channels  |
                     |              ^      ^      ^       |
                     |              |      |      |       |
              systemrs-tlm1 --------+      |  systemrs-tlm2
                     ^                     |      ^   ^
                     |                     |      |   |
                     |                     |  systemrs-tlm-utils
                     |                     |      ^
                     +---- systemrs-trace -+------+   (trace -> kernel, channels, tlm2, time, diag)
                                  |
   ===================== facade ==========================
   systemrs  ->  kernel, core, channels, tlm1, tlm2, tlm-utils, trace, diag, time, macros
                                  |
   systemrs-ffi  ->  systemrs (facade), tlm2, kernel      [+ external/systemc via cxx, feature]
   systemrs-examples  ->  systemrs (facade)               [+ systemrs-ffi under `cosim`]
```

Layering (lowest → highest): `L0` diag/time/runtime/macros · `L1` kernel · `L2` core · `L3` channels · `L4` tlm1, tlm2 · `L5` tlm-utils, trace · `L6` facade · `L7` ffi, examples.

### 10.3 Directory tree

```
systemrs/
├── Cargo.toml                      # workspace root (virtual manifest)
├── rust-toolchain.toml             # channel pin
├── deny.toml                       # cargo-deny gate
├── doc/
├── external/systemc/               # vendored reference (gitignored child)
└── crates/
    ├── systemrs-diag/      src/{lib,severity,actions,report,handler,registry}.rs
    ├── systemrs-time/      src/{lib,sim_time,resolution}.rs
    ├── systemrs-runtime/   src/{lib,coroutine,stackful}.rs        # async.rs behind feature
    ├── systemrs-kernel/    src/{lib,simcontext,scheduler,runnable,process,event,
    │                            notify,timed,phase,status,stage,lifecycle,ids}.rs
    ├── systemrs-core/      src/{lib,object,module,name,sensitivity,wait,
    │                            elaboration,attribute,vector,hierarchy}.rs
    ├── systemrs-channels/  src/{lib,interface,port,export,finder,prim_channel,
    │                            signal,buffer,fifo,clock,mutex,semaphore,resolved}.rs
    ├── systemrs-tlm1/      src/{lib,ifs,tlm_fifo,analysis,req_rsp}.rs
    ├── systemrs-tlm2/      src/{lib,gp,mm,extension,transport,phase,dmi,sync,protocol,
    │                            socket/{mod,initiator,target,base}.rs}
    ├── systemrs-tlm-utils/ src/{lib,quantum,global_quantum,peq_cb,peq_get,
    │                            simple_socket,passthrough,multi,adapter_lt_at}.rs
    ├── systemrs-trace/     src/{lib,sink,traceable,txn_recorder,vcd,fst}.rs
    ├── systemrs-macros/    src/{lib,module,process,extension}.rs
    ├── systemrs-ffi/       build.rs include/systemrs.h src/{lib,c_api,cosim}.rs
    ├── systemrs/           src/{lib,prelude}.rs
    └── systemrs-examples/  examples/{lt_memory,at_bus,signal_clock,scoreboard}.rs
                            tests/{delta_order,notify_collapse,base_protocol}.rs
```

### 10.4 Workspace `Cargo.toml` skeleton

```toml
[workspace]
resolver = "3"
members = [
    "crates/systemrs-diag", "crates/systemrs-time", "crates/systemrs-runtime",
    "crates/systemrs-kernel", "crates/systemrs-core", "crates/systemrs-channels",
    "crates/systemrs-tlm1", "crates/systemrs-tlm2", "crates/systemrs-tlm-utils",
    "crates/systemrs-trace", "crates/systemrs-macros", "crates/systemrs-ffi",
    "crates/systemrs", "crates/systemrs-examples",
]

[workspace.package]
version      = "0.1.0"
edition      = "2024"
rust-version = "1.90"               # MSRV; CI also tests current stable
license      = "Apache-2.0"          # match SystemC's Apache-2.0 lineage
repository   = "https://github.com/londey/systemrs"
authors      = ["Nicholas Londey <londey@gmail.com>"]

[workspace.dependencies]
systemrs-diag      = { path = "crates/systemrs-diag",      version = "0.1.0" }
systemrs-time      = { path = "crates/systemrs-time",      version = "0.1.0" }
systemrs-runtime   = { path = "crates/systemrs-runtime",   version = "0.1.0" }
systemrs-kernel    = { path = "crates/systemrs-kernel",    version = "0.1.0" }
systemrs-core      = { path = "crates/systemrs-core",      version = "0.1.0" }
systemrs-channels  = { path = "crates/systemrs-channels",  version = "0.1.0" }
systemrs-tlm1      = { path = "crates/systemrs-tlm1",      version = "0.1.0" }
systemrs-tlm2      = { path = "crates/systemrs-tlm2",      version = "0.1.0" }
systemrs-tlm-utils = { path = "crates/systemrs-tlm-utils", version = "0.1.0" }
systemrs-trace     = { path = "crates/systemrs-trace",     version = "0.1.0" }
systemrs-macros    = { path = "crates/systemrs-macros",    version = "0.1.0" }

slotmap = "1"; bitflags = "2"; smallvec = "1"; corosensei = "0.2"
log = "0.4"; tracing = "0.1"; inventory = "0.3"
serde = { version = "1", features = ["derive"] }; bincode = "2"
proc-macro2 = "1"; quote = "1"; syn = { version = "2", features = ["full"] }
cxx = "1"; cc = "1"; libc = "0.2"

[workspace.lints.rust]
unsafe_code = "warn"                 # flipped to allow locally only in ffi/runtime, with SAFETY notes
missing_docs = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }

[profile.release]
lto = "thin"
codegen-units = 1
```

The proc-macro crate carries `[lib] proc-macro = true` and depends only on `proc-macro2`/`quote`/`syn`, generating path-qualified code (`::systemrs::…`) so the facade re-exports the macros without a cyclic dependency.

### Naming / edition / MSRV

| Topic | Recommendation |
|---|---|
| Crate prefix | `systemrs-` internally; the user-facing umbrella is just `systemrs`. |
| Module naming | drop the `sc_` prefix (`Signal`, not `sc_signal`); keep a mapping table (Appendix). |
| Edition | 2024 (resolver 3). |
| MSRV | 1.90, verified in CI alongside current stable. |
| `Send`/`Sync` | core crates intentionally `!Send`; the foreign-thread path is the only `Send` boundary, behind a feature. |
| Lints | workspace `clippy::pedantic`, `unsafe_code = "warn"` (allowed only in ffi/runtime with `// SAFETY:`); `cargo-deny` gates licenses/advisories/dup-versions. |

---

## 11. Interoperability with SystemC

Co-simulation is the make-or-break adoption lever. A Rust TLM framework that cannot plug into the installed base of SystemC virtual platforms is a research toy.

### 11.1 The governing constraint: there can be only one scheduler

Every kernel finding converges on one fact: SystemC is cooperatively single-threaded by construction, and each kernel owns its own `m_curr_time`, delta counters, and notification queues. Two live schedulers cannot share one logical timeline without an explicit synchronization protocol. This dictates the design space:

| Approach | Owns time/delta | Determinism | Isolation | Effort |
|---|---|---|---|---|
| 1a. Rust models guest-in-SystemC | C++ `sc_simcontext` | native (one kernel) | none (shared address space) | Low–Med |
| 1b. C++ models guest-in-Rust | Rust kernel | native (one kernel) | none | High |
| 1c. Two kernels, in-process time-synced | negotiated quantum | approximate | none | Very high |
| 3. Two kernels, out-of-process | negotiated quantum | approximate | strong | High |

**Recommendation: 1a first, 1b second, 3 last. Reject 1c** — two live kernels in one process give the determinism loss of out-of-process with none of the crash isolation.

### 11.2 Phase 1: Rust models as guests inside the SystemC kernel (recommended first deliverable)

The C++ `sc_core` scheduler is authoritative. Rust TLM components are constructed during C++ elaboration, registered as `sc_module`s, and their process bodies are driven by C++ dispatch via callbacks across the FFI. The Rust side never runs its own scheduler; `wait()`, `notify()`, `b_transport`, and `nb_transport_*` delegate to the C++ kernel. This inherits IEEE-1666 determinism for free.

**The coroutine-stack risk — unwinding in *both* directions.** A Rust `SC_THREAD` body running on the C++ QuickThreads stack is safe across a `wait()` stack switch (a stack switch is just an SP save/restore; Rust locals are untouched) **only if no unwind of either kind ever crosses the QuickThreads boundary**, and there are two distinct hazards, not one:

- **Rust panic escaping into C++.** A panic with `panic = "unwind"` that is not caught at the Rust `extern "C"` entry is undefined behaviour once it reaches the C++ frame — worse here because the C++ frame may itself be a *suspended coroutine frame* of a different process, so the unwinder walks a stack that is not the logical caller. The `catch_unwind` firewall below addresses this direction; for belt-and-braces the FFI build can also use `panic = "abort"`.
- **C++ exception escaping into Rust (the under-covered direction).** SystemC routinely *throws*: `SC_REPORT_ERROR`/`SC_REPORT_FATAL` from inside `sc_wait`, `sc_report` as a thrown object, and `sc_unwind_exception` used to deliver `kill`/`reset`. When a Rust `SC_THREAD` calls `cx.wait(t)` → `sc_wait`, the C++ side can throw *while logically inside the Rust frame*. A C++ exception unwinding *through* Rust frames is undefined behaviour under `panic = "unwind"` (Rust frames are not C++ EH frames; destructors/`Drop` interleaving is unspecified), and `catch_unwind` at the Rust *entry* does **not** help — the throw originates *below* that entry, on the far side of the `sc_wait` call. The correct firewall is therefore *symmetric*: the **C++ shim must `try { … } catch (...)`** around every call into `sc_wait`/`b_transport`/`nb_transport_*` that can re-enter Rust, convert the exception to a status code or a re-entrant "process should terminate" signal *before* control returns across the boundary, and let the Rust side observe that as a normal `Err(ProcessControl::{Kill,Reset})` from `wait()` rather than as a live unwind. In short: Rust panics are caught at the Rust entry; C++ exceptions are caught at the C++ shim *before* they reach a Rust frame; neither unwind is ever allowed to traverse a foreign frame or the coroutine stack switch.

Every `extern "C"` entry wraps the body in `catch_unwind` and converts a panic into `SC_REPORT_FATAL` (the Rust→C++ half of the symmetric firewall):

```rust
#[no_mangle]
pub extern "C" fn rust_process_entry(ctx: *mut KernelCtx) {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let cx = unsafe { Cx::from_raw(ctx) };  // borrowed handle, valid only while running
        my_thread_body(&cx);                     // may call cx.wait(t) -> ffi sc_wait
    }));
    if r.is_err() { unsafe { ffi::sc_report_fatal(c"SYSTEMRS/PANIC", c"Rust process panicked"); } }
}
```

**Tooling: `cxx` over a de-templated C++ shim.** `autocxx` chokes on SystemC's heavy templates (`tlm_initiator_socket<BUSWIDTH, TYPES, N, POL>`, virtual-diamond interfaces); `bindgen` gives only a raw C ABI with no C++ exceptions/RAII. The shim monomorphizes the base-protocol types and exposes non-template C functions `cxx` can bind:

```cpp
// systemrs_shim.cpp — de-templated, cxx-friendly surface over TLM-2.0 base protocol.
extern "C" {
  tlm::tlm_sync_enum srs_nb_transport_fw(void* rust_target, tlm::tlm_generic_payload* gp,
                                         tlm::tlm_phase* phase, sc_core::sc_time* t);
  void     srs_b_transport(void* rust_target, tlm::tlm_generic_payload* gp, sc_core::sc_time* t);
  unsigned srs_transport_dbg(void* rust_target, tlm::tlm_generic_payload* gp);
  bool     srs_get_dmi(void* rust_target, tlm::tlm_generic_payload* gp, tlm::tlm_dmi* dmi);
}
```

### 11.3 Payload marshaling & ownership across the FFI

**Do NOT mirror `tlm_generic_payload` as a `#[repr(C)]` struct.** Its `tlm_array<T>` is a private-inheritance `std::vector` subclass (no stable ABI), its extension indices are assigned at C++ static-init time, and its deleted copy ctor + `deep_copy_from` encode subtle byte-enable rules. Instead treat the C++ `tlm_generic_payload*` as an **opaque handle** and expose typed accessors; the data buffer is a **borrowed slice** tied to the call frame (never freed by Rust):

```rust
pub struct GpRef<'a> { raw: *mut ffi::TlmGenericPayload, _life: PhantomData<&'a mut ()> }
impl<'a> GpRef<'a> {
    pub fn command(&self) -> Command { ffi::gp_get_command(self.raw).into() }
    pub fn data(&mut self) -> &'a mut [u8] {            // valid only for 'a; C++/initiator owns it
        let p = ffi::gp_get_data_ptr(self.raw);
        let n = ffi::gp_get_data_length(self.raw) as usize;
        unsafe { core::slice::from_raw_parts_mut(p, n) }
    }
}
```

Ownership rules across FFI: the GP object (no MM) is freed only by its creating side, Rust never drops a C++-created GP; with an MM, a `GpHold` RAII guard proxies the C++ `acquire`/`release` ref-count so the "initiator holds for the whole transaction" rule cannot be violated by accident; data/byte-enable buffers are always borrowed, never freed by Rust; extension indices are resolved from the C++ registry at runtime via a shim and cached in a `OnceLock`. DMI back-door pointers are meaningful only within one address space — the out-of-process bridge refuses DMI.

```rust
pub struct GpHold { raw: *mut ffi::TlmGenericPayload }
impl GpHold {
    pub fn acquire(raw: *mut ffi::TlmGenericPayload) -> Self {
        assert!(ffi::gp_has_mm(raw), "acquire requires a memory manager");
        ffi::gp_acquire(raw); GpHold { raw }
    }
}
impl Drop for GpHold { fn drop(&mut self) { ffi::gp_release(self.raw); } }
```

### 11.4 Both directions

**Rust initiator → C++ target:** the Rust socket holds an opaque `*mut tlm_fw_transport_if` and calls the shim, which performs the real C++ virtual `b_transport`; the possibly-increased delay is copied back and (in 1a) settled via `sc_wait`. **C++ initiator → Rust target:** the shim's C++ `RustBackedTarget : tlm_fw_transport_if` routes each virtual to a `srs_*` function carrying the `void* rust` handle; the Rust entry uses a `catch_unwind` firewall, builds a `GpRef`, and runs pure Rust (which may `cx.wait()` for LT timing). Phase semantics carry across unchanged because both sides reference the same C++ `tlm_phase` registry and the same GP object.

### 11.5 Phase 3: out-of-process, quantum-synchronized co-simulation

When in-process is impossible or undesirable (crash isolation, language/runtime isolation, mixed-host/scaling, IP separation), run the two kernels as separate OS processes connected by a transport, synchronized by a **conservative quantum barrier** (the same `compute_local_quantum()` grid). Cross-boundary transactions are buffered and delivered at the next barrier with their timestamp; the schedule is deterministic given the same quantum and tie-break sequence numbers. Each side hosts a proxy socket that serializes the GP (command, address, data bytes, byte-enables, response slot, extensions by registry name), ships it, and blocks the calling thread on an event until the response returns — structurally identical to the b↔nb adapter. The cost is loss of delta-cycle fidelity: this is inherently an LT/quantum-synchronized model. Transport: shared memory + eventfd (same host, zero-copy large `data`) or gRPC (`tonic`+`prost`, cross host). DMI must be disabled unless backed by shared memory with correct address translation; honour the invalidate re-entrancy ban.

### 11.6 Risk register (interop)

| Risk | Severity | Mitigation |
|---|---|---|
| C++ ABI/STL instability (`tlm_array`, `sc_time` layout) | High | never pass container layouts; opaque `cxx` handles + accessor shims; pin compiler + `_GLIBCXX_USE_CXX11_ABI`. |
| C++ exception unwinding *through* Rust frames (`SC_REPORT_ERROR`/`FATAL` from `sc_wait`, `sc_report`-as-throw, `sc_unwind_exception` for kill/reset), interleaved with Rust frames and the QuickThreads stack switch | High | a C++ exception traversing Rust frames is UB under `panic = "unwind"`, and Rust-entry `catch_unwind` does **not** cover it (the throw originates below the entry). Mitigation is *symmetric*: the C++ shim does `try { … } catch (...)` around every `sc_wait`/`b_transport`/`nb_transport_*` that can re-enter Rust, converting the throw to a status code / "terminate this process" signal *before* returning across the boundary; the Rust side then sees it as a normal `Err(ProcessControl::{Kill,Reset})` from `wait()`, never as a live unwind. |
| Panic across FFI / coroutine stack | High | `catch_unwind` firewall on every Rust `extern "C"` entry → `SC_REPORT_FATAL`; optionally `panic = "abort"` for the FFI build so a stray panic aborts deterministically instead of unwinding into C++. |
| Coroutine-stack interplay | Med | safe only if *neither* a Rust panic *nor* a C++ exception crosses the SP switch (both firewalls above active) and no `&mut Kernel` is held across the yield (borrowed `Cx` valid only while running). |
| GP double-free / use-after-recycle | High | RAII `GpHold`; buffers always borrowed; honour MM-present asserts. |
| Extension index mismatch | Med | resolve indices from the C++ registry at runtime; cache in `OnceLock`. |
| Determinism drift out-of-process | Med (by design) | conservative quantum barrier + deterministic tie-breaks; document that delta-exact equivalence is not preserved across the bridge. |
| DMI across processes | Med | disable unless shared-memory-backed with address translation; honour the invalidate ban. |

---

## 12. Phased roadmap / MVP milestones

Ordered to de-risk the concurrency model and interop earliest.

### Milestone 0 — Time, events, and the bare delta loop (the riskiest 200 lines)
Build the kernel skeleton with no processes: `SimTime`, the three-phase `crunch()`, `sc_event` with the full collapse state machine, the timed heap with explicit `seq` tie-breaks, the counters.
**Exit:** raw-event scheduling tests assert fire order against a hand-computed reference (delta-overrides-timed, earliest-wins, empty-deltas-don't-count); `triggered()` true only within the firing change-stamp window; a 1000-sequence property test never panics or double-fires.

### Milestone 1 — Process model: settle the concurrency bet
Commit to stackful coroutines via `corosensei`; build a throwaway async prototype for one week to confirm the colouring cost, then commit. `SC_METHOD` as `FnMut`.
**Exit:** `wait(time)` works three helper-calls deep inside an `SC_THREAD`; two threads + one method in one delta produce the documented evaluate order; immediate self-notification guard verified; a 10k-thread `wait(1ns)` benchmark runs without stack exhaustion at a documented throughput; a "stackful vs async" decision memo is committed with prototype evidence.

### Milestone 2 — Modules, hierarchy, ports/exports, elaboration barrier
Arena object store, `#[module]` macro, four lifecycle callbacks in fixed registry order with the construction fixpoint, deferred two-phase binding, `complete_binding`.
**Exit:** a two-level hierarchy elaborates with correct unique names; a channel bind and a hierarchical port-to-port bind both resolve; binding after `start` is a compile error or clean `Err`; a module created in `before_end_of_elaboration` still gets its callback.

### Milestone 3 — Channels + first end-to-end LT transaction
`prim_channel` update queue, `Signal<bool>`/`Fifo` over `VecDeque`, the init update pass; minimal TLM-2 (GP with `Rc`+pool MM, `TypeId` extensions, `b_transport`, initiator/target/passthrough/simple sockets).
**Exit:** a FIFO producer/consumer shows the "written in N, readable in N+1" rule; an initiator `b_transport(read)`s to a memory target across a bound socket with correct data, `t`, and response; the payload pool recycles with no leaks under stress.

### Milestone 4 — Temporal decoupling + AT protocol + PEQ
Quantum keeper, global quantum, `peq_with_get` then the phase-aware PEQ with delta parity, `nb_transport_fw/bw` four-phase, b↔nb adapters.
**Exit:** an LT initiator runs ahead and syncs on grid boundaries (sync count matches a hand calculation); a full BEGIN_REQ→END_RESP exchange completes with all three `TlmSync` paths exercised; LT-initiator↔AT-target and AT-initiator↔LT-target both work; two same-time zero-delay notifications fire one delta apart, FIFO.

### Milestone 5 — Observability & reporting
Reporting (severity/action/verbosity with exact precedence as a pure fn; ERROR→`Result`, FATAL→abort), analysis ports + analysis fifo, off-thread telemetry writer, transaction-record sink, `transport_dbg` query API.
**Exit:** fan-out `write()` reaches N subscribers synchronously in registration order; analysis fifo never back-pressures; report action precedence matches a golden table; telemetry-on vs -off traces are identical.

### Milestone 6 — Digital-twin layer
`RealTimePacer`, `ExternalInput` inbox + suspend-on-starvation, seeded RNG service, input journal + replay.
**Exit:** a twin paces to wall clock within tolerance and emits slip telemetry; an externally-driven model parks (does not exit) when idle and resumes on injection; a recorded journal + seed replays to a byte-identical transaction trace.

### Milestone 7+ (DEFER)
Snapshot/restore (in the *bounded* sense of §6f: arena columns + kernel queues + resumable-state-machine processes blocked at `wait`, **not** transparent native-coroutine-stack capture, which stays research-grade), structural hot-swap, endianness helpers, instance-specific extensions, full kill/reset throw semantics, and the optional Tier-1 parallel region orchestrator (with `--verify-determinism` and integer-only time arithmetic per §8a). Gated on demand; the arena + resumable-process design from M0–M1 must not preclude these.

---

## 13. Risks & open questions

### Risks (ranked)
1. **Stackful-coroutine determinism & cost (M1).** If the backend introduces nondeterminism, hidden allocation, or unacceptable per-thread stack overhead at twin scale, the concurrency bet is wrong. *Mitigation:* the parallel async prototype in M1; benchmark stack usage early; keep the `CoroutineRuntime` trait so the backend is swappable.
2. **Async vs stackful is a one-way door.** It colours the entire public API. *Mitigation:* settle in M1 with evidence; document irreversibly.
3. **Snapshotting a suspended coroutine.** Arbitrary native-stack capture is not portable: a `corosensei` fiber parked mid-`b_transport` holds its resume point and live locals on a machine stack that does not `serde`-serialize across machines, rebuilds, or reliably across layout shifts. *Mitigation (and explicit scope bound):* never snapshot a raw stack. Snapshot only at a timestep boundary with every thread blocked at `wait()`; serialize the *kernel-visible* waiting state (installed `Sensitivity`, `ProcessId`) plus model state held in arena columns — **not** stack locals; require snapshottable thread bodies to be resumable state machines whose resume points are the `wait()` calls (locals that must survive live in serializable component state). Threads with non-trivial live locals on the native stack across a `wait` are explicitly *not* snapshottable in the MVP; transparent stack capture is research-grade future work (see §6f).
4. **Trace-equivalence scope creep.** Bit-exact compatibility partly couples us to C++ implementation-defined behaviour. *Mitigation:* define a conformance tier — guarantee determinism and IEEE-1666 *semantic* equivalence, with explicit (seeded) tie-breaks documented where they differ from C++ heap order.
5. **Payload aliasing across yields.** The AT shared-mutable transaction is the central borrow-checker fight. *Mitigation:* `Rc<RefCell<Payload>>` on the AT path, plain `&mut` on the LT path.
6. **Telemetry off-thread is the one place real concurrency exists.** *Mitigation:* serialize records to owned `Send` structs at the boundary; the core stays `Rc`-based.
7. **DMI raw-pointer safety / un-snapshotability.** *Mitigation:* model DMI as an arena handle + slice with a runtime re-entrancy guard.

### Open questions
- **Backend:** `corosensei` vs `generator` vs a hand-rolled fiber — which gives guard pages, low stack overhead, and stable behaviour on the WSL/Linux targets? (M1 spike.)
- **Time resolution:** const-generic (impossible to change after start) vs runtime-builder-frozen field? (Leaning runtime-builder-frozen.)
- **Conformance target:** binary/IPC interoperation with real SystemC, or only semantic fidelity? This decides how literally we replicate trigger ordering.
- **Multiple simulations per process:** per-runtime state (no globals) enables what-if forking but adds a handle to every API. (Leaning yes.)
- **Immediate notification:** expose-but-discourage (compatibility) vs omit (non-determinism footgun)? (Leaning expose-but-discourage behind a clearly-named call.)
- **Snapshot granularity:** timestep-only (simple) vs mid-delta (much harder)? (MVP timestep-only.)
- **Quantum default:** what global quantum balances simulation speed vs real-time pacing granularity for a twin?

---

## 14. Appendix: SystemC → SystemRS naming map

| SystemC / TLM construct | SystemRS equivalent | Notes |
|---|---|---|
| `sc_simcontext` | `Kernel` / `SimContext` | arena-owning scheduler; typestate `Building`/`Running` |
| `sc_time`, `SC_ZERO_TIME`, `sc_time::max()` | `Time(u64)`, `Time::ZERO`, `Time::INF` | resolution units; `Time::INF = u64::MAX` is bit-for-bit `sc_time::max()` (`~value_type{}`, sc_time.h:254-256), not just a chosen sentinel |
| `sc_event` | `Event` (arena entry, `EventId`) | `Pending {None,Delta,Timed}` collapse state |
| `notify()` / `notify(SC_ZERO_TIME)` / `notify(t)` | `notify_immediate` / `notify_delta` / `notify_timed` | priority immediate>delta>timed |
| `sc_process_b` / `sc_process_handle` | `Process` (arena, `ProcId`) / `ProcessHandle (ProcId, gen)` | refcount dissolved into generation |
| `SC_METHOD(f)` | `cx.method("n", f)` → `ProcessBody::Method(FnMut)` | run-to-completion |
| `SC_THREAD(f)` | `cx.thread("n", f)` → `ProcessBody::Thread(Coroutine)` | stackful coroutine |
| `SC_CTHREAD` | — | dropped (RTL) |
| `wait(...)` / `next_trigger(...)` | `cx.wait_*(...)` / `.next_trigger(...)` | sync call from any depth |
| `sensitive << ev` | `.sensitive_to(&ev)` builder | no hidden last-process state |
| `sc_object` / `sc_module` | `Object` (`ObjectId`) / `Module` + `#[module]` | `cx.module("n", \|m\| {..})` |
| lifecycle callbacks | `trait Elaborate` (default-empty methods) | fixed registry order, fixpoint |
| `sc_interface` / `sc_port<IF>` / `sc_export<IF>` | `trait Interface` / `Port<IF>` (`BindState`) / `Export<IF>` | two-phase deferred bind |
| `sc_signal<T>` / `sc_buffer<T>` | `Signal<T, P>` / `Buffer<T>` | `Cell` double-buffer; Buffer always fires |
| `sc_fifo<T>` | `Fifo<T>` over `VecDeque` | `Capacity {Bounded,Unbounded,Zero}` |
| `sc_clock` / `sc_mutex` / `sc_semaphore` | `Clock` (self-scheduling) / `Mutex` / `Semaphore` | |
| `sc_report` / `SC_REPORT_*` | `Report` + `ReportError`; `tracing` macros | ERROR→`Result`, FATAL→abort |
| `sc_trace` (VCD) | `TraceSink` + transaction recorder | VCD/FST optional |
| `sc_vector<T>` | `ModuleVec<T>` / `Vec<T>` + scoped builder | |
| `sc_dt::*` (`sc_int`, `sc_bigint`, `sc_logic`, …) | native `u8`/`u32`/`u64`, `Vec<u8>` | datatypes library dropped |
| `tlm_generic_payload` | `GenericPayload`; `Txn = Rc<RefCell<GenericPayload>>` | owned `Vec<u8>` data; `TxnPool` |
| `tlm_mm_interface` / acquire/release | `TxnPool` + `Rc` strong count | |
| `tlm_extension<T>` / extension array | `trait Extension` + `ExtensionMap` (`TypeId`) | `clone_ext() -> Option`, no RTTI |
| `tlm_command` / `tlm_response_status` | `enum Command` / `enum ResponseStatus` | OK=1 sole OK, INCOMPLETE=0 (not error), errors −1..−5; `is_error()` total = "discriminant < 0" |
| `tlm_sync_enum` | `enum TlmSync { Accepted, Updated(Phase), Completed }` | phase carried in `Updated` |
| `tlm_phase` / extended phases | `enum Phase {…, Extended(PhaseId)}` | base phases cheap discriminants |
| `tlm_fw_transport_if` / `tlm_bw_transport_if` | `trait FwTransport<P>` / `trait BwTransport<P>` | `Protocol` associated types |
| `b_transport` / `nb_transport_*` / `transport_dbg` | same names; `b_transport` may `cx.wait`; `transport_dbg` no `Cx` | |
| `tlm_dmi` | `Dmi` + `DmiAccess` bitflags | backdoor as arena handle/slice |
| `tlm_initiator_socket` / `tlm_target_socket` | `InitiatorSocket<BW,P>` / `TargetSocket<BW,P>` (`SocketId`) | const-generic width; crossed double-bind |
| `simple_*` / `passthrough_*` / `multi_*` sockets | closure-registering convenience sockets + LT↔AT adapters | boxed closures, no `void*` trampoline |
| `tlm_quantumkeeper` / `tlm_global_quantum` | `QuantumKeeper` / `GlobalQuantum` (in the runtime) | not a singleton |
| `peq_with_get` / `peq_with_cb_and_phase` | `PeqWithGet` / `PhaseQueue` over `BTreeMap<(SimTime,seq),_>` | deterministic ties |
| `tlm_analysis_port` / `tlm_write_if` / `tlm_analysis_fifo` / `tlm_analysis_triple` | `AnalysisPort` / `AnalysisWrite` / `AnalysisFifo` / `AnalysisTriple` | telemetry backbone |

---

## 15. References (into `external/systemc`)

Key file:line citations grounding the design, gathered from the subsystem analyses.

**Kernel & scheduler**
- `src/sysc/kernel/sc_simcontext.cpp:497-637` — `crunch`: evaluate/update/notify three-phase delta loop.
- `src/sysc/kernel/sc_simcontext.cpp:511-554` — evaluate phase: toggle methods then threads, run to empty.
- `src/sysc/kernel/sc_simcontext.cpp:564-567` — update phase bumps `m_change_stamp` *before* `perform_update()`, guarded by `!empty_eval_phase`.
- `src/sysc/kernel/sc_simcontext.cpp:614` — `m_delta_count++` guarded by `!empty_eval_phase` (empty delta advances neither counter).
- `src/sysc/kernel/sc_simcontext.cpp:871-970` — `simulate`: time advance, timed-event popping, starvation, SC_ZERO_TIME.
- `src/sysc/kernel/sc_simcontext.cpp:972-988` — `do_timestep`: `sc_assert(m_curr_time < t)` (975), set time, `m_change_stamp++` on *every* time advance (986), reset per-time delta baseline (987).
- `src/sysc/kernel/sc_simcontext.cpp:654-821` — `elaborate` + `prepare_to_simulate`: lifecycle, callbacks, initial scheduling.
- `src/sysc/kernel/sc_runnable_int.h:471-496` — `toggle_methods`/`toggle_threads` push/pop double-buffer.
- `src/sysc/kernel/sc_time.h:92-181, 254-256`; `sc_time.cpp:153-165, 331-394` — 64-bit time, `max()` = `max_time_tag` ctor `m_value( ~value_type{} )` (all-ones), integer scaling, resolution freeze.

**Processes & coroutines**
- `src/sysc/kernel/sc_cor.h:86` — `sc_cor_pkg` interface: create/yield/abort/get_main.
- `src/sysc/kernel/sc_cor_qt.cpp:234, 269, 84` — QuickThreads create/yield/destroy (the stackful analogue).
- `src/sysc/kernel/sc_thread_process.h:211` — `suspend_me`: yield + throw-on-resume state machine.
- `src/sysc/kernel/sc_method_process.h:296` — `run_process`: run-to-completion + reset restart loop.
- `src/sysc/kernel/sc_thread_process.cpp:107, 257, 536, 596, 674` — coroutine top-level, kill, throw_reset, throw_user, trigger_dynamic.
- `src/sysc/kernel/sc_simcontext.cpp:1422` — `preempt_with`: nested immediate execution, three caller cases.

**Events & sensitivity**
- `src/sysc/kernel/sc_event.cpp:86-101` — immediate notify must be in evaluate phase.
- `src/sysc/kernel/sc_event.cpp:103-155` — delta/timed notify collapse (earliest wins, delta overrides timed).
- `src/sysc/kernel/sc_event.cpp:378-458` — `trigger` ordering, verified: static methods → dynamic methods → static threads → dynamic threads; static lists iterate high-index→0, dynamic lists iterate 0→high with tail swap-in + `resize`.
- `src/sysc/kernel/sc_thread_process.h:474-510` — `trigger_static`: per-process self-notification guard `if (sc_get_current_process_b() == this) { report_immediate_self_notification(); return; }`, plus runnable/queued/wait-count predicate.
- `src/sysc/kernel/sc_simcontext.cpp:1376-1405` — `next_time`, O(1) delta cancel via swap.
- `src/sysc/communication/sc_event_queue.h:33-50` — lossless event queue contract.

**Modules, ports, channels**
- `src/sysc/kernel/sc_module.cpp:119-143, 247-275` — name-stack discovery, `end_module` finalization.
- `src/sysc/kernel/sc_simcontext.cpp:654-706, 1067-1072` — fixed registry callback order; end_of_simulation.
- `src/sysc/communication/sc_port.cpp:274-322, 417-557` — deferred bind, `complete_binding`, policy enforcement.
- `src/sysc/communication/sc_prim_channel.cpp:325-346` — registry `perform_update`, async drain first.
- `src/sysc/communication/sc_signal.h:289-300, 323-337`; `sc_signal.cpp:159-164` — write stages, update commits, next-delta notify.
- `src/sysc/communication/sc_fifo.h:242-289, 343-356` — immediate buffer mutation + delta-deferred events.

**TLM-2.0**
- `src/tlm_core/tlm_2/tlm_generic_payload/tlm_gp.h:42-46, 132-145, 149-150` — `tlm_mm_interface`, acquire/release, deleted copy.
- `src/tlm_core/tlm_2/tlm_generic_payload/tlm_gp.h:96-103` — `tlm_response_status`: OK=1 (sole OK), INCOMPLETE=0 (initial, not error), errors −1..−5 (`is_error` ⇔ discriminant < 0).
- `src/tlm_core/tlm_2/tlm_generic_payload/tlm_gp.cpp:34-82, 126-130, 311-358` — extension registry, `reset`, extension removal semantics.
- `src/tlm_core/tlm_2/tlm_2_interfaces/tlm_fw_bw_ifs.h:36-57, 63-158, 166-191, 197-225` — transport methods, DMI, dbg, aggregate ifs.
- `src/tlm_core/tlm_2/tlm_generic_payload/tlm_phase.h:33-58` — base phases.
- `src/tlm_core/tlm_2/tlm_sockets/tlm_initiator_socket.h:113-155`; `tlm_target_socket.h:113-159` — crossed double-bind, hierarchical-bind checks.
- `src/tlm_utils/simple_target_socket.h:137-163, 298-339, 417-510` — b↔nb conversion, pending-transaction map.
- `src/tlm_utils/tlm_quantumkeeper.h:71-127`; `tlm_global_quantum.cpp:37-47` — local time, sync, grid-aligned quantum.
- `src/tlm_utils/peq_with_cb_and_phase.h:219-282` — delta-parity bucketing, three-tier drain order.

**TLM-1.0 & support**
- `src/tlm_core/tlm_1/tlm_analysis/tlm_analysis_port.h:30-78` — synchronous in-order fan-out.
- `src/tlm_core/tlm_1/tlm_req_rsp/tlm_channels/tlm_fifo/tlm_fifo.h:185-254` — evaluate/update FIFO visibility.
- `src/sysc/utils/sc_report_handler.cpp:276-325` — action-resolution precedence.
- `src/sysc/tracing/sc_trace_file_base.cpp:80,178` — stage-callback-driven sampling (PRE_TIMESTEP/POST_UPDATE).
