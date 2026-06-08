# Real-time pacing

A batch simulation runs as fast as the CPU allows — a microsecond of simulated time might
take a nanosecond of wall-clock time, or vice versa. A twin that mirrors a real system,
or that drives real hardware/UI, needs simulated time to track **wall-clock time**. That
is what a `RealTimePacer` does.

```rust,ignore
let pacer = RealTimePacer::new(1.0, SimTime::from_us(1)); // wall-ns per sim-ns, tolerance
pacer.install(&sim);
// ...run the sim; read slip telemetry afterward:
let stats = pacer.stats();
```

The pacer hooks the kernel's **time-advance** hook, so *only time advance is paced* —
delta cycles stay instantaneous. On each advance it computes where wall-clock time should
be (from the simulated time elapsed since it started, in femtoseconds, scaled by the
factor) and, if the simulation has run *ahead* by more than the tolerance, sleeps to
re-align.

- **`scale`** is wall nanoseconds per simulation nanosecond: `1.0` is real time, `< 1.0`
  runs faster than real time, `> 1.0` slower.
- **`tolerance`** is how far ahead the sim may drift before the pacer sleeps.

Pacing is observable as **slip telemetry**: `stats()` returns a plain `Copy` `PacerStats`
— how far behind or ahead wall clock the last advance was, the largest slip seen, and how
often it had to sleep. (The slip is exposed as plain stats rather than a trace event
precisely so the pacer needs no dependency on the tracing layer.)

Pacing changes *wall-clock* timing only — never the simulation result. A paced run and an
unpaced run compute the same thing; the pacer only decides how fast real time passes
while it does. That is what lets a burst of queued external inputs be processed at a
steady, human-watchable cadence instead of all at once — exactly what the
[sensor twin](ex-twin.md) demonstrates.

> **Go deeper:** design report §6f (real-time pacing on the time-advance hook).
