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

## On the horizon

A couple of things this guide does not yet cover, because they are not yet built:

- **SystemC interoperability** (`systemrs-ffi`) — running Rust models as guests inside a
  C++ SystemC kernel, and eventually the reverse and out-of-process co-simulation
  (design §11). A *migration guide* for SystemC users will land alongside it.
- **Optional parallel execution** — barrier-synchronous parallel discrete-event
  simulation that runs disjoint regions in parallel and re-converges at quantum
  boundaries, preserving deterministic replay (design §8a).

Until then: the deterministic single-threaded core is the golden reference, and it is
everything you need to model and observe a digital system as a twin.

[`systemrs-modeling` skill]: https://github.com/systemrs/systemrs-modeling-skill
[`doc/systemrs-design.md`]: https://github.com/londey/systemrs/blob/master/doc/systemrs-design.md
