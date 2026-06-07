//! [`JournalReplayer`] — deterministic replay of a recorded [`Journal`]
//! (`doc/systemrs-design.md` §6f, §8).
//!
//! Replays injections with **no live producer thread**: a dedicated replay-driver
//! `SC_THREAD` walks the journal in order, waiting until each record's sim time, then
//! injecting its value through the same injector the live run used. The driver itself
//! advances the clock (a real waiting process, not a tombstone event), so replay is
//! self-driving. With the default exit-on-starvation policy the run ends once the
//! journal is exhausted. Combined with restoring the journal's RNG seed, the run
//! reproduces a byte-identical transaction trace.

use systemrs_kernel::{Ctx, Sim};

use crate::input::SendInjector;
use crate::journal::Journal;

/// Replays a [`Journal`] into a fresh simulation with no external thread.
pub struct JournalReplayer {
    /// The recorded run.
    journal: Journal,

    /// The injector mapping each recorded value to activity (must match the live
    /// run). `Send` because the replay-driver runs as a spawned coroutine.
    injector: SendInjector<u64>,
}

impl JournalReplayer {
    /// Creates a replayer over `journal`, injecting via `injector`.
    ///
    /// # Arguments
    ///
    /// * `journal` - The recorded run (carries the seed and the injections).
    /// * `injector` - The same value→activity mapping the live run used.
    ///
    /// # Returns
    ///
    /// A new [`JournalReplayer`].
    pub fn new<F>(journal: Journal, injector: F) -> Self
    where
        F: Fn(&Ctx, u64) + Send + 'static,
    {
        JournalReplayer {
            journal,
            injector: Box::new(injector),
        }
    }

    /// Returns the recorded RNG seed (restore it into [`crate::Rng::install`] before
    /// running, or replay will diverge).
    pub fn seed(&self) -> u64 {
        self.journal.seed
    }

    /// Installs the replay-driver process. The simulation keeps the default
    /// exit-on-starvation policy, so it ends when the journal is exhausted.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    pub fn install(self, sim: &Sim) {
        let records = self.journal.records;
        let injector = self.injector;
        sim.add_thread("replay-driver", &[], true, move |cx| {
            for rec in &records {
                let now = cx.now();
                if rec.at > now {
                    cx.wait(rec.at - now); // advance to the record's sim time
                }
                injector(cx, rec.value); // same injection path as the live run
            }
        });
    }
}
