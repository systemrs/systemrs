//! [`Rng`] — a seeded, deterministic PRNG service (`doc/systemrs-design.md` §6f, §8).
//!
//! Deterministic replay requires that **all** twin randomness be reproducible: there
//! is no ambient `thread_rng`. Models draw from this `Rng`, installed as a [`Sim`]
//! service under a known seed; the seed is recorded in the run header / journal so a
//! replay reproduces the exact draw sequence. The generator is SplitMix64 — small,
//! fast, no external `rand` dependency.

use std::cell::Cell;
use std::rc::Rc;

use systemrs_kernel::{Ctx, Sim};

/// A seeded SplitMix64 pseudo-random generator (a `Sim` service).
///
/// Interior `Cell` so draws work through the shared `Rc<Rng>` the service map holds.
#[derive(Debug)]
pub struct Rng {
    /// The evolving generator state.
    state: Cell<u64>,

    /// The seed it was created with (for journaling / replay).
    seed: u64,
}

impl Rng {
    /// Creates a generator seeded with `seed`.
    ///
    /// # Arguments
    ///
    /// * `seed` - The starting seed (also the replay key).
    ///
    /// # Returns
    ///
    /// A new [`Rng`].
    pub fn new(seed: u64) -> Self {
        Rng {
            state: Cell::new(seed),
            seed,
        }
    }

    /// Installs a fresh `Rng` seeded with `seed` as a [`Sim`] service and returns the
    /// shared handle. Models draw via `cx.service::<Rng>()`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `seed` - The seed.
    ///
    /// # Returns
    ///
    /// The shared [`Rng`] service handle.
    pub fn install(sim: &Sim, seed: u64) -> Rc<Rng> {
        let rng = Rc::new(Rng::new(seed));
        sim.register_service(Rc::clone(&rng));
        rng
    }

    /// Returns the running simulation's `Rng` service.
    ///
    /// # Arguments
    ///
    /// * `cx` - A kernel handle.
    ///
    /// # Returns
    ///
    /// The shared [`Rng`] service handle.
    ///
    /// # Panics
    ///
    /// Panics if no `Rng` service was installed (call [`Rng::install`] at elaboration).
    pub fn from_ctx(cx: &Ctx) -> Rc<Rng> {
        cx.service::<Rng>()
    }

    /// Returns the seed this generator was created with.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Draws the next `u64` (SplitMix64).
    ///
    /// # Returns
    ///
    /// A pseudo-random 64-bit value.
    pub fn next_u64(&self) -> u64 {
        let mut z = self.state.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
        self.state.set(z);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Draws the next `u32` (the high 32 bits of [`Rng::next_u64`]).
    pub fn next_u32(&self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Draws a `f64` uniformly in `[0, 1)` (the high 53 bits).
    pub fn next_f64(&self) -> f64 {
        // 53-bit mantissa: divide a 53-bit integer by 2^53.
        ((self.next_u64() >> 11) as f64) * (1.0 / 9_007_199_254_740_992.0)
    }

    /// Draws a `u64` in `[lo, hi)`.
    ///
    /// Uses a modulo reduction (a slight low-end bias for ranges that do not divide
    /// `2^64` evenly — acceptable for twin stimulus; use [`Rng::next_f64`] scaling for
    /// bias-free reals).
    ///
    /// # Arguments
    ///
    /// * `lo` - The inclusive lower bound.
    /// * `hi` - The exclusive upper bound.
    ///
    /// # Returns
    ///
    /// A value in `[lo, hi)`.
    ///
    /// # Panics
    ///
    /// Panics if `lo >= hi`.
    pub fn gen_range(&self, lo: u64, hi: u64) -> u64 {
        assert!(lo < hi, "gen_range requires lo < hi");
        lo + self.next_u64() % (hi - lo)
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;
    use systemrs_kernel::Sim;

    /// Same seed → identical draw sequence; different seed → different sequence.
    #[test]
    fn deterministic_and_seed_dependent() {
        let a = Rng::new(0xDEAD_BEEF);
        let b = Rng::new(0xDEAD_BEEF);
        let seq_a: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let seq_b: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        assert_eq!(seq_a, seq_b, "same seed reproduces the sequence");

        let c = Rng::new(0x1234_5678);
        let seq_c: Vec<u64> = (0..8).map(|_| c.next_u64()).collect();
        assert_ne!(seq_a, seq_c, "different seed diverges");
    }

    /// `gen_range`/`next_f64` stay in bounds.
    #[test]
    fn bounded_draws() {
        let r = Rng::new(42);
        for _ in 0..1000 {
            let v = r.gen_range(10, 20);
            assert!((10..20).contains(&v));
            let f = r.next_f64();
            assert!((0.0..1.0).contains(&f));
        }
    }

    /// Installs as a service and draws via the `Ctx`.
    #[test]
    fn installs_as_service() {
        let sim = Sim::new();
        let rng = Rng::install(&sim, 7);
        assert_eq!(rng.seed(), 7);
        let from_service = sim.ctx().service::<Rng>();
        // Same underlying generator: a draw through the service advances `rng` too.
        let v1 = from_service.next_u64();
        let v2 = rng.next_u64();
        assert_ne!(v1, v2); // sequential draws from one shared generator differ
    }
}
