# Why SystemRS

## What "transaction-level" means

A digital system can be modelled at many altitudes. At the bottom is **RTL** — every
wire, every clock edge, bit-accurate and cycle-accurate, but slow and verbose. At the
top is an untimed functional model. **Transaction-level modeling (TLM)** sits in
between: components communicate by passing *transactions* (a read of 4 bytes at address
`0x40`, a burst write, a packet) through function calls, not by toggling wires. A CPU
model issues a `read` and gets bytes back; it does not model the address/data/handshake
signals that carry them. This is fast enough to boot an OS, yet structured enough to
measure latency, contention, and throughput.

SystemRS is **TLM-only**. It reproduces the parts of SystemC and TLM-2.0 needed to
author transaction-level models, and deliberately omits the RTL machinery — resolved
multi-driver signals, the `sc_dt` fixed/arbitrary-precision datatype library, clocked
`SC_CTHREAD`s — that a transaction-level tool does not use. (See §2 and §4 of the design
report for the full feature-by-feature decisions.)

## Why Rust

A SystemC kernel is a graph of raw C++ pointers with intricate object-lifetime and
destruction-order rules, RTTI-based dispatch, and `sc_report`-as-exception error
handling. SystemRS keeps the *semantics* — the scheduler, the generic payload, the
four-phase handshake — and replaces the *mechanisms* with Rust ones:

- An **arena + generational-id** object store dissolves the pointer graph: components
  refer to each other by small `Copy` ids, and whole classes of use-after-free and
  destruction-order bugs simply cannot occur.
- **Stackful coroutines** give `SC_THREAD`-style processes that can `wait()` from any
  call depth — no `async` colouring spreading across your forward path.
- **Sum types** (`enum`) replace signed-integer status conventions, and **`Result`**
  replaces thrown reports.

## Digital twins

A *digital twin* is a long-lived, observable, sometimes wall-clock-coupled model of a
real system. SystemC was designed as a batch simulator; a twin needs more: it must
track real time, accept external inputs between timesteps without exiting when idle,
reproduce a run deterministically for forensic replay, and stream telemetry without
perturbing timing. These are first-class subsystems in SystemRS (the
[Digital Twins](../twins/needs.md) part), built on the same deterministic core.

## When to reach for it

Reach for SystemRS when you want a fast, deterministic, *observable* model of a
digital system — a SoC, an accelerator, a signal-processing pipeline, a sensor node —
in safe Rust, especially one that will run as a long-lived twin alongside real inputs.
Reach for an RTL simulator instead when you need bit- and cycle-accurate wire-level
fidelity.

> **Go deeper:** design report §1 (executive summary), §2 (what TLM-only means), §4
> (feature-coverage decisions).
