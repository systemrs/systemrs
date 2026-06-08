# SystemC → SystemRS map

If you are coming from SystemC, this table is your Rosetta stone for the common names.
SystemRS drops the `sc_`/`tlm_` prefixes and the name-stack/macro ceremony; behaviour is
preserved, surface is modernised. (The design report §14 has the complete map.)

| SystemC / TLM-2.0 | SystemRS |
|---|---|
| `sc_module`, `SC_MODULE` | `module(...)` / `#[module]` |
| `SC_THREAD` | `sim.add_thread(...)` / `b.thread(...)` |
| `SC_METHOD` | `sim.add_method(...)` / `sim.method(...)` |
| `sc_event`, `.notify()` | `EventId`, `cx.notify(ev)` |
| `wait()`, `wait(ev)` | `cx.wait(dt)`, `cx.wait_event(ev)` |
| `sensitive << ev` | `.sensitive_to(ev)` / the `&[events]` slice |
| `sc_signal<T>` | `Signal<T>` |
| `sc_clock` | `Clock` |
| `sc_fifo<T>` | `Fifo<T>` |
| `sc_time` | `SimTime` |
| `sc_start(t)` | `sim.run_until(t)` |
| `sc_report`, `SC_REPORT_*` | `diag` (`Severity`, free fns, `ReportHandler`) |
| `tlm_generic_payload` | `GenericPayload` |
| `tlm_initiator_socket` | `InitiatorSocket` |
| `tlm_target_socket` | `TargetSocket` |
| `b_transport(...)` | `isock.b_transport(cx, &mut gp, &mut delay)` |
| `nb_transport_fw` / `_bw` | `nb_transport_fw` / `nb_transport_bw` |
| `tlm_phase` | `Phase` (`BeginReq`, `EndReq`, `BeginResp`, `EndResp`) |
| `tlm_sync_enum` | `TlmSync` (`Accepted`, `Updated(_)`, `Completed`) |
| `tlm_analysis_port` | `AnalysisPort<T>` |
| `tlm_analysis_fifo` | `AnalysisFifo<T>` |
| `tlm_utils::peq_with_get` | `PeqWithGet<T>` / `PhaseQueue` |
| `tlm_quantumkeeper` | `QuantumKeeper`, `set_global_quantum` |
| value-change tracing (`sc_trace`) | `Tracer` + a `TraceSink` |
| `sc_dt` (datatypes) | **dropped** — bring your own (e.g. `qfixed`) |

Three deeper shifts beyond the names:

- **`sc_report`-as-exception → `Result`.** A recoverable error is a `Result` you `?`;
  only fatal aborts. ([reporting](../obs/reporting.md))
- **Raw-pointer graph → arena ids.** You hold `Copy` handles, not pointers, and pass them
  by value. ([mental model](../getting-started/mental-model.md))
- **Freeze-on-first-use binding → an explicit elaboration barrier** with a
  `Building → Running` typestate. ([modules](../core/modules.md))

> **Go deeper:** design report §14 (the full naming map), §7 (making it idiomatically
> Rust).
