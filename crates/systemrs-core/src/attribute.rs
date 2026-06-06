//! Per-object attribute storage (`sc_attribute<T>`).
//!
//! A lazily-allocated, `TypeId`-keyed bag of arbitrary values attached to an
//! [`crate::ObjectMeta`] (`doc/systemrs-design.md` §6b). This mirrors SystemC's
//! attribute mechanism in idiomatic Rust: a `HashMap<TypeId, Box<dyn Any>>` rather
//! than a name-keyed `sc_attribute` list.
//!
//! At most one value per concrete type is stored (keyed by [`TypeId`]); the feature
//! is not load-bearing for the elaboration barrier.

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

    /// Stores `value`, replacing any previous attribute of the same type `T`.
    ///
    /// Allocates the backing map on first use.
    ///
    /// # Arguments
    ///
    /// * `value` - The attribute value (one per concrete type `T`).
    pub fn set<T: Any>(&mut self, value: T) {
        self.map
            .get_or_insert_with(HashMap::new)
            .insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Returns a reference to the stored attribute of type `T`, if any.
    ///
    /// # Returns
    ///
    /// The stored `&T`, or `None` if no attribute of type `T` is set.
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.map
            .as_ref()?
            .get(&TypeId::of::<T>())?
            .downcast_ref::<T>()
    }
}

#[cfg(test)]
mod tests {
    use super::AttributeStore;

    /// Stays unallocated until first set; then round-trips two distinct types.
    #[test]
    fn set_get_round_trips() {
        let mut a = AttributeStore::new();
        assert!(a.map.is_none()); // no allocation until first set
        assert_eq!(a.get::<u32>(), None);

        a.set(7u32);
        a.set("name");
        assert_eq!(a.get::<u32>(), Some(&7));
        assert_eq!(a.get::<&str>(), Some(&"name"));
        assert_eq!(a.get::<i64>(), None);

        a.set(9u32); // replaces
        assert_eq!(a.get::<u32>(), Some(&9));
    }
}
