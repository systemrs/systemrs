# Where to go next

You now have the whole arc: from a clock and a counter, through loosely- and
approximately-timed transactions, observability, and the digital-twin layer. Where to go
from here:

- **The API reference.** Every public type carries rustdoc with worked examples. Build
  and open it with `cargo doc --open` (or browse it online). The high-traffic types —
  `Sim`, `Signal`, `Fifo`, `Memory`, `AnalysisPort`, `Tracer`, `SimTime`, `Rng` — all
  have runnable `# Examples` snippets.

- **The example sources.** The five models this guide draws on are the best next read —
  each is a complete, tested model:
  - `counter` — signals and a clocked method
  - `rv32i_hart` — a CPU over loosely-timed transport
  - `dma` — a DMA engine over the AT protocol
  - `reverb` — fixed-point DSP streamed over TLM
  - `twin` — a real-time, replayable sensor twin

- **The modeling skill.** If you author models with Claude Code, the
  [`systemrs-modeling` skill] distils the idioms (refer-by-id, the `!Send`/spawned-body
  rule, the LT/AT patterns, the twin wiring) into an applied guide.

- **The design report.** [`doc/systemrs-design.md`] is the authoritative specification —
  the rationale behind every decision this guide paraphrases, with the `§…` sections
  cited throughout. Read the section a chapter points to when you want the *why* in full.

## Beyond the modelling core

Two capabilities reach past the single-threaded modelling core, both covered in the
**Going Further** part:

- **[Parallel execution](../advanced/parallel.md)** — deterministic, barrier-synchronous
  PDES that runs disjoint regions in parallel and re-converges at quantum boundaries, with
  a Tier-1 run bit-identical to the serial Tier-0 reference (design §8a).
- **[Snapshot and restore](../advanced/snapshots.md)** — bounded checkpointing: capture
  the scheduler at a quiescent boundary and resume from it (design §6f).

## On the horizon

One major piece is designed but not yet built:

- **SystemC interoperability** — running Rust models as guests inside a C++ SystemC kernel,
  and eventually the reverse and out-of-process co-simulation (design §11). It lives in a
  **separate bridge repo** (so this core stays pure Rust) that plugs into the in-core seam:
  a swappable `Rc<dyn FwTransport>` forward-transport target
  (`TargetSocket::set_fw_transport`) plus the generic-payload byte API. A *migration guide*
  for SystemC users will land alongside it.

The deterministic single-threaded core remains the golden reference — everything you need
to model and observe a digital system as a twin, and the reference a parallel run is
verified against.

[`systemrs-modeling` skill]: https://github.com/systemrs/systemrs-modeling-skill
[`doc/systemrs-design.md`]: https://github.com/londey/systemrs/blob/master/doc/systemrs-design.md
