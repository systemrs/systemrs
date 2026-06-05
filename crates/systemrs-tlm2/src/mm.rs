//! Payload memory management: pooled, reference-counted transactions.
//!
//! Since the kernel is single-threaded by spec, `Rc` is correct and `Arc` would be
//! a category error (`doc/systemrs-design.md` §6d). [`TxnPool::acquire`] performs a
//! full reset on reuse — deliberately fixing SystemC's stale-field hazard — so a
//! premature-recycle bug (which SystemC asserts on) is impossible here.

use std::cell::RefCell;
use std::rc::Rc;

use crate::gp::GenericPayload;

/// A pooled, shareable transaction handle (replaces `GP* + acquire/release`).
pub type Txn = Rc<RefCell<GenericPayload>>;

/// A recycling pool of generic payloads.
#[derive(Default)]
pub struct TxnPool {
    /// Free payloads available for reuse.
    free: RefCell<Vec<GenericPayload>>,
}

impl TxnPool {
    /// Creates an empty pool.
    pub fn new() -> Self {
        TxnPool::default()
    }

    /// Acquires a fully-reset transaction (popped from the pool or freshly made).
    ///
    /// # Returns
    ///
    /// A reference-counted [`Txn`] in a clean state.
    pub fn acquire(&self) -> Txn {
        let mut gp = self.free.borrow_mut().pop().unwrap_or_default();
        gp.reset();
        Rc::new(RefCell::new(gp))
    }

    /// Recycles a transaction back into the pool, if it is the sole owner.
    ///
    /// Mirrors `release()` at reference count 0: a transaction still referenced
    /// elsewhere (e.g. parked in a PEQ) is simply dropped here and not pooled.
    ///
    /// # Arguments
    ///
    /// * `txn` - The transaction to recycle.
    pub fn recycle(&self, txn: Txn) {
        if let Ok(cell) = Rc::try_unwrap(txn) {
            self.free.borrow_mut().push(cell.into_inner());
        }
    }

    /// Returns the number of payloads currently pooled.
    pub fn pooled(&self) -> usize {
        self.free.borrow().len()
    }
}
