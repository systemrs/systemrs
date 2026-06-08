# Worked example: a fixed-point reverb

`cargo run --example reverb` is a transaction-level **DSP model**: an electric-guitar
reverb pedal that streams blocks of audio samples through a comb-and-allpass reverb (with
a tremolo), all in bit-accurate fixed point. It brings together four threads of this
guide at once.

- **Bring-your-own datatypes.** Samples are `qfixed` `Q2.14`, and the tremolo LFO is a
  *complex* `CQ` numerically-controlled oscillator — an IQ phasor rotated by one complex
  multiply per sample. This is the [datatypes](datatypes.md) story made concrete: a
  fixed-point IQ datapath, the same shape a radio or audio twin needs.
- **Streaming over TLM with temporal decoupling.** The pedal is a target socket; an
  initiator `b_transport`s one block of samples per transaction, processed in place,
  advancing time by the block duration — a [quantum](temporal-decoupling.md) of audio.
- **Non-intrusive observability.** Each block's peak level is broadcast on an
  `AnalysisPort` (the [analysis ports](../obs/analysis-ports.md) chapter), so a meter can
  watch the signal without perturbing it.

The DSP core is a per-sample function — a feedback comb for the echo train, a Schroeder
allpass for diffusion, then the tremolo gain — using `qfixed`'s saturating operators:

```rust,ignore
{{#include ../../../../crates/systemrs-examples/src/reverb.rs:process}}
```

The pedal wraps that in a `register_b_transport` callback: it unpacks the block's
`Q2.14` samples from the payload bytes, runs each through `process_sample`, packs the
results back, and writes the block's peak level to the analysis port. The initiator
streams a plucked-string signal followed by silence; the demo's ASCII meter shows the
reverb tail ringing out after the pluck stops — and the long comb echo arriving later.

This is the pattern for *any* transaction-level signal processing: carry your samples as
bytes in the payload, do the math in saturating fixed point, advance one block-quantum
per transaction, and tap the output on an analysis port.

> **Go deeper:** design report §6d (streaming TLM), §6e (observability), §3.12 (no
> `sc_dt`). Full source: `crates/systemrs-examples/src/reverb.rs`.
