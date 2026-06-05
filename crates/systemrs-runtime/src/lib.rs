//! Stackful coroutine backend for SystemRS `SC_THREAD` processes.
//!
//! This crate realises the design's central technical bet (`doc/systemrs-design.md`
//! §6a): an `SC_THREAD` is a **stackful coroutine**, so [`suspend`] (the engine
//! behind `wait()`) is an ordinary synchronous call reachable from *any* call
//! depth — deep inside `b_transport`, helpers, library code — without "async
//! colouring" spreading across the TLM forward path.
//!
//! The mechanism is a thin wrapper over [`corosensei`]. The only communication
//! through the coroutine boundary is *control*: the body suspends with `()` and
//! resumes with `()`. All wake-reason/wait-request data flows through the kernel's
//! shared state, not through the coroutine resume value (`doc/systemrs-design.md`
//! §6a "all communication happens through the shared `Inner`"). This keeps the
//! runtime crate fully decoupled from kernel id types.
//!
//! ## Safety
//!
//! This is one of the two crates allowed `unsafe` (the other being `-ffi`), per the
//! design's lint policy. The single use is publishing the [`corosensei::Yielder`]
//! address so [`suspend`] can reach it from arbitrary depth; every block carries a
//! `// SAFETY:` justification.
#![allow(unsafe_code)]

mod stackful;

pub use stackful::{Fiber, FiberState, suspend};
