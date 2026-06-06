//! [`PeqWithGet`] — the pull-model payload event queue (`tlm_utils::peq_with_get`).
//!
//! A time-ordered queue of items, each released back to a *puller* at its scheduled
//! time. The "fire on the next delta" parity SystemC achieves with even/odd delta
//! arithmetic is obtained here for free by routing through the kernel's delta
//! notification (`Ctx::notify_after(ev, ZERO)` collapses to `notify_delta`,
//! `doc/systemrs-design.md` §3.11, §6d): two zero-delay items released at the same
//! time come out one delta apart, in insertion (FIFO) order.
//!
//! Equal-time ordering is pinned by an insertion sequence number in the key
//! (`BTreeMap<(SimTime, seq), T>`), mirroring the kernel's timed-heap `seq`
//! tie-break — so there is no `HashMap`-order or pointer-identity nondeterminism.

use std::collections::BTreeMap;

use systemrs_kernel::{Ctx, EventId, Sim};
use systemrs_time::SimTime;

/// A pull-model payload event queue of items `T` (typically `Txn` handles).
///
/// Items are *handles* — the queue does not own payload bytes (recycling stays with
/// the `TxnPool` at the initiator).
pub struct PeqWithGet<T> {
    /// Time-and-sequence-ordered pending items.
    queue: BTreeMap<(SimTime, u64), T>,

    /// Monotonic insertion sequence for equal-time FIFO ordering.
    seq: u64,

    /// The single kernel event a puller waits on; re-armed for the next due item.
    event: EventId,
}

impl<T> PeqWithGet<T> {
    /// Creates an empty PEQ with a fresh kernel event.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    ///
    /// # Returns
    ///
    /// A new [`PeqWithGet`].
    pub fn new(sim: &Sim) -> Self {
        PeqWithGet {
            queue: BTreeMap::new(),
            seq: 0,
            event: sim.alloc_event(),
        }
    }

    /// Returns the event a draining process should wait on / be sensitive to.
    pub fn event(&self) -> EventId {
        self.event
    }

    /// Returns `true` if no items are queued.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Schedules `item` to become available `after` the current time.
    ///
    /// `after == ZERO` releases it on the next delta (the delta-parity rule).
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    /// * `item` - The item to queue.
    /// * `after` - The relative release delay.
    pub fn notify(&mut self, cx: &Ctx, item: T, after: SimTime) {
        let when = cx.now() + after;
        self.queue.insert((when, self.seq), item);
        self.seq += 1;
        cx.notify_after(self.event, after);
    }

    /// Schedules `item` for release on the next delta (`after == ZERO`).
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    /// * `item` - The item to queue.
    pub fn notify_now(&mut self, cx: &Ctx, item: T) {
        self.notify(cx, item, SimTime::ZERO);
    }

    /// Pops the next item due at or before now, releasing exactly **one** per call.
    ///
    /// If more due items remain, the queue re-arms its event so the next is released
    /// one delta later (the one-per-delta shape). If the next item is in the future,
    /// the event is re-armed for that time and `None` is returned.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    ///
    /// # Returns
    ///
    /// The next due item, or `None` if none is due now.
    pub fn get_next(&mut self, cx: &Ctx) -> Option<T> {
        let key = *self.queue.keys().next()?;
        let now = cx.now();
        if key.0 > now {
            // Not due yet: re-arm for its time and report nothing due.
            cx.notify_after(self.event, key.0 - now);
            return None;
        }
        let item = self.queue.remove(&key);
        // If another item is already due, re-arm so it is released on the NEXT delta;
        // a future item re-arms for its time.
        if let Some(&next_key) = self.queue.keys().next() {
            if next_key.0 <= now {
                cx.notify(self.event); // delta -> next delta round
            } else {
                cx.notify_after(self.event, next_key.0 - now);
            }
        }
        item
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;

    use super::PeqWithGet;

    /// A `Send` record of `(item, now_units, delta_count)` per release.
    type Records = Arc<Mutex<Vec<(&'static str, u64, u64)>>>;

    /// E5: two same-time zero-delay notifications are released one delta apart, FIFO.
    #[test]
    fn two_zero_delay_notifies_fire_one_delta_apart_fifo() {
        let sim = Sim::new();
        let mut peq: PeqWithGet<&'static str> = PeqWithGet::new(&sim);
        let event = peq.event();
        let records: Records = Arc::new(Mutex::new(Vec::new()));
        let r = Arc::clone(&records);

        // The PEQ is `Send` (BTreeMap + EventId of Send data), so it is moved into
        // the single draining thread that both produces and consumes.
        sim.add_thread("drain", &[], true, move |cx| {
            peq.notify_now(cx, "A");
            peq.notify_now(cx, "B");
            let mut left = 2;
            while left > 0 {
                cx.wait_event(event);
                if let Some(item) = peq.get_next(cx) {
                    r.lock()
                        .expect("lock")
                        .push((item, cx.now().units(), cx.delta_count()));
                    left -= 1;
                }
            }
        });

        sim.run_until(SimTime::from_ns(10));

        let recs = records.lock().expect("lock");
        assert_eq!(recs.len(), 2);
        // FIFO order, both at the same time, exactly one delta apart.
        assert_eq!(recs[0].0, "A");
        assert_eq!(recs[1].0, "B");
        assert_eq!(recs[0].1, recs[1].1); // same sim time
        assert_eq!(recs[1].2, recs[0].2 + 1); // one delta apart
    }

    /// A future-timed item is released at its time, not before; equal-time items keep
    /// insertion order across many entries.
    #[test]
    fn timed_and_equal_time_ordering() {
        let sim = Sim::new();
        let mut peq: PeqWithGet<u32> = PeqWithGet::new(&sim);
        let event = peq.event();
        let out: Arc<Mutex<Vec<(u32, u64)>>> = Arc::new(Mutex::new(Vec::new()));
        let o = Arc::clone(&out);

        sim.add_thread("drain", &[], true, move |cx| {
            // Two now-items (insertion order 1,2) and one at +5 ns.
            peq.notify_now(cx, 1);
            peq.notify_now(cx, 2);
            peq.notify(cx, 3, SimTime::from_ns(5));
            let mut left = 3;
            while left > 0 {
                cx.wait_event(event);
                if let Some(v) = peq.get_next(cx) {
                    o.lock().expect("lock").push((v, cx.now().units()));
                    left -= 1;
                }
            }
        });

        sim.run_until(SimTime::from_ns(100));

        let got = out.lock().expect("lock");
        assert_eq!(got[0], (1, 0));
        assert_eq!(got[1], (2, 0));
        assert_eq!(got[2], (3, SimTime::from_ns(5).units())); // released at its time
    }
}
