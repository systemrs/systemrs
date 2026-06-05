//! Base-protocol conformance: an initiator `b_transport`s to a memory target.

use crate::{
    ByteEnable, Command, GenericPayload, InitiatorSocket, Memory, ResponseStatus, TargetSocket,
    TxnPool,
};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use systemrs_kernel::Sim;
use systemrs_time::SimTime;

/// Verifies an initiator thread can write then read a memory target over a bound
/// socket, with correct data and `Ok` responses, and that the modelled per-access
/// latency advances time (latency waited *inside* `b_transport`).
#[test]
fn b_transport_write_then_read_roundtrip() {
    let sim = Sim::new();

    let mem = Memory::new(256, SimTime::from_ns(5));
    let target = TargetSocket::new(&sim, "mem.target");
    mem.connect(&sim, &target);

    let isock = InitiatorSocket::new(&sim, "cpu.isock");
    isock.bind(&sim, &target);

    let observed: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let o = Arc::clone(&observed);

    sim.add_thread("cpu", &[], true, move |cx| {
        // Write the word 0xDEAD_BEEF at address 0x40.
        let mut wr = GenericPayload::write(0x40, 0xDEAD_BEEFu32.to_le_bytes().to_vec());
        let mut delay = SimTime::ZERO;
        isock.b_transport(cx, &mut wr, &mut delay);
        assert!(wr.is_response_ok());

        // Read it back.
        let mut rd = GenericPayload::read(0x40, 4);
        isock.b_transport(cx, &mut rd, &mut delay);
        assert_eq!(rd.response_status(), ResponseStatus::Ok);
        let value = u32::from_le_bytes(rd.data().try_into().expect("4 bytes"));
        o.lock().expect("lock").push(value);
    });

    sim.run_until(SimTime::from_ns(100));
    assert_eq!(*observed.lock().expect("lock"), vec![0xDEAD_BEEF]);
    // One write + one read, each modelling 5 ns of latency via wait().
    assert_eq!(sim.now(), SimTime::from_ns(10));
    // The backdoor read sees the same value with no latency.
    assert_eq!(mem.read_u32(0x40), 0xDEAD_BEEF);
}

/// Verifies an out-of-range access yields an `AddressError`, not a panic.
#[test]
fn out_of_range_access_is_address_error() {
    let sim = Sim::new();
    let mem = Memory::new(16, SimTime::ZERO);
    let target = TargetSocket::new(&sim, "mem.target");
    mem.connect(&sim, &target);
    let isock = InitiatorSocket::new(&sim, "cpu.isock");
    isock.bind(&sim, &target);

    let status = Arc::new(Mutex::new(ResponseStatus::Incomplete));
    let s = Arc::clone(&status);
    sim.add_thread("cpu", &[], true, move |cx| {
        let mut rd = GenericPayload::read(0x100, 4); // beyond 16 bytes
        let mut delay = SimTime::ZERO;
        isock.b_transport(cx, &mut rd, &mut delay);
        *s.lock().expect("lock") = rd.response_status();
    });
    sim.run_until(SimTime::from_ns(10));
    assert_eq!(*status.lock().expect("lock"), ResponseStatus::AddressError);
}

/// Verifies the response-status discriminants match SystemC's encoding and the
/// total `is_error` predicate ("discriminant < 0").
#[test]
fn response_status_discriminants_and_is_error() {
    assert_eq!(ResponseStatus::Ok.discriminant(), 1);
    assert_eq!(ResponseStatus::Incomplete.discriminant(), 0);
    assert_eq!(ResponseStatus::GenericError.discriminant(), -1);
    assert_eq!(ResponseStatus::ByteEnableError.discriminant(), -5);

    assert!(ResponseStatus::Ok.is_ok());
    assert!(!ResponseStatus::Ok.is_error());
    assert!(!ResponseStatus::Incomplete.is_error()); // 0 is NOT an error
    assert!(ResponseStatus::AddressError.is_error());
}

/// Verifies two initiators may re-enter the same target's `b_transport` while one
/// is parked at a `wait()` inside it — the callback is shared (`Rc<dyn Fn>`), not
/// taken out, so this no longer panics.
#[test]
fn shared_target_reentrant_b_transport() {
    let sim = Sim::new();
    let mem = Memory::new(256, SimTime::from_ns(5));
    let target = TargetSocket::new(&sim, "mem");
    mem.connect(&sim, &target);

    let sock_a = InitiatorSocket::new(&sim, "a");
    sock_a.bind(&sim, &target);
    let sock_b = InitiatorSocket::new(&sim, "b");
    sock_b.bind(&sim, &target);

    let done = Arc::new(AtomicU32::new(0));
    for (name, sock, addr, byte) in [("a", sock_a, 0x10u32, 0xAAu8), ("b", sock_b, 0x20, 0xBB)] {
        let d = Arc::clone(&done);
        sim.add_thread(name, &[], true, move |cx| {
            let mut wr = GenericPayload::write(u64::from(addr), vec![byte]);
            let mut delay = SimTime::ZERO;
            sock.b_transport(cx, &mut wr, &mut delay);
            assert!(wr.is_response_ok());
            d.fetch_add(1, Ordering::Relaxed);
        });
    }

    sim.run_until(SimTime::from_ns(100));
    assert_eq!(done.load(Ordering::Relaxed), 2);
    assert_eq!(mem.read_byte(0x10), 0xAA);
    assert_eq!(mem.read_byte(0x20), 0xBB);
}

/// Verifies `transport_dbg` services both peek (read) and poke (write) with no
/// modelled latency (it does not advance simulation time).
#[test]
fn transport_dbg_peek_and_poke() {
    let sim = Sim::new();
    let mem = Memory::new(256, SimTime::from_ns(5));
    let target = TargetSocket::new(&sim, "mem");
    mem.connect(&sim, &target);
    let isock = InitiatorSocket::new(&sim, "i");
    isock.bind(&sim, &target);

    let log: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let l = Arc::clone(&log);
    sim.add_thread("t", &[], true, move |cx| {
        // Poke a word via the backdoor.
        let mut poke = GenericPayload::write(4, 0x1234_5678u32.to_le_bytes().to_vec());
        let n = isock.transport_dbg(cx, &mut poke);
        l.lock().expect("lock").push(n); // 4 bytes serviced
        // Peek it back via the backdoor.
        let mut peek = GenericPayload::read(4, 4);
        isock.transport_dbg(cx, &mut peek);
        l.lock()
            .expect("lock")
            .push(u32::from_le_bytes(peek.data().try_into().expect("4 bytes")));
    });

    sim.run_until(SimTime::from_ns(100));
    assert_eq!(*log.lock().expect("lock"), vec![4, 0x1234_5678]);
    assert_eq!(mem.read_u32(4), 0x1234_5678);
    // The debug accesses advanced no time (the hart never b_transport'd).
    assert_eq!(sim.now(), SimTime::ZERO);
}

/// Verifies a read honours byte-enables: disabled byte lanes leave the initiator's
/// buffer untouched.
#[test]
fn read_honours_byte_enables() {
    let sim = Sim::new();
    let mem = Memory::new(256, SimTime::ZERO);
    let target = TargetSocket::new(&sim, "mem");
    mem.connect(&sim, &target);
    let isock = InitiatorSocket::new(&sim, "i");
    isock.bind(&sim, &target);
    // memory[0..4] = EF BE AD DE  (0xDEADBEEF little-endian)
    mem.load(0, &0xDEAD_BEEFu32.to_le_bytes());

    let result: Arc<Mutex<[u8; 4]>> = Arc::new(Mutex::new([0; 4]));
    let r = Arc::clone(&result);
    sim.add_thread("t", &[], true, move |cx| {
        let mut rd = GenericPayload::read(0, 4);
        rd.data_mut().copy_from_slice(&[0xFF; 4]); // pre-fill so untouched lanes show
        rd.set_byte_enable(ByteEnable::Mask(vec![0xff, 0x00, 0xff, 0x00]));
        let mut delay = SimTime::ZERO;
        isock.b_transport(cx, &mut rd, &mut delay);
        *r.lock().expect("lock") = rd.data().try_into().expect("4 bytes");
    });

    sim.run_until(SimTime::from_ns(10));
    // Lanes 0,2 enabled (EF, AD); lanes 1,3 disabled (stay FF).
    assert_eq!(*result.lock().expect("lock"), [0xEF, 0xFF, 0xAD, 0xFF]);
}

/// Verifies the transaction pool recycles a payload and resets stale fields.
#[test]
fn txn_pool_recycles_and_resets() {
    let pool = TxnPool::new();
    let txn = pool.acquire();
    txn.borrow_mut().set_command(Command::Write);
    txn.borrow_mut().set_address(0x1234);
    pool.recycle(txn);
    assert_eq!(pool.pooled(), 1);

    let reused = pool.acquire();
    // Fully reset on reuse (unlike SystemC's stale-field reset()).
    assert_eq!(reused.borrow().command(), Command::Ignore);
    assert_eq!(reused.borrow().address(), 0);
    assert_eq!(pool.pooled(), 0);
}
