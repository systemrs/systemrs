//! Example 4: a register-programmed DMA engine driving the AT protocol.
//!
//! A two-master offload pattern: a CPU programs a DMA controller's registers over the
//! **loosely-timed** path (`b_transport`), kicks it off, then waits for a completion
//! interrupt; the DMA copies a block of words through the **approximately-timed**
//! four-phase handshake (`nb_transport_fw`/`nb_transport_bw`) to an
//! [`AtMemory`](systemrs::AtMemory) whose backward responses are scheduled on a PEQ.
//! It exercises the parts of TLM-2.0 the LT examples don't:
//!
//! - **The AT four-phase FSM.** The copy engine drives `BEGIN_REQ` itself and
//!   completes the handshake with `END_RESP` from its `nb_transport_bw` callback —
//!   the explicit non-blocking transport path, not `b_transport`.
//! - **PEQ-timed responses.** The memory returns `BEGIN_RESP` one access-latency later
//!   via its phase queue, so each access advances simulation time.
//! - **Two initiators + an interrupt.** The CPU (LT) and the DMA (AT) are distinct
//!   masters on distinct paths; the DMA signals completion with a kernel event the CPU
//!   waits on. (Shared-bus arbitration between masters is a future extension, pending
//!   multi-socket fan-out.)
//!
//! The engine here is strictly sequential (one outstanding access); issuing several
//! `BEGIN_REQ`s before their responses — true AT pipelining — is the natural next
//! step, keyed by a per-transaction id as the LT↔AT adapters do.

use std::cell::RefCell;
use std::rc::Rc;

use systemrs::prelude::*;

/// Control register: copy source byte address (`u32`).
pub const REG_SRC: u64 = 0x00;

/// Control register: copy destination byte address (`u32`).
pub const REG_DST: u64 = 0x04;

/// Control register: number of 32-bit words to copy (`u32`).
pub const REG_LEN: u64 = 0x08;

/// Control register: writing here starts the copy (value ignored).
pub const REG_START: u64 = 0x0C;

/// The DMA's programmable descriptor, held as a `Sim` service so the (`Send`) copy
/// engine can read it without capturing a non-`Send` handle.
#[derive(Default)]
struct DmaRegs {
    /// Source byte address.
    src: u32,
    /// Destination byte address.
    dst: u32,
    /// Word count.
    len: u32,
}

/// A register-programmed DMA engine.
pub struct Dma {
    /// The completion-interrupt event (the CPU waits on it).
    irq: EventId,
}

impl Dma {
    /// Builds a DMA engine: a control register interface on `ctrl` (LT), an AT copy
    /// engine driving `mem`, and a completion interrupt on `irq`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `ctrl` - The control-register target socket (the CPU programs this over LT).
    /// * `mem` - The DMA's initiator socket, bound to an AT memory target.
    /// * `irq` - The event the DMA notifies on completion.
    ///
    /// # Returns
    ///
    /// The [`Dma`] handle.
    pub fn build(sim: &Sim, ctrl: &TargetSocket, mem: InitiatorSocket, irq: EventId) -> Dma {
        sim.register_service(Rc::new(RefCell::new(DmaRegs::default())));
        let start = sim.alloc_event();
        let done = sim.alloc_event();

        // Control register interface (loosely-timed): the CPU writes the descriptor;
        // a write to REG_START kicks the engine.
        ctrl.register_b_transport(sim, move |cx, payload, _delay| {
            let val = payload
                .data()
                .get(0..4)
                .and_then(|s| <[u8; 4]>::try_from(s).ok())
                .map_or(0, u32::from_le_bytes);
            let regs = cx.service::<RefCell<DmaRegs>>();
            match payload.address() {
                REG_SRC => regs.borrow_mut().src = val,
                REG_DST => regs.borrow_mut().dst = val,
                REG_LEN => regs.borrow_mut().len = val,
                REG_START => cx.notify(start),
                _ => {}
            }
            payload.set_response_status(ResponseStatus::Ok);
        });

        // Backward AT path: on BEGIN_RESP, complete the handshake (END_RESP) and wake
        // the engine. Strictly sequential, so a single `done` event suffices.
        mem.register_nb_transport_bw(sim, move |cx, txn, phase, _t| {
            if phase == Phase::BeginResp {
                let mut t = SimTime::ZERO;
                mem.nb_transport_fw(cx, txn, Phase::EndResp, &mut t);
                cx.notify(done);
            }
            TlmSync::Accepted
        });

        // The copy engine: an AT initiator that reads each word then writes it back.
        sim.add_thread("dma.engine", &[], true, move |cx| {
            let pool = TxnPool::new();
            loop {
                cx.wait_event(start);
                let (src, dst, len) = {
                    let regs = cx.service::<RefCell<DmaRegs>>();
                    let r = regs.borrow();
                    (u64::from(r.src), u64::from(r.dst), r.len)
                };
                for i in 0..u64::from(len) {
                    let off = i * 4;
                    // AT read of src+off.
                    let rd = pool.acquire();
                    *rd.borrow_mut() = GenericPayload::read(src + off, 4);
                    at_access(cx, mem, &rd, done);
                    let word = rd.borrow().data().to_vec();
                    // AT write of the same word to dst+off.
                    let wr = pool.acquire();
                    *wr.borrow_mut() = GenericPayload::write(dst + off, word);
                    at_access(cx, mem, &wr, done);
                }
                cx.notify(irq); // completion interrupt
            }
        });

        Dma { irq }
    }

    /// Returns the completion-interrupt event.
    pub fn irq(&self) -> EventId {
        self.irq
    }
}

/// Drives one AT transaction to completion: issue `BEGIN_REQ`, then block until the
/// backward path (which drives `END_RESP`) notifies `done`.
fn at_access(cx: &Ctx, mem: InitiatorSocket, txn: &Txn, done: EventId) {
    let mut t = SimTime::ZERO;
    mem.nb_transport_fw(cx, txn, Phase::BeginReq, &mut t);
    cx.wait_event(done);
}

#[cfg(test)]
mod tests {
    use super::{Dma, REG_DST, REG_LEN, REG_SRC, REG_START};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use systemrs::AtMemory;
    use systemrs::prelude::*;

    /// The CPU programs the DMA, which copies a block over AT; the data arrives at the
    /// destination, the CPU sees the completion interrupt, and time advanced by the
    /// modelled access latencies.
    #[test]
    fn cpu_programs_dma_which_copies_over_at() {
        const SRC: u32 = 0x100;
        const DST: u32 = 0x200;
        const WORDS: u32 = 8;

        let sim = Sim::new();
        let mem = AtMemory::new(1024, SimTime::from_ns(2));
        let mem_target = TargetSocket::new(&sim, "mem");
        mem.connect(&sim, &mem_target);

        let dma_mem = InitiatorSocket::new(&sim, "dma.mem");
        dma_mem.bind(&sim, &mem_target);
        let ctrl = TargetSocket::new(&sim, "dma.ctrl");
        let irq = sim.alloc_event();
        Dma::build(&sim, &ctrl, dma_mem, irq);

        let cpu = InitiatorSocket::new(&sim, "cpu");
        cpu.bind(&sim, &ctrl);

        // Seed the source region with a recognizable pattern (backdoor).
        let src_bytes: Vec<u8> = (0u32..WORDS * 4).map(|i| i.to_le_bytes()[0]).collect();
        mem.load(SRC as usize, &src_bytes);

        let saw_irq = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&saw_irq);
        sim.add_thread("cpu", &[], true, move |cx| {
            let prog = |reg: u64, val: u32| {
                let mut pay = GenericPayload::write(reg, val.to_le_bytes().to_vec());
                let mut delay = SimTime::ZERO;
                cpu.b_transport(cx, &mut pay, &mut delay);
            };
            prog(REG_SRC, SRC);
            prog(REG_DST, DST);
            prog(REG_LEN, WORDS);
            prog(REG_START, 1);
            cx.wait_event(irq); // block until the DMA completes
            flag.store(true, Ordering::SeqCst);
        });

        sim.run_until(SimTime::from_us(10));

        // The block was copied byte-for-byte to the destination.
        for (i, &b) in src_bytes.iter().enumerate() {
            assert_eq!(mem.read_byte(DST as usize + i), b);
        }
        assert!(saw_irq.load(Ordering::SeqCst), "CPU saw the completion IRQ");
        // 8 words × (read + write) × 2 ns latency = 32 ns of modelled AT traffic.
        assert_eq!(sim.now(), SimTime::from_ns(32));
    }
}
