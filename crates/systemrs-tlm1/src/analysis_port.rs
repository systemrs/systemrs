//! [`AnalysisPort`] — the synchronous fan-out telemetry broadcast (`sc`/`tlm`
//! `tlm_analysis_port` + `tlm_write_if`, `doc/systemrs-design.md` §3.7, §6e).
//!
//! `write()` delivers to every bound subscriber **synchronously, immediately, in
//! registration order, with no back-pressure** — the non-intrusive observability
//! mechanism a digital twin needs. Subscribers are held as `Weak` (the port does not
//! own them); dead ones are reaped. `write()` snapshots the live subscriber set
//! before delivering, so a subscriber may legally bind/unbind (or write) during
//! delivery without a `RefCell` double-borrow.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// The write side of the analysis interface (`tlm_write_if`): receive one value.
pub trait AnalysisWrite<T> {
    /// Receives one broadcast value (synchronous, in-order, must not block).
    ///
    /// # Arguments
    ///
    /// * `txn` - The broadcast value.
    fn write(&self, txn: &T);
}

/// A one-to-many synchronous broadcast port.
///
/// # Examples
///
/// One `write` fans out to every bound subscriber, in registration order:
///
/// ```
/// use systemrs_tlm1::{AnalysisPort, AnalysisWrite};
/// use std::cell::RefCell;
/// use std::rc::Rc;
///
/// struct Sink(Rc<RefCell<Vec<i32>>>);
/// impl AnalysisWrite<i32> for Sink {
///     fn write(&self, v: &i32) {
///         self.0.borrow_mut().push(*v);
///     }
/// }
///
/// let log = Rc::new(RefCell::new(Vec::new()));
/// let port: AnalysisPort<i32> = AnalysisPort::new();
/// let a = Rc::new(Sink(Rc::clone(&log)));
/// let b = Rc::new(Sink(Rc::clone(&log)));
/// port.bind(&a); // subscribers are held weakly; the caller keeps `a`/`b` alive
/// port.bind(&b);
///
/// port.write(&42); // synchronous, in-order, no back-pressure
/// assert_eq!(*log.borrow(), vec![42, 42]);
/// ```
pub struct AnalysisPort<T> {
    /// Bound subscribers, held weakly in registration order.
    subs: RefCell<Vec<Weak<dyn AnalysisWrite<T>>>>,
}

impl<T: 'static> AnalysisPort<T> {
    /// Creates an analysis port with no subscribers.
    ///
    /// # Returns
    ///
    /// A new [`AnalysisPort`].
    pub fn new() -> Self {
        AnalysisPort {
            subs: RefCell::new(Vec::new()),
        }
    }

    /// Binds a subscriber (held weakly; the caller retains ownership).
    ///
    /// # Arguments
    ///
    /// * `sub` - The subscriber to broadcast to.
    pub fn bind<S: AnalysisWrite<T> + 'static>(&self, sub: &Rc<S>) {
        self.subs
            .borrow_mut()
            .push(Rc::downgrade(sub) as Weak<dyn AnalysisWrite<T>>);
    }

    /// Returns the number of currently-live subscribers (reaping dead ones).
    pub fn num_subscribers(&self) -> usize {
        let mut subs = self.subs.borrow_mut();
        subs.retain(|w| w.strong_count() > 0);
        subs.len()
    }

    /// Broadcasts `txn` to every live subscriber, synchronously and in registration
    /// order.
    ///
    /// Snapshots the live subscribers (upgrading and reaping dead `Weak`s under a
    /// single brief borrow) before delivering, so a subscriber may re-enter
    /// `bind`/`write` on this port during delivery without panicking.
    ///
    /// # Arguments
    ///
    /// * `txn` - The value to broadcast.
    pub fn write(&self, txn: &T) {
        let live: Vec<Rc<dyn AnalysisWrite<T>>> = {
            let mut subs = self.subs.borrow_mut();
            subs.retain(|w| w.strong_count() > 0);
            subs.iter().filter_map(Weak::upgrade).collect()
        };
        for sub in live {
            sub.write(txn);
        }
    }
}

impl<T: 'static> Default for AnalysisPort<T> {
    fn default() -> Self {
        AnalysisPort::new()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{AnalysisPort, AnalysisWrite};

    /// A subscriber that records `(tag, value)` it receives.
    struct Recorder {
        tag: u32,
        log: Rc<RefCell<Vec<(u32, i32)>>>,
    }

    impl AnalysisWrite<i32> for Recorder {
        fn write(&self, txn: &i32) {
            self.log.borrow_mut().push((self.tag, *txn));
        }
    }

    /// EC1: a write reaches every subscriber synchronously, in registration order.
    #[test]
    fn fan_out_in_registration_order() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let port: AnalysisPort<i32> = AnalysisPort::new();
        let subs: Vec<Rc<Recorder>> = (0..3)
            .map(|tag| {
                let s = Rc::new(Recorder {
                    tag,
                    log: Rc::clone(&log),
                });
                port.bind(&s);
                s
            })
            .collect();

        port.write(&42);
        assert_eq!(*log.borrow(), vec![(0, 42), (1, 42), (2, 42)]);
        assert_eq!(port.num_subscribers(), 3);
        drop(subs);
    }

    /// A dropped subscriber is skipped (reaped), not delivered to.
    #[test]
    fn dropped_subscriber_skipped() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let port: AnalysisPort<i32> = AnalysisPort::new();
        let keep = Rc::new(Recorder {
            tag: 0,
            log: Rc::clone(&log),
        });
        port.bind(&keep);
        {
            let temp = Rc::new(Recorder {
                tag: 9,
                log: Rc::clone(&log),
            });
            port.bind(&temp);
        } // temp dropped here
        port.write(&7);
        assert_eq!(*log.borrow(), vec![(0, 7)]); // tag 9 reaped
        assert_eq!(port.num_subscribers(), 1);
    }

    /// A subscriber that binds another subscriber during `write()` does not panic
    /// (snapshot-then-iterate re-entrancy safety).
    #[test]
    fn reentrant_bind_during_write_is_safe() {
        let log = Rc::new(RefCell::new(Vec::new()));

        /// On its first write, binds a fresh recorder to the same port.
        struct Binder {
            log: Rc<RefCell<Vec<(u32, i32)>>>,
            port: Rc<AnalysisPort<i32>>,
            extra: RefCell<Option<Rc<Recorder>>>,
        }
        impl AnalysisWrite<i32> for Binder {
            fn write(&self, txn: &i32) {
                self.log.borrow_mut().push((100, *txn));
                if self.extra.borrow().is_none() {
                    let r = Rc::new(Recorder {
                        tag: 200,
                        log: Rc::clone(&self.log),
                    });
                    self.port.bind(&r); // re-entrant bind during fan-out
                    *self.extra.borrow_mut() = Some(r);
                }
            }
        }

        let port = Rc::new(AnalysisPort::<i32>::new());
        let binder = Rc::new(Binder {
            log: Rc::clone(&log),
            port: Rc::clone(&port),
            extra: RefCell::new(None),
        });
        port.bind(&binder);

        port.write(&1); // binder fires + binds the extra (no panic)
        port.write(&2); // both fire
        assert_eq!(*log.borrow(), vec![(100, 1), (100, 2), (200, 2)]);
    }
}
