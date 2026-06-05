//! The kernel's view of a primitive channel: an update callback.
//!
//! Reproduces `sc_prim_channel`'s deferred-side-effect discipline
//! (`doc/systemrs-design.md` §3.6, §6c) without the kernel knowing concrete channel
//! types. A channel calls `Ctx::request_update` during the evaluate phase; the
//! kernel calls [`UpdatableChannel::update`] for each requesting channel during the
//! update phase, where the channel commits its new value and posts its
//! value-changed event for the *next* delta.

use crate::ctx::Ctx;
use core::any::Any;

/// A primitive channel that defers commits to the update phase.
///
/// Implemented with `&self` + interior mutability (`Cell`/`RefCell`) so the kernel
/// can hold channels behind shared `Rc` handles and still update them. The
/// [`UpdatableChannel::as_any`] hook lets a `Copy`/`Send` channel *handle* (an id +
/// phantom type) downcast the kernel-held state to its concrete type, realising the
/// design's "refer by id, never by reference" rule (`doc/systemrs-design.md` §6a)
/// uniformly for both methods and (Send-required) threads.
pub trait UpdatableChannel: Any {
    /// Commits this channel's staged value(s) and posts any value-changed event.
    ///
    /// Called exactly once per delta in which the channel requested an update.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle, used to post delta notifications.
    fn update(&self, ctx: &Ctx);

    /// Returns this channel as `&dyn Any` for downcasting from a typed handle.
    fn as_any(&self) -> &dyn Any;
}
