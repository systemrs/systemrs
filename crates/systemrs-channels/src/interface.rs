//! The [`Interface`] marker trait (`sc_interface`).
//!
//! In SystemC every channel exposes one or more *interfaces*, and a port is
//! parameterised by the interface it requires. In Milestone 2 binding resolves to
//! [`systemrs_kernel::ObjectId`]s rather than typed interface pointers, so the
//! interface type parameter on [`crate::Port`]/[`crate::Export`] is a compile-time
//! tag and this trait is a marker. Typed interface dispatch (a port forwarding a
//! method call to its bound channel) lands with the channel interfaces in M3.

/// Marker for a channel interface type.
///
/// Implemented by the interface types a channel provides; the first interface
/// bound to a port is its canonical interface (`doc/systemrs-design.md` §3.5).
pub trait Interface: 'static {}
