# Bring your own datatypes

SystemC ships `sc_dt`, a ~58k-line library of fixed-point and arbitrary-precision integer
types. SystemRS **deliberately drops it** (design §3.12): a transaction-level tool does
not need RTL arithmetic, and the generic payload carries *bytes*, not typed values. This
is not a gap — it is the design philosophy:

> You bring your own datatypes as idiomatic Rust, and the generic payload carries their
> bytes. No 58k-line datatype library required.

## How it works in practice

Whatever a transaction *means* — a pixel, a network packet, a fixed-point audio sample —
you model that type in plain Rust and serialise it into the payload's data buffer. A
target deserialises, computes, and writes the result back. The payload stays a dumb byte
container; the meaning lives in your model.

For example, the [reverb](ex-reverb.md) carries blocks of **Q-format fixed-point** audio
samples. It uses the external [`qfixed`] crate, whose `Q<I, F>` (signed) and `CQ<I, F>`
(complex/IQ) types are bit-accurate fixed-point with TI-style Q notation and type-level
overflow safety — exactly what a DSP or radio twin wants. A 16-bit `Q2.14` sample packs
to two little-endian bytes:

```rust,ignore
use qfixed::Q;
use qfixed::typenum::{U2, U14};
type Sample = Q<U2, U14>;          // 1 sign + 1 integer + 14 fractional bits

let s = Sample::from_f64(0.5);
let bytes = s.to_bits().to_le_bytes(); // -> the payload's data buffer
// ...and back out of a payload:
let r = Sample::from_bits(u64::from(u16::from_le_bytes([bytes[0], bytes[1]])));
```

That `to_bits`/`from_bits` round-trip is all the "marshaling" a custom datatype needs.
`qfixed`'s arithmetic is *saturating* (it clamps at the format limits rather than
wrapping — the honest behaviour of a fixed-width register), and its widening operators
track bit-growth at the type level, so an overflow is a compile error, not a silent
wrap.

The takeaway: SystemRS is datatype-agnostic. Pick (or write) the Rust type that fits your
domain, give it a byte representation, and the whole TLM machinery carries it unchanged.

> **Go deeper:** design report §3.12 (why `sc_dt` is dropped), §3.8 (the generic
> payload's byte buffer).

[`qfixed`]: https://github.com/systemrs/qfixed
