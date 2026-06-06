//! The object hierarchy: an arena of [`ObjectMeta`] keyed by [`ObjectId`].
//!
//! This dissolves SystemC's raw parent/child pointer graph into a `SlotMap` of
//! metadata records linked by `Copy` ids (`doc/systemrs-design.md` §6b). The store
//! owns the hierarchical name table (the introspection key), a LIFO **scope stack**
//! that replaces the `sc_module_name` global, and an **implicit root** so that even
//! a flat model (one that never opens a module scope) has a well-defined current
//! scope to register objects under.
//!
//! The store is a [`Sim`] service (registered via [`store`]) exactly like the TLM
//! socket registry, so it is reachable during elaboration without threading a
//! handle through every constructor.

use std::cell::RefCell;
use std::rc::Rc;

use slotmap::SlotMap;
use std::collections::HashMap;
use systemrs_kernel::{ObjectId, Sim};

use crate::attribute::AttributeStore;
use crate::elaborate::Elaborate;
use crate::name;

/// A shared, dynamically-typed elaborator handle.
///
/// Held as `Rc<RefCell<dyn Elaborate>>` (not `Box`) so the elaboration driver can
/// clone the `Rc` *out* of the store and run a callback with **no** live store
/// borrow — the borrow-release discipline that prevents a re-entrant `cx.module`
/// from a callback body double-borrowing the store (`doc/systemrs-design.md` §6b).
pub(crate) type Elaborator = Rc<RefCell<dyn Elaborate>>;

/// The fixed order the four elaborator buckets are driven in each elaboration
/// phase: ports, then exports, then primitive channels, then modules. This
/// reproduces SystemC's registry order so binding completes (ports/exports) before
/// modules observe it in `end_of_elaboration` (`doc/systemrs-design.md` §6b).
pub(crate) const BUCKET_ORDER: [ObjectKind; 4] = [
    ObjectKind::Port,
    ObjectKind::Export,
    ObjectKind::PrimChannel,
    ObjectKind::Module,
];

/// One per-kind elaborator registry with its own construction-done cursor.
///
/// Four `ElabBucket`s (ports, exports, prim-channels, modules) reproduce SystemC's
/// separate registries: each cursor marks how far `before_end_of_elaboration` has
/// been driven, advancing independently so the construction fixpoint converges per
/// bucket (`doc/systemrs-design.md` §6b; `sc_module_registry.cpp`).
#[derive(Default)]
pub(crate) struct ElabBucket {
    /// Registered elaborators in insertion (source) order.
    pub(crate) entries: Vec<(ObjectId, Elaborator)>,

    /// How many entries have already had `before_end_of_elaboration` driven.
    pub(crate) construction_done_cursor: usize,
}

/// The role an object plays in the hierarchy.
///
/// Besides classifying an arena entry, the kind routes an *elaborator* into one of
/// the four per-bucket registries that reproduce SystemC's separate
/// `sc_port`/`sc_export`/`sc_prim_channel`/`sc_module` registries (the bucketing is
/// wired in M2-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// A module (`sc_module`): an interior node of the hierarchy.
    Module,

    /// A port (`sc_port`): a required-interface endpoint.
    Port,

    /// An export (`sc_export`): a provided-interface endpoint.
    Export,

    /// A primitive channel (`sc_prim_channel`): a `Signal`/`Fifo`/`Clock`/….
    PrimChannel,

    /// A process (`SC_METHOD`/`SC_THREAD`).
    Process,

    /// A kernel event surfaced into the hierarchy.
    Event,

    /// A TLM socket (a port + export pair, §6d).
    Socket,
}

/// Metadata for one object in the hierarchy.
///
/// The dissolved replacement for SystemC's raw parent/child back-pointer graph:
/// links are `ObjectId`s into the owning [`ObjectStore`], never references.
pub struct ObjectMeta {
    /// The object's local (base) name, sanitised and made unique among siblings.
    pub local_name: String,

    /// The object's full, dot-joined hierarchical name (the introspection key).
    pub full_name: String,

    /// The parent object, or `None` for the root.
    pub parent: Option<ObjectId>,

    /// The object's children, in insertion (source) order — deterministic.
    pub children: Vec<ObjectId>,

    /// The object's kind.
    pub kind: ObjectKind,

    /// Lazily-allocated typed attributes (`sc_attribute<T>`); `None` until first use.
    pub attributes: Option<AttributeStore>,
}

/// The object hierarchy arena, name table, and scope stack.
///
/// # Determinism
///
/// `children` and (in M2-03) the per-bucket registries are append-ordered by
/// insertion sequence (source order). The `names` table is consulted **only** for
/// existence checks during uniquification; it is never iterated to assign names or
/// to order callbacks, so no observable ordering depends on `HashMap` iteration
/// order (`doc/systemrs-design.md` §8).
pub struct ObjectStore {
    /// The object arena.
    objects: SlotMap<ObjectId, ObjectMeta>,

    /// Full-name → id, for uniqueness existence checks only (never iterated).
    names: HashMap<String, ObjectId>,

    /// The LIFO hierarchy scope stack; always non-empty (the root is the floor).
    scope_stack: Vec<ObjectId>,

    /// The implicit root object (`full_name == ""`).
    root: ObjectId,

    /// Port elaborators (driven first each fixpoint pass).
    ports: ElabBucket,

    /// Export elaborators (driven second).
    exports: ElabBucket,

    /// Primitive-channel elaborators (driven third).
    prim_channels: ElabBucket,

    /// Module elaborators (driven last).
    modules: ElabBucket,
}

impl ObjectStore {
    /// Creates a store containing only the implicit root, with the root as the
    /// current scope.
    ///
    /// # Returns
    ///
    /// A fresh [`ObjectStore`].
    pub fn new() -> Self {
        let mut objects = SlotMap::with_key();
        let root = objects.insert(ObjectMeta {
            local_name: String::new(),
            full_name: String::new(),
            parent: None,
            children: Vec::new(),
            kind: ObjectKind::Module,
            attributes: None,
        });
        ObjectStore {
            objects,
            names: HashMap::new(),
            scope_stack: vec![root],
            root,
            ports: ElabBucket::default(),
            exports: ElabBucket::default(),
            prim_channels: ElabBucket::default(),
            modules: ElabBucket::default(),
        }
    }

    /// Returns the implicit root object's id.
    pub fn root(&self) -> ObjectId {
        self.root
    }

    /// Returns the current (innermost) scope; the root if no module is open.
    pub fn current_scope(&self) -> ObjectId {
        self.scope_stack.last().copied().unwrap_or(self.root)
    }

    /// Pushes `id` as the current scope (called when a module body opens).
    ///
    /// # Arguments
    ///
    /// * `id` - The object whose scope is being entered.
    pub fn push_scope(&mut self, id: ObjectId) {
        self.scope_stack.push(id);
    }

    /// Pops the current scope, refusing to pop the root floor.
    ///
    /// # Returns
    ///
    /// The popped scope id, or `None` if only the root remained.
    pub fn pop_scope(&mut self) -> Option<ObjectId> {
        if self.scope_stack.len() > 1 {
            self.scope_stack.pop()
        } else {
            None
        }
    }

    /// Inserts a new object under `parent`, composing and uniquifying its name.
    ///
    /// The local name is sanitised (reserved separators replaced), joined to the
    /// parent's full name, and — on a collision — renamed with a numeric suffix
    /// (`name_0`, `name_1`, …) after a warning, mirroring `sc_gen_unique_name`.
    ///
    /// # Arguments
    ///
    /// * `parent` - The parent scope (e.g. [`ObjectStore::current_scope`]).
    /// * `local_name` - The caller-supplied local name.
    /// * `kind` - The object's kind.
    ///
    /// # Returns
    ///
    /// The new object's [`ObjectId`].
    pub fn insert(&mut self, parent: ObjectId, local_name: &str, kind: ObjectKind) -> ObjectId {
        let base = name::sanitize(local_name);
        let parent_full = self.objects[parent].full_name.clone();

        let mut local = base.clone();
        let mut full = name::join(&parent_full, &local);
        if self.names.contains_key(&full) {
            systemrs_diag::report_warning(
                "SYSTEMRS/OBJECT",
                &format!("object name '{full}' already exists; auto-renaming"),
            );
            let mut n = 0u64;
            while self.names.contains_key(&full) {
                local = format!("{base}_{n}");
                full = name::join(&parent_full, &local);
                n += 1;
            }
        }

        let id = self.objects.insert(ObjectMeta {
            local_name: local,
            full_name: full.clone(),
            parent: Some(parent),
            children: Vec::new(),
            kind,
            attributes: None,
        });
        self.objects[parent].children.push(id);
        self.names.insert(full, id);
        id
    }

    /// Returns the metadata for `id`, if present.
    ///
    /// # Arguments
    ///
    /// * `id` - The object id.
    ///
    /// # Returns
    ///
    /// A reference to the object's [`ObjectMeta`], or `None` if `id` is stale.
    pub fn get(&self, id: ObjectId) -> Option<&ObjectMeta> {
        self.objects.get(id)
    }

    /// Returns the full hierarchical name of `id` (empty for the root, `""` if stale).
    pub fn full_name(&self, id: ObjectId) -> &str {
        self.objects.get(id).map_or("", |m| m.full_name.as_str())
    }

    /// Returns the local (base) name of `id` (`""` if stale).
    pub fn basename(&self, id: ObjectId) -> &str {
        self.objects.get(id).map_or("", |m| m.local_name.as_str())
    }

    /// Returns the parent of `id`, if any.
    pub fn parent(&self, id: ObjectId) -> Option<ObjectId> {
        self.objects.get(id).and_then(|m| m.parent)
    }

    /// Returns the children of `id` in source order (empty if stale).
    pub fn children(&self, id: ObjectId) -> &[ObjectId] {
        self.objects.get(id).map_or(&[], |m| m.children.as_slice())
    }

    /// Returns the kind of `id`, if present.
    pub fn kind(&self, id: ObjectId) -> Option<ObjectKind> {
        self.objects.get(id).map(|m| m.kind)
    }

    /// Returns the number of objects in the store, including the root.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Returns `true` if the store holds only the implicit root.
    pub fn is_empty(&self) -> bool {
        self.objects.len() <= 1
    }

    /// Re-parents every child of `id` to the root (the orphan-to-root rule, §6b).
    ///
    /// Implemented as a pure id manipulation: each child's `parent` is set to the
    /// root and it is appended to the root's children; `id`'s own child list is
    /// cleared. Full names are intentionally not recomputed — in the M2 static
    /// hierarchy objects live for the whole simulation, so this is exercised only
    /// by the unit test (full destruction-order handling is deferred, §12 M7+).
    ///
    /// # Arguments
    ///
    /// * `id` - The object whose children are being re-parented.
    pub fn reparent_children_to_root(&mut self, id: ObjectId) {
        let root = self.root;
        let orphans = std::mem::take(&mut self.objects[id].children);
        for child in &orphans {
            if let Some(meta) = self.objects.get_mut(*child) {
                meta.parent = Some(root);
            }
        }
        self.objects[root].children.extend(orphans);
    }

    /// Returns the elaborator bucket for `kind`, if `kind` is an elaborator kind.
    fn bucket_for(&self, kind: ObjectKind) -> Option<&ElabBucket> {
        match kind {
            ObjectKind::Port => Some(&self.ports),
            ObjectKind::Export => Some(&self.exports),
            ObjectKind::PrimChannel => Some(&self.prim_channels),
            ObjectKind::Module => Some(&self.modules),
            ObjectKind::Process | ObjectKind::Event | ObjectKind::Socket => None,
        }
    }

    /// Returns the mutable elaborator bucket for `kind`, if any.
    fn bucket_for_mut(&mut self, kind: ObjectKind) -> Option<&mut ElabBucket> {
        match kind {
            ObjectKind::Port => Some(&mut self.ports),
            ObjectKind::Export => Some(&mut self.exports),
            ObjectKind::PrimChannel => Some(&mut self.prim_channels),
            ObjectKind::Module => Some(&mut self.modules),
            ObjectKind::Process | ObjectKind::Event | ObjectKind::Socket => None,
        }
    }

    /// Inserts an object and registers it as an elaborator in its kind's bucket.
    ///
    /// The four elaborator kinds (`Port`/`Export`/`PrimChannel`/`Module`) land in
    /// their own bucket so the driver can drive their callbacks in the fixed
    /// inter-bucket order with independent construction-done cursors. A non-
    /// elaborator `kind` (`Process`/`Event`/`Socket`) is still inserted into the
    /// hierarchy, but the supplied `elaborator` is ignored (such objects use
    /// [`ObjectStore::insert`] directly).
    ///
    /// # Arguments
    ///
    /// * `parent` - The parent scope.
    /// * `kind` - The object's kind (selects the bucket).
    /// * `local_name` - The caller-supplied local name.
    /// * `elaborator` - The elaborator handle to drive at the barrier.
    ///
    /// # Returns
    ///
    /// The new object's [`ObjectId`].
    pub fn register_elaborator(
        &mut self,
        parent: ObjectId,
        kind: ObjectKind,
        local_name: &str,
        elaborator: Elaborator,
    ) -> ObjectId {
        let id = self.insert(parent, local_name, kind);
        if let Some(bucket) = self.bucket_for_mut(kind) {
            bucket.entries.push((id, elaborator));
        }
        id
    }

    /// Returns the number of elaborators registered in `kind`'s bucket.
    ///
    /// # Arguments
    ///
    /// * `kind` - An elaborator kind.
    ///
    /// # Returns
    ///
    /// The bucket's entry count, or `0` for a non-elaborator kind.
    pub fn bucket_len(&self, kind: ObjectKind) -> usize {
        self.bucket_for(kind).map_or(0, |b| b.entries.len())
    }

    /// Returns the construction-done cursor for `kind`'s bucket.
    ///
    /// # Arguments
    ///
    /// * `kind` - An elaborator kind.
    ///
    /// # Returns
    ///
    /// The bucket's cursor, or `0` for a non-elaborator kind.
    pub fn bucket_cursor(&self, kind: ObjectKind) -> usize {
        self.bucket_for(kind)
            .map_or(0, |b| b.construction_done_cursor)
    }

    /// Returns clones of the elaborators in `kind`'s bucket that have not yet had
    /// `before_end_of_elaboration` driven, advancing that bucket's cursor to the
    /// current length.
    ///
    /// The driver clones the `Rc`s *out* under this brief borrow, then releases the
    /// store before invoking the callbacks — so a callback that creates new objects
    /// (re-entering the store) cannot double-borrow it (`doc/systemrs-design.md` §6b).
    /// Objects created during those callbacks append past the new cursor and are
    /// returned on the next call (the construction fixpoint).
    ///
    /// # Arguments
    ///
    /// * `kind` - An elaborator kind.
    ///
    /// # Returns
    ///
    /// The newly-registered elaborators of `kind` since the last call.
    pub(crate) fn take_new_before_end(&mut self, kind: ObjectKind) -> Vec<Elaborator> {
        let Some(bucket) = self.bucket_for_mut(kind) else {
            return Vec::new();
        };
        let start = bucket.construction_done_cursor;
        let end = bucket.entries.len();
        bucket.construction_done_cursor = end;
        bucket.entries[start..end]
            .iter()
            .map(|(_, e)| Rc::clone(e))
            .collect()
    }

    /// Attaches an elaborator to an already-inserted object, routing it into the
    /// bucket for the object's kind.
    ///
    /// Used for modules, whose object is inserted first (to open the hierarchy
    /// scope under which children are built) and whose instance is registered as an
    /// elaborator afterwards. A no-op if `id` is stale or not an elaborator kind.
    ///
    /// # Arguments
    ///
    /// * `id` - The object to attach the elaborator to.
    /// * `elaborator` - The elaborator handle to drive.
    pub(crate) fn attach_elaborator(&mut self, id: ObjectId, elaborator: Elaborator) {
        let Some(kind) = self.kind(id) else {
            return;
        };
        if let Some(bucket) = self.bucket_for_mut(kind) {
            bucket.entries.push((id, elaborator));
        }
    }

    /// Returns clones of every elaborator in `kind`'s bucket (for the
    /// `end_of_elaboration`/`start_of_simulation`/`end_of_simulation` passes).
    ///
    /// # Arguments
    ///
    /// * `kind` - An elaborator kind.
    ///
    /// # Returns
    ///
    /// All elaborators of `kind`, in registration order.
    pub(crate) fn all_elaborators(&self, kind: ObjectKind) -> Vec<Elaborator> {
        self.bucket_for(kind).map_or_else(Vec::new, |b| {
            b.entries.iter().map(|(_, e)| Rc::clone(e)).collect()
        })
    }
}

impl Default for ObjectStore {
    fn default() -> Self {
        ObjectStore::new()
    }
}

/// Returns the simulation's object store, creating and registering it on first use.
///
/// Mirrors the TLM socket registry's lazy `Sim`-service pattern, so the store is
/// shared by every elaboration-time constructor without an explicit handle.
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
///
/// # Returns
///
/// The shared [`ObjectStore`] handle.
pub fn store(sim: &Sim) -> Rc<RefCell<ObjectStore>> {
    let ctx = sim.ctx();
    if let Some(existing) = ctx.try_service::<RefCell<ObjectStore>>() {
        return existing;
    }
    let store = Rc::new(RefCell::new(ObjectStore::new()));
    sim.register_service(Rc::clone(&store));
    // Installing the store also installs the elaboration driver and teardown hook,
    // so any model that creates hierarchy objects gets the barrier driven
    // automatically. A hierarchy-free model never calls `store`, so its `run_until`
    // stays hook-free and bit-identical (`doc/systemrs-design.md` §6b).
    sim.set_elaboration_hook(crate::elaboration::drive);
    sim.set_end_of_sim_hook(crate::elaboration::end_of_simulation);
    store
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two-level inserts produce dot-joined names; the root contributes no prefix.
    #[test]
    fn dot_joined_names() {
        let mut s = ObjectStore::new();
        let cpu = s.insert(s.current_scope(), "cpu", ObjectKind::Module);
        s.push_scope(cpu);
        let r = s.insert(s.current_scope(), "r", ObjectKind::PrimChannel);
        assert_eq!(s.full_name(cpu), "cpu");
        assert_eq!(s.full_name(r), "cpu.r");
        assert_eq!(s.basename(r), "r");
        assert_eq!(s.parent(r), Some(cpu));
        assert_eq!(s.children(cpu), &[r]);
    }

    /// A duplicate sibling name is auto-renamed with a numeric suffix.
    #[test]
    fn duplicate_name_is_renamed() {
        let mut s = ObjectStore::new();
        let a = s.insert(s.root(), "x", ObjectKind::Module);
        let b = s.insert(s.root(), "x", ObjectKind::Module);
        assert_eq!(s.full_name(a), "x");
        assert_eq!(s.full_name(b), "x_0");
        assert_ne!(a, b);
    }

    /// An embedded separator in a local name is sanitised, not treated as a path.
    #[test]
    fn embedded_separator_is_sanitised() {
        let mut s = ObjectStore::new();
        let id = s.insert(s.root(), "a.b", ObjectKind::Module);
        assert_eq!(s.full_name(id), "a_b");
    }

    /// Deterministic suffixing: the same source order always yields the same names.
    #[test]
    fn sibling_order_is_deterministic() {
        let names = |()| {
            let mut s = ObjectStore::new();
            let p = s.insert(s.root(), "p", ObjectKind::Module);
            s.push_scope(p);
            for n in ["m", "m", "m"] {
                s.insert(s.current_scope(), n, ObjectKind::Module);
            }
            s.children(p)
                .iter()
                .map(|&c| s.full_name(c).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(()), names(()));
        assert_eq!(names(()), vec!["p.m", "p.m_0", "p.m_1"]);
    }

    /// A fresh store has the root as a non-empty current scope; inserts never panic.
    #[test]
    fn implicit_root_gives_a_scope() {
        let mut s = ObjectStore::new();
        assert_eq!(s.current_scope(), s.root());
        assert!(s.is_empty());
        let id = s.insert(s.current_scope(), "top", ObjectKind::Module);
        assert_eq!(s.parent(id), Some(s.root()));
        // The root cannot be popped away.
        assert_eq!(s.pop_scope(), None);
        assert_eq!(s.current_scope(), s.root());
    }

    /// Re-parenting moves children under the root and updates the root's child list.
    #[test]
    fn reparent_children_to_root_moves_children() {
        let mut s = ObjectStore::new();
        let p = s.insert(s.root(), "p", ObjectKind::Module);
        s.push_scope(p);
        let c = s.insert(s.current_scope(), "c", ObjectKind::Module);
        s.pop_scope();
        s.reparent_children_to_root(p);
        assert_eq!(s.parent(c), Some(s.root()));
        assert!(s.children(p).is_empty());
        assert!(s.children(s.root()).contains(&c));
    }

    /// `store(sim)` registers exactly one shared instance (idempotent).
    #[test]
    fn store_service_is_idempotent() {
        let sim = Sim::new();
        let a = store(&sim);
        let b = store(&sim);
        assert!(Rc::ptr_eq(&a, &b));
    }

    /// An elaborator that overrides `object_kind`, for bucket-routing tests.
    struct Kinded(ObjectKind);

    impl Elaborate for Kinded {
        fn object_kind(&self) -> ObjectKind {
            self.0
        }
    }

    /// An elaborator using the default `object_kind` (must be `Module`).
    struct DefaultElab;

    impl Elaborate for DefaultElab {}

    /// The trait default routes to the module bucket.
    #[test]
    fn default_object_kind_is_module() {
        assert_eq!(DefaultElab.object_kind(), ObjectKind::Module);
    }

    /// Elaborators registered in scrambled order land in their own buckets, each
    /// cursor independent (registration never advances a cursor).
    #[test]
    fn elaborators_route_to_their_own_bucket() {
        let mut s = ObjectStore::new();
        let root = s.root();
        let order = [
            ObjectKind::Module,
            ObjectKind::Port,
            ObjectKind::PrimChannel,
            ObjectKind::Export,
            ObjectKind::Port,
        ];
        for (i, &k) in order.iter().enumerate() {
            let e: Elaborator = Rc::new(RefCell::new(Kinded(k)));
            let id = s.register_elaborator(root, k, &format!("o{i}"), e);
            assert_eq!(s.kind(id), Some(k));
        }
        assert_eq!(s.bucket_len(ObjectKind::Port), 2);
        assert_eq!(s.bucket_len(ObjectKind::Export), 1);
        assert_eq!(s.bucket_len(ObjectKind::PrimChannel), 1);
        assert_eq!(s.bucket_len(ObjectKind::Module), 1);
        for k in [
            ObjectKind::Port,
            ObjectKind::Export,
            ObjectKind::PrimChannel,
            ObjectKind::Module,
        ] {
            assert_eq!(s.bucket_cursor(k), 0);
        }

        // A non-elaborator kind is inserted into the hierarchy but not bucketed.
        let p: Elaborator = Rc::new(RefCell::new(Kinded(ObjectKind::Process)));
        let pid = s.register_elaborator(root, ObjectKind::Process, "proc", p);
        assert!(s.get(pid).is_some());
        assert_eq!(s.bucket_len(ObjectKind::Module), 1);
    }
}
