//! Hierarchical-name helpers: sanitisation and dot-joining.
//!
//! SystemC composes an object's full hierarchical name by joining its parent's
//! full name and its local name with `.` (`sc_object.cpp`); the `.` separator is
//! therefore reserved. These helpers centralise that rule so [`crate::ObjectStore`]
//! and any future name-aware code agree on it (`doc/systemrs-design.md` §6b).

/// The hierarchy name separator (`.`), matching SystemC's `SC_HIERARCHY_CHAR`.
pub(crate) const SEPARATOR: char = '.';

/// Replaces the reserved separator in a local name so it cannot corrupt the
/// hierarchical path.
///
/// # Arguments
///
/// * `local` - The caller-supplied local (base) name.
///
/// # Returns
///
/// The local name with every `.` replaced by `_`.
pub(crate) fn sanitize(local: &str) -> String {
    local.replace(SEPARATOR, "_")
}

/// Joins a parent's full name and a (already-sanitised) local name into a full
/// hierarchical name.
///
/// # Arguments
///
/// * `parent_full` - The parent's full name (empty for the implicit root).
/// * `local` - The sanitised local name.
///
/// # Returns
///
/// `local` when `parent_full` is empty (a top-level object under the root),
/// otherwise `"{parent_full}.{local}"`.
pub(crate) fn join(parent_full: &str, local: &str) -> String {
    if parent_full.is_empty() {
        local.to_owned()
    } else {
        format!("{parent_full}{SEPARATOR}{local}")
    }
}
