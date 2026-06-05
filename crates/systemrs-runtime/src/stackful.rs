//! The corosensei-backed [`Fiber`] and the depth-callable [`suspend`].

use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

use corosensei::{Coroutine, CoroutineResult, Yielder};

/// The corosensei yielder type for a control-only coroutine (no payload either
/// direction; see the module docs).
type ControlYielder = Yielder<(), ()>;

thread_local! {
    /// Address of the yielder for the *currently running* fiber, or null on the
    /// kernel stack. Installed and restored by [`Fiber::resume`].
    static CURRENT_YIELDER: core::cell::Cell<*const ControlYielder> =
        const { core::cell::Cell::new(ptr::null()) };
}

/// The state of a [`Fiber`] after a [`Fiber::resume`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiberState {
    /// The body called [`suspend`] (e.g. via `wait()`) and can be resumed again.
    Suspended,

    /// The body returned; the fiber must not be resumed again.
    Done,
}

/// A stackful coroutine hosting one `SC_THREAD` body.
///
/// The body runs on its own stack and yields control to the kernel at each
/// [`suspend`]. Constructing a fiber does not start the body; the first
/// [`Fiber::resume`] does.
pub struct Fiber {
    /// The underlying control-only coroutine.
    coro: Coroutine<(), (), ()>,

    /// The yielder address, published by the body on first activation. Stored in
    /// an [`AtomicPtr`] purely to satisfy corosensei's `Send` bound on the body
    /// closure — it is only ever accessed from the single simulation thread.
    yielder: Arc<AtomicPtr<ControlYielder>>,

    /// Whether the body has returned.
    done: bool,
}

impl Fiber {
    /// Creates a fiber that will run `body` on first resume.
    ///
    /// # Arguments
    ///
    /// * `body` - The `SC_THREAD` body. It may call [`suspend`] from any depth.
    ///   Must be `Send + 'static` (corosensei's requirement); SystemRS thread
    ///   bodies capture only `Copy` id handles and owned data, satisfying this.
    ///
    /// # Returns
    ///
    /// A new, not-yet-started [`Fiber`].
    pub fn new<F>(body: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let yielder: Arc<AtomicPtr<ControlYielder>> = Arc::new(AtomicPtr::new(ptr::null_mut()));
        let published = Arc::clone(&yielder);

        let coro = Coroutine::new(move |y: &ControlYielder, ()| {
            // Publish the yielder address so `resume` can re-install it on every
            // subsequent activation (the closure entry runs only once).
            published.store(ptr::from_ref(y).cast_mut(), Ordering::Relaxed);
            CURRENT_YIELDER.with(|c| c.set(ptr::from_ref(y)));
            body();
        });

        Fiber {
            coro,
            yielder,
            done: false,
        }
    }

    /// Resumes the body until its next [`suspend`] or its return.
    ///
    /// # Returns
    ///
    /// [`FiberState::Suspended`] if the body yielded, or [`FiberState::Done`] if it
    /// returned.
    ///
    /// # Panics
    ///
    /// Panics if called after the body has already returned.
    pub fn resume(&mut self) -> FiberState {
        assert!(!self.done, "resume() called on a finished fiber");

        // Install this fiber's yielder as current, saving the previous one so
        // nested resumes (kernel stack → fiber) restore correctly.
        let mine = self.yielder.load(Ordering::Relaxed);
        let prev = CURRENT_YIELDER.with(|c| {
            let prev = c.get();
            if !mine.is_null() {
                c.set(mine.cast_const());
            }
            prev
        });

        let result = self.coro.resume(());

        CURRENT_YIELDER.with(|c| c.set(prev));

        match result {
            CoroutineResult::Yield(()) => FiberState::Suspended,
            CoroutineResult::Return(()) => {
                self.done = true;
                FiberState::Done
            }
        }
    }

    /// Returns `true` if the body has returned.
    pub fn is_done(&self) -> bool {
        self.done
    }
}

/// Suspends the currently running fiber, returning control to the kernel's resume
/// site.
///
/// This is the primitive behind every `wait()`. It is callable from any call
/// depth inside an `SC_THREAD` body.
///
/// # Panics
///
/// Panics if called outside a running fiber (i.e. on the kernel stack).
pub fn suspend() {
    let y = CURRENT_YIELDER.with(core::cell::Cell::get);
    assert!(
        !y.is_null(),
        "suspend() (wait) called outside an SC_THREAD body"
    );

    // SAFETY: `y` is the address of the yielder for the currently running fiber.
    // `Fiber::resume` installs it before resuming and restores the previous value
    // after, and a fiber can only reach this code while it is the current fiber,
    // so the yielder is alive and uniquely ours for the duration of this call.
    let yielder: &ControlYielder = unsafe { &*y };
    yielder.suspend(());
}

#[cfg(test)]
mod tests {
    use super::{Fiber, FiberState, suspend};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    // Thread bodies must be `Send` (corosensei's requirement); the tests observe
    // progress through `Arc<AtomicU32>` rather than `Rc<Cell>`, mirroring how real
    // SystemRS thread bodies stay `Send` by capturing only owned/atomic state and
    // reaching the kernel via a thread-local (never by capture).

    /// Verifies a fiber suspends and resumes, and that `suspend` reaches the
    /// yielder from a nested helper call (the depth-callable property).
    #[test]
    fn suspend_from_depth_and_resume() {
        let log = Arc::new(AtomicU32::new(0));
        let l = Arc::clone(&log);

        fn deep_wait() {
            // Three calls deep, exactly like wait() inside b_transport's callees.
            fn helper() {
                suspend();
            }
            helper();
        }

        let mut fiber = Fiber::new(move || {
            l.store(1, Ordering::Relaxed);
            deep_wait();
            l.store(2, Ordering::Relaxed);
            deep_wait();
            l.store(3, Ordering::Relaxed);
        });

        assert_eq!(log.load(Ordering::Relaxed), 0);
        assert_eq!(fiber.resume(), FiberState::Suspended);
        assert_eq!(log.load(Ordering::Relaxed), 1);
        assert_eq!(fiber.resume(), FiberState::Suspended);
        assert_eq!(log.load(Ordering::Relaxed), 2);
        assert_eq!(fiber.resume(), FiberState::Done);
        assert_eq!(log.load(Ordering::Relaxed), 3);
    }

    /// Verifies two interleaved fibers each re-install their own yielder, so a
    /// stale thread-local can never misroute a `suspend`.
    #[test]
    fn interleaved_fibers_keep_distinct_yielders() {
        let a_steps = Arc::new(AtomicU32::new(0));
        let b_steps = Arc::new(AtomicU32::new(0));
        let (a, b) = (Arc::clone(&a_steps), Arc::clone(&b_steps));

        let mut fa = Fiber::new(move || {
            for i in 1..=3 {
                a.store(i, Ordering::Relaxed);
                suspend();
            }
        });
        let mut fb = Fiber::new(move || {
            for i in 1..=3 {
                b.store(i * 10, Ordering::Relaxed);
                suspend();
            }
        });

        let snap = || {
            (
                a_steps.load(Ordering::Relaxed),
                b_steps.load(Ordering::Relaxed),
            )
        };

        // Interleave: A, B, A, B, ...
        assert_eq!(fa.resume(), FiberState::Suspended);
        assert_eq!(fb.resume(), FiberState::Suspended);
        assert_eq!(snap(), (1, 10));
        assert_eq!(fa.resume(), FiberState::Suspended);
        assert_eq!(fb.resume(), FiberState::Suspended);
        assert_eq!(snap(), (2, 20));
        assert_eq!(fa.resume(), FiberState::Suspended);
        assert_eq!(fb.resume(), FiberState::Suspended);
        assert_eq!(snap(), (3, 30));
    }

    /// Verifies a suspended fiber can be dropped without resuming to completion
    /// (corosensei force-unwinds the stack, running destructors).
    #[test]
    fn drop_suspended_fiber() {
        let mut fiber = Fiber::new(|| {
            let _guard = String::from("local across wait");
            suspend();
            unreachable!("not resumed");
        });
        assert_eq!(fiber.resume(), FiberState::Suspended);
        drop(fiber);
    }
}
