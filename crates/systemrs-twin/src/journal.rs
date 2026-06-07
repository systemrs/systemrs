//! Input journal: record external injections for deterministic replay
//! (`doc/systemrs-design.md` §6f, §8).
//!
//! A [`JournalRecorder`] decorates a `u64`-valued external input: it drains the inbox
//! like a [`crate::ChannelInput`], but tees each injection — tagged with the sim time
//! and delta it landed at — into a [`Journal`] alongside the run's RNG seed. The
//! journal serializes to plain text (no serde) and is replayed by
//! [`crate::JournalReplayer`] with no live producer thread, reproducing a
//! byte-identical transaction trace.
//!
//! M6 journals `u64`-shaped inputs (the journal value column is a `u64`); richer
//! payload types are a deferred follow-up.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Receiver;

use systemrs_kernel::Ctx;
use systemrs_time::SimTime;

use crate::input::{ChannelInputSender, ExternalInput, Injector, StopSignal, new_channel};

/// Whether an injection used a delta or a timed notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionKind {
    /// A same-time (delta) injection.
    Delta,

    /// A future (timed) injection.
    Timed,
}

impl InjectionKind {
    /// The text tag used in serialized journals.
    fn tag(self) -> &'static str {
        match self {
            InjectionKind::Delta => "D",
            InjectionKind::Timed => "T",
        }
    }
}

/// One recorded injection: the value, and the `(time, delta)` it landed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectionRecord {
    /// The simulation time of the injection.
    pub at: SimTime,

    /// The delta count at the injection.
    pub delta: u64,

    /// The injection kind.
    pub kind: InjectionKind,

    /// The injected value.
    pub value: u64,
}

/// A recorded run: the RNG seed plus the ordered injections.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Journal {
    /// The RNG seed the run used (load-bearing for replay).
    pub seed: u64,

    /// The injections, in occurrence order.
    pub records: Vec<InjectionRecord>,
}

impl Journal {
    /// Creates an empty journal for `seed`.
    ///
    /// # Arguments
    ///
    /// * `seed` - The run's RNG seed.
    ///
    /// # Returns
    ///
    /// A new [`Journal`].
    pub fn new(seed: u64) -> Self {
        Journal {
            seed,
            records: Vec::new(),
        }
    }

    /// Serializes the journal to stable text (one header line + one line per record).
    ///
    /// # Returns
    ///
    /// The text form (`seed <n>` then `<at> <delta> <kind> <value>` lines).
    pub fn serialize(&self) -> String {
        use core::fmt::Write as _;
        let mut out = format!("seed {}\n", self.seed);
        for r in &self.records {
            let _ = writeln!(
                out,
                "{} {} {} {}",
                r.at.units(),
                r.delta,
                r.kind.tag(),
                r.value
            );
        }
        out
    }

    /// Parses a journal from its text form.
    ///
    /// # Arguments
    ///
    /// * `text` - The serialized journal.
    ///
    /// # Returns
    ///
    /// The parsed [`Journal`], or `None` on malformed input.
    pub fn parse(text: &str) -> Option<Journal> {
        let mut lines = text.lines();
        let seed = lines.next()?.strip_prefix("seed ")?.parse().ok()?;
        let mut records = Vec::new();
        for line in lines.filter(|l| !l.trim().is_empty()) {
            let mut f = line.split_whitespace();
            let at = SimTime::from_units(f.next()?.parse().ok()?);
            let delta = f.next()?.parse().ok()?;
            let kind = match f.next()? {
                "D" => InjectionKind::Delta,
                "T" => InjectionKind::Timed,
                _ => return None,
            };
            let value = f.next()?.parse().ok()?;
            records.push(InjectionRecord {
                at,
                delta,
                kind,
                value,
            });
        }
        Some(Journal { seed, records })
    }
}

/// A recording external input: drains a `u64` inbox, injecting each value AND teeing
/// it into a shared [`Journal`].
pub struct JournalRecorder {
    /// The channel receive side.
    rx: Receiver<u64>,

    /// Turns one value into injected activity (same shape as a `ChannelInput`).
    injector: Injector<u64>,

    /// The journal being recorded into.
    journal: Rc<RefCell<Journal>>,
}

impl ExternalInput for JournalRecorder {
    fn poll(&mut self, cx: &Ctx) -> bool {
        let mut injected = false;
        while let Ok(value) = self.rx.try_recv() {
            self.journal.borrow_mut().records.push(InjectionRecord {
                at: cx.now(),
                delta: cx.delta_count(),
                kind: InjectionKind::Delta,
                value,
            });
            (self.injector)(cx, value);
            injected = true;
        }
        injected
    }
}

/// Creates a recording `u64` external input + its sender/stop + the shared journal.
///
/// The journal is seeded with `seed` (record the same seed into the RNG with
/// [`crate::Rng::install`]); read it back after the run via the returned handle.
///
/// # Arguments
///
/// * `seed` - The run's RNG seed (stored in the journal).
/// * `injector` - Maps each received value to injected activity.
///
/// # Returns
///
/// `(recorder, sender, stop, journal)`.
#[allow(clippy::type_complexity)] // a constructor returning its four coupled handles
pub fn journal_input<F>(
    seed: u64,
    injector: F,
) -> (
    JournalRecorder,
    ChannelInputSender<u64>,
    StopSignal,
    Rc<RefCell<Journal>>,
)
where
    F: Fn(&Ctx, u64) + 'static,
{
    let (rx, sender, stop) = new_channel::<u64>();
    let journal = Rc::new(RefCell::new(Journal::new(seed)));
    let recorder = JournalRecorder {
        rx,
        injector: Box::new(injector),
        journal: Rc::clone(&journal),
    };
    (recorder, sender, stop, journal)
}

#[cfg(test)]
mod tests {
    use super::{InjectionKind, InjectionRecord, Journal};
    use systemrs_time::SimTime;

    /// A journal round-trips through its text form.
    #[test]
    fn journal_text_round_trips() {
        let mut j = Journal::new(0xABCD);
        j.records.push(InjectionRecord {
            at: SimTime::from_ns(5),
            delta: 2,
            kind: InjectionKind::Delta,
            value: 42,
        });
        j.records.push(InjectionRecord {
            at: SimTime::from_ns(12),
            delta: 0,
            kind: InjectionKind::Timed,
            value: 7,
        });
        let text = j.serialize();
        let parsed = Journal::parse(&text).expect("parse");
        assert_eq!(parsed, j);
    }
}
