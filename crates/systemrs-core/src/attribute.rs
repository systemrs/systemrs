//! Per-object attribute storage (`sc_attribute<T>`).
//!
//! A lazily-allocated, `TypeId`-keyed bag of arbitrary values attached to an
//! [`crate::ObjectMeta`] (`doc/systemrs-design.md` §6b). This mirrors SystemC's
//! attribute mechanism in idiomatic Rust: a `HashMap<TypeId, Box<dyn Any>>` rather
//! than a name-keyed `sc_attribute` list.
//!
//! In Milestone 2 the *type* is defined here so [`crate::ObjectMeta`] can name it;
//! the `get`/`set` bodies are filled in by M2-12 (the feature is not load-bearing
//! for the elaboration barrier).

use std::any::Any;
use std::any::TypeId;
use std::collections::HashMap;

/// A lazily-allocated store of typed attributes for one object.
///
/// The backing map is not allocated until the first attribute is set, so objects
/// that carry no attributes (the common case) cost nothing beyond an `Option`.
#[derive(Default)]
pub struct AttributeStore {
    /// The `TypeId`-keyed attribute values, allocated on first use.
    map: Option<HashMap<TypeId, Box<dyn Any>>>,
}

impl AttributeStore {
    /// Creates an empty attribute store (no backing allocation yet).
    ///
    /// # Returns
    ///
    /// An empty [`AttributeStore`].
    pub fn new() -> Self {
        Self::default()
    }
}
