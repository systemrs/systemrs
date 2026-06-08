//! Example 3: a two-level TLM platform exercising the Milestone-2 hierarchy and
//! elaboration API.
//!
//! `top` contains a `cpu` module (an initiator socket + a thread that writes then
//! reads a word) and a `mem` module (a target socket backed by a [`systemrs::Memory`]),
//! built through the `Kernel<Building>` front door and the `module`/`Builder` scope
//! closures. `top` binds the cpu's initiator socket to the mem's target socket; the
//! binding is resolved at the elaboration barrier, after which the cpu thread drives
//! a live `b_transport` to memory.
//!
//! It demonstrates the M2 exit criteria end-to-end: unique dot-joined names (EC1),
//! a socket bind resolved via two-phase `complete_binding` (EC2), the construction
//! fixpoint (EC5, a probe that registers a child during `before_end_of_elaboration`),
//! and the four lifecycle callbacks firing in order (EC6). EC3 (hierarchical
//! port-to-port) and EC7 (port-policy cardinality) are covered by the unit tests; the
//! compile-time half of EC4 is shown here:
//!
//! ## Binding after start is a compile error (EC4, compile half)
//!
//! ```compile_fail
//! use systemrs::prelude::*;
//! let running = Kernel::<Building>::new().build(); // Kernel<Running>
//! running.module("late", |_m| {}); // ERROR: no method `module` on Kernel<Running>
//! ```

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use systemrs::prelude::*;

/// A shared, ordered log of `"tag:phase"` lifecycle-callback markers.
type Log = Rc<RefCell<Vec<String>>>;

/// Handles for inspecting a built [`build_platform`] platform.
pub struct Platform {
    /// The word the cpu thread read back (proves the bound transaction path).
    pub result: Arc<Mutex<Option<u32>>>,

    /// The ordered lifecycle-callback log across the modules.
    pub log: Log,

    /// The memory, for backdoor verification.
    pub mem: Memory,

    /// The `top` module's object id (root of the assertion walk).
    pub top: ObjectId,
}

/// The CPU module: an initiator socket and a thread driving `b_transport`.
struct Cpu {
    /// The forward initiator socket (bound by `top`).
    isock: InitiatorSocket,

    /// The shared callback log.
    log: Log,
}

impl Elaborate for Cpu {
    fn before_end_of_elaboration(&mut self, _ctx: &Ctx) {
        self.log.borrow_mut().push("cpu:before".to_owned());
    }
    fn end_of_elaboration(&mut self, _ctx: &Ctx) {
        self.log.borrow_mut().push("cpu:end".to_owned());
    }
    fn start_of_simulation(&mut self, _ctx: &Ctx) {
        self.log.borrow_mut().push("cpu:start".to_owned());
    }
    fn end_of_simulation(&mut self, _ctx: &Ctx) {
        self.log.borrow_mut().push("cpu:eos".to_owned());
    }
}

impl Module for Cpu {}

/// The memory module: a target socket backed by a [`Memory`].
struct Mem {
    /// The target socket (the cpu binds to this).
    tsock: TargetSocket,

    /// The shared callback log.
    log: Log,
}

impl Elaborate for Mem {
    fn before_end_of_elaboration(&mut self, _ctx: &Ctx) {
        self.log.borrow_mut().push("mem:before".to_owned());
    }
    fn end_of_elaboration(&mut self, _ctx: &Ctx) {
        self.log.borrow_mut().push("mem:end".to_owned());
    }
    fn start_of_simulation(&mut self, _ctx: &Ctx) {
        self.log.borrow_mut().push("mem:start".to_owned());
    }
    fn end_of_simulation(&mut self, _ctx: &Ctx) {
        self.log.borrow_mut().push("mem:eos".to_owned());
    }
}

impl Module for Mem {}

/// A probe that registers a child module during `before_end_of_elaboration`,
/// proving the construction fixpoint (the child still receives its callbacks).
struct Probe {
    /// The shared callback log.
    log: Log,

    /// Whether the child has already been spawned (spawn exactly once).
    spawned: bool,
}

impl Elaborate for Probe {
    fn before_end_of_elaboration(&mut self, ctx: &Ctx) {
        self.log.borrow_mut().push("probe:before".to_owned());
        if self.spawned {
            return;
        }
        self.spawned = true;
        // Register a child from inside the callback (the construction fixpoint); the
        // driver holds no store borrow here, so re-entering the store is safe.
        if let Some(store) = ctx.try_service::<RefCell<ObjectStore>>() {
            let root = store.borrow().root();
            let child = ProbeChild {
                log: Rc::clone(&self.log),
            };
            store.borrow_mut().register_elaborator(
                root,
                ObjectKind::Module,
                "probe_child",
                Rc::new(RefCell::new(child)),
            );
        }
    }
}

impl Module for Probe {}

/// The child spawned by [`Probe`] during the construction fixpoint.
struct ProbeChild {
    /// The shared callback log.
    log: Log,
}

impl Elaborate for ProbeChild {
    fn before_end_of_elaboration(&mut self, _ctx: &Ctx) {
        self.log.borrow_mut().push("probe_child:before".to_owned());
    }
}

impl Module for ProbeChild {}

/// Builds the cpu module: an initiator socket plus a thread that writes `0xCAFE` to
/// `0x10` and reads it back into `result`.
fn build_cpu(m: &mut Builder, result: Arc<Mutex<Option<u32>>>, log: Log) -> Cpu {
    let isock = InitiatorSocket::new(m.sim(), "isock");
    m.thread("run").finish(move |cx| {
        let mut delay = SimTime::ZERO;
        let mut wr = GenericPayload::write(0x10, 0xCAFE_u32.to_le_bytes().to_vec());
        isock.b_transport(cx, &mut wr, &mut delay);

        let mut rd = GenericPayload::read(0x10, 4);
        isock.b_transport(cx, &mut rd, &mut delay);
        let value = u32::from_le_bytes(rd.data().try_into().expect("4 bytes"));
        *result.lock().expect("lock") = Some(value);
    });
    Cpu { isock, log }
}

/// Builds the mem module: a target socket connected to a fresh memory, a clone of
/// which is stashed in `mem_slot` for backdoor verification.
fn build_mem(m: &mut Builder, log: Log, mem_slot: &RefCell<Option<Memory>>) -> Mem {
    let tsock = TargetSocket::new(m.sim(), "socket");
    let mem = Memory::new(256, SimTime::from_ns(2));
    mem.connect(m.sim(), &tsock);
    *mem_slot.borrow_mut() = Some(mem);
    Mem { tsock, log }
}

/// Builds the two-level platform into `kernel` and returns inspection handles.
///
/// # Arguments
///
/// * `kernel` - The building kernel.
///
/// # Returns
///
/// [`Platform`] handles for asserting names, the transaction result, and the
/// callback order.
///
/// # Panics
///
/// Panics if any construction step fails (it should not during elaboration).
pub fn build_platform(kernel: &Kernel<Building>) -> Platform {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let result = Arc::new(Mutex::new(None));
    let mem_slot: Rc<RefCell<Option<Memory>>> = Rc::new(RefCell::new(None));

    let top = kernel
        // ANCHOR: elaborate
        .module("top", |t| {
            let cpu = t
                .module_with("cpu", {
                    let r = Arc::clone(&result);
                    let l = Rc::clone(&log);
                    move |m| build_cpu(m, r, l)
                })
                .expect("cpu");

            let mem = t
                .module_with("mem", {
                    let l = Rc::clone(&log);
                    let ms = Rc::clone(&mem_slot);
                    move |m| build_mem(m, l, &ms)
                })
                .expect("mem");

            // Bind the cpu's initiator socket to the mem's target socket (deferred,
            // resolved at the barrier). Copy the handle out to drop the borrow.
            let tsock = mem.borrow().tsock;
            cpu.borrow().isock.bind(t.sim(), &tsock);

            t.module_with("probe", {
                let l = Rc::clone(&log);
                move |_m| Probe {
                    log: l,
                    spawned: false,
                }
            })
            .expect("probe");
        })
        .expect("top");
    // ANCHOR_END: elaborate

    let mem = mem_slot.borrow().clone().expect("mem built");
    Platform {
        result,
        log,
        mem,
        top,
    }
}
