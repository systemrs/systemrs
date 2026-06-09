//! M7 bounded structural hot-swap (`doc/systemrs-design.md` §6f): a target's forward
//! transport (`Rc<dyn FwTransport>`) is replaced at a quiescent point, keeping the socket
//! binding intact — the same seam an out-of-tree interop bridge plugs into (§11).

use std::cell::RefCell;
use std::rc::Rc;

use systemrs::{
    Ctx, FwTransport, GenericPayload, InitiatorSocket, ResponseStatus, Sim, SimTime, TargetSocket,
};

type Log = Rc<RefCell<Vec<u8>>>;

/// A forward target that records its tag whenever it services a transaction.
struct Tagged {
    tag: u8,
    log: Log,
}

impl FwTransport for Tagged {
    fn b_transport(&mut self, _ctx: &Ctx, txn: &mut GenericPayload, _delay: &mut SimTime) {
        self.log.borrow_mut().push(self.tag);
        txn.set_response_status(ResponseStatus::Ok);
    }
}

/// Bind an initiator to a target, run one transaction, swap the target's forward
/// transport at the quiescent boundary, run a second transaction — the swap takes effect
/// without rebinding (the socket's `ObjectId` is unchanged).
#[test]
fn fw_transport_hot_swap_at_quiescent_point() {
    let sim = Sim::new();
    let log: Log = Rc::new(RefCell::new(Vec::new()));

    let target = TargetSocket::new(&sim, "mem");
    target.set_fw_transport(
        &sim,
        Rc::new(RefCell::new(Tagged {
            tag: 0xAA,
            log: Rc::clone(&log),
        })),
    );
    let isock = InitiatorSocket::new(&sim, "cpu");
    isock.bind(&sim, &target);

    sim.add_thread("cpu", &[], true, move |cx| {
        let mut delay = SimTime::ZERO;
        let mut first = GenericPayload::read(0, 1);
        isock.b_transport(cx, &mut first, &mut delay); // serviced by tag 0xAA
        cx.wait(SimTime::from_ns(10));
        let mut second = GenericPayload::read(0, 1);
        isock.b_transport(cx, &mut second, &mut delay); // serviced by the swapped-in tag
    });

    sim.run_until(SimTime::from_ns(5)); // first transaction done; thread parked until 10 ns

    // Hot-swap at a quiescent point: same binding, a different forward target.
    target.set_fw_transport(
        &sim,
        Rc::new(RefCell::new(Tagged {
            tag: 0xBB,
            log: Rc::clone(&log),
        })),
    );

    sim.run_until(SimTime::from_ns(15)); // second transaction done

    assert_eq!(
        *log.borrow(),
        vec![0xAA, 0xBB],
        "the swap changed which forward target serviced the transaction, without rebinding"
    );
}
