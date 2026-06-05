//! Generic-payload extensions: a `TypeId`-keyed map replacing SystemC's RTTI.

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// A generic-payload extension.
///
/// Replaces SystemC's `tlm_extension<T>` + RTTI with a plain trait object keyed by
/// [`TypeId`] (`doc/systemrs-design.md` §6d). `clone_ext` returns `None` to mean
/// "not clonable" (no null-pointer convention).
pub trait Extension: Any {
    /// Returns a boxed clone, or `None` if this extension is not clonable.
    fn clone_ext(&self) -> Option<Box<dyn Extension>>;

    /// Returns this extension as `&dyn Any` for downcasting.
    fn as_any(&self) -> &dyn Any;
}

/// A type-keyed map of extensions carried by a [`crate::GenericPayload`].
///
/// Collapses SystemC's three removal semantics onto Rust ownership: `set` (the map
/// owns and `Drop` frees), `take` (returns ownership), and `clear` (free all).
#[derive(Default)]
pub struct ExtensionMap {
    /// The extensions, keyed by concrete type id.
    map: HashMap<TypeId, Box<dyn Extension>>,
}

impl ExtensionMap {
    /// Inserts (or replaces) an extension of type `T`; the map owns it.
    ///
    /// # Arguments
    ///
    /// * `ext` - The extension to store.
    pub fn set<T: Extension>(&mut self, ext: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(ext));
    }

    /// Returns a reference to the extension of type `T`, if present.
    pub fn get<T: Extension>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|b| b.as_any().downcast_ref::<T>())
    }

    /// Removes and returns the extension of type `T`, if present.
    pub fn take<T: Extension>(&mut self) -> Option<Box<dyn Extension>> {
        self.map.remove(&TypeId::of::<T>())
    }

    /// Returns `true` if an extension of type `T` is present.
    pub fn contains<T: Extension>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    /// Removes all extensions (`free_all`).
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Returns the number of extensions present.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns `true` if no extensions are present.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl Clone for ExtensionMap {
    fn clone(&self) -> Self {
        let mut map = HashMap::new();
        for (&k, v) in &self.map {
            if let Some(cloned) = v.clone_ext() {
                map.insert(k, cloned);
            }
        }
        ExtensionMap { map }
    }
}

impl core::fmt::Debug for ExtensionMap {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExtensionMap")
            .field("len", &self.map.len())
            .finish()
    }
}

impl PartialEq for ExtensionMap {
    /// Two extension maps compare equal iff they carry the same set of extension
    /// *types* (extension contents are opaque trait objects).
    fn eq(&self, other: &Self) -> bool {
        self.map.len() == other.map.len() && self.map.keys().all(|k| other.map.contains_key(k))
    }
}

impl Eq for ExtensionMap {}
