//! External input + suspend-on-starvation gating (`doc/systemrs-design.md` §6f).
//!
//! The critical twin feature: an externally driven model must **park** (not exit)
//! when idle and resume when input arrives. An [`ExternalInput`] is drained on the
//! sim thread at starvation; if it injects activity the run resumes, otherwise the
//! sim blocks on a [`StopSignal`] condvar until a [`ChannelInputSender`] wakes it or
//! a shutdown is requested.
//!
//! The single-threaded core stays `!Send`: the [`ExternalInput`] and its `Receiver`
//! live on the sim thread; only the `Send` [`ChannelInputSender`] and [`StopSignal`]
//! cross to a producer thread. The condvar in `StopSignal` is the second (and last)
//! sanctioned cross-thread primitive alongside the mpsc channel — justified because a
//! parked single-threaded sim with no timed wake needs an OS-level wakeup + clean
//! shutdown.

use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use systemrs_kernel::{Ctx, GateOutcome, Sim, Starvation};

/// How long the parked sim sleeps between re-polls if no wake arrives — a safety
/// net; normal wakeups come promptly from [`ChannelInputSender::send`]/[`StopSignal`].
const PARK_TIMEOUT: Duration = Duration::from_millis(50);

/// A boxed value→activity injector, run on the sim thread.
pub(crate) type Injector<T> = Box<dyn Fn(&Ctx, T)>;

/// A `Send` value→activity injector, for an injector that runs inside a spawned
/// coroutine (the replay driver).
pub(crate) type SendInjector<T> = Box<dyn Fn(&Ctx, T) + Send>;

/// A source of activity injected into a running simulation from outside.
///
/// `poll` runs **on the sim thread** at a starvation point. It may inject activity
/// only via delta/timed notifications (`cx.notify`/`cx.notify_after`) — never
/// `notify_now`, which asserts the evaluate phase. It is not `Send`: it never crosses
/// threads (only the [`ChannelInputSender`] does).
pub trait ExternalInput {
    /// Drains any pending input, injecting activity.
    ///
    /// # Arguments
    ///
    /// * `cx` - A kernel handle (sim thread).
    ///
    /// # Returns
    ///
    /// `true` if it injected at least one unit of activity.
    fn poll(&mut self, cx: &Ctx) -> bool;
}

/// A cross-thread stop + wake signal (the second sanctioned `Send` primitive).
///
/// Wraps a `Mutex<bool>` flag and a `Condvar`. A producer thread calls
/// [`StopSignal::stop`] to end the run, or `send` notifies the condvar to wake the
/// parked sim to re-poll.
#[derive(Clone)]
pub struct StopSignal {
    /// `(stopped flag, wake condvar)`.
    inner: Arc<(Mutex<bool>, Condvar)>,
}

impl StopSignal {
    /// Creates a fresh, un-stopped signal.
    ///
    /// # Returns
    ///
    /// A new [`StopSignal`].
    pub fn new() -> Self {
        StopSignal {
            inner: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    /// Requests shutdown: sets the flag and wakes the parked sim.
    pub fn stop(&self) {
        let (lock, cvar) = &*self.inner;
        if let Ok(mut stopped) = lock.lock() {
            *stopped = true;
        }
        cvar.notify_all();
    }

    /// Returns whether shutdown has been requested.
    pub fn is_stopped(&self) -> bool {
        let (lock, _) = &*self.inner;
        lock.lock().map_or(true, |g| *g)
    }

    /// Wakes a parked waiter without requesting shutdown (used after a send).
    fn wake(&self) {
        self.inner.1.notify_all();
    }

    /// Parks until woken, stopped, or the safety timeout elapses.
    fn park(&self) {
        let (lock, cvar) = &*self.inner;
        if let Ok(guard) = lock.lock()
            && !*guard
        {
            let _ = cvar.wait_timeout(guard, PARK_TIMEOUT);
        }
    }
}

impl Default for StopSignal {
    fn default() -> Self {
        StopSignal::new()
    }
}

/// The send side of a [`ChannelInput`]: hand a value to the sim from another thread.
///
/// `Send`; cloning yields more producers. Each `send` notifies the parked sim.
pub struct ChannelInputSender<T> {
    /// The channel send side.
    tx: Sender<T>,

    /// Wakes the parked sim after a send.
    wake: StopSignal,
}

impl<T> Clone for ChannelInputSender<T> {
    fn clone(&self) -> Self {
        ChannelInputSender {
            tx: self.tx.clone(),
            wake: self.wake.clone(),
        }
    }
}

impl<T> ChannelInputSender<T> {
    /// Sends `value` to the simulation and wakes it if parked.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to inject.
    ///
    /// # Returns
    ///
    /// `Ok(())`, or `Err(value)` if the simulation's receiver has been dropped.
    pub fn send(&self, value: T) -> Result<(), T> {
        self.tx.send(value).map_err(|e| e.0)?;
        self.wake.wake();
        Ok(())
    }
}

/// The receive side of an mpsc inbox, with an injector that turns each value into
/// simulation activity. Lives on the sim thread (`!Send`).
pub struct ChannelInput<T> {
    /// The channel receive side.
    rx: Receiver<T>,

    /// Turns one received value into injected activity.
    injector: Injector<T>,
}

impl<T> ExternalInput for ChannelInput<T> {
    fn poll(&mut self, cx: &Ctx) -> bool {
        let mut injected = false;
        while let Ok(value) = self.rx.try_recv() {
            (self.injector)(cx, value);
            injected = true;
        }
        injected
    }
}

/// Creates an mpsc external-input channel: the sim-side [`ChannelInput`], a `Send`
/// [`ChannelInputSender`], and the shared [`StopSignal`].
///
/// # Arguments
///
/// * `injector` - Maps each received value to injected activity (delta/timed notify).
///
/// # Returns
///
/// `(input, sender, stop)`.
#[allow(clippy::type_complexity)] // a constructor returning its three coupled halves
pub fn channel_input<T, F>(injector: F) -> (ChannelInput<T>, ChannelInputSender<T>, StopSignal)
where
    F: Fn(&Ctx, T) + 'static,
{
    let (rx, sender, stop) = new_channel::<T>();
    let input = ChannelInput {
        rx,
        injector: Box::new(injector),
    };
    (input, sender, stop)
}

/// Creates the raw channel halves: the sim-side `Receiver`, a `Send`
/// [`ChannelInputSender`], and the shared [`StopSignal`]. The caller wraps the
/// `Receiver` in whatever drains it (a [`ChannelInput`] or a recording input).
#[allow(clippy::type_complexity)] // the three coupled channel halves
pub(crate) fn new_channel<T>() -> (Receiver<T>, ChannelInputSender<T>, StopSignal) {
    let (tx, rx) = mpsc::channel();
    let stop = StopSignal::new();
    let sender = ChannelInputSender {
        tx,
        wake: stop.clone(),
    };
    (rx, sender, stop)
}

/// Attaches `input` as a suspend-on-starvation source: the run parks (does not exit)
/// when idle and resumes on injection, terminating cleanly when `stop` is signalled.
///
/// Sets the [`Starvation::SuspendOnStarvation`] policy and installs a starvation gate
/// that drains `input`, returning [`GateOutcome::Resume`] on injection or parking on
/// the `stop` condvar until woken or stopped. With nothing attached the kernel keeps
/// its default starvation-EXIT.
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
/// * `input` - The external input to drain at starvation.
/// * `stop` - The shutdown/wake signal (clone the producer's copy).
pub fn attach_external_input(sim: &Sim, input: impl ExternalInput + 'static, stop: StopSignal) {
    sim.set_starvation_policy(Starvation::SuspendOnStarvation);
    let input = RefCell::new(input);
    sim.set_starvation_gate(move |cx| {
        loop {
            // Drain available input first (catches already-queued values).
            if input.borrow_mut().poll(cx) {
                return GateOutcome::Resume;
            }
            // Nothing pending: stop if asked, else park for a wake.
            if stop.is_stopped() {
                return GateOutcome::Stop;
            }
            stop.park(); // blocks the sim thread (no Inner borrow held)
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{ChannelInputSender, StopSignal};

    /// The cross-thread handles are `Send`; the sim-side input is not required to be.
    #[test]
    fn cross_thread_handles_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<ChannelInputSender<i32>>();
        assert_send::<StopSignal>();
    }
}
