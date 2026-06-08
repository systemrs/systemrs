# The generic payload and sockets

Transaction-level modeling means components talk by passing *transactions*. SystemRS
implements the TLM-2.0 standard for this: a **generic payload** describes a transaction,
and **sockets** connect an initiator to a target.

## The generic payload

A `GenericPayload` is the universal transaction object — a memory-mapped command with an
address, a data buffer, a command, and a response status. The constructors cover the two
common cases:

```rust
# use systemrs::prelude::*;
let write = GenericPayload::write(0x40, vec![0xDE, 0xAD, 0xBE, 0xEF]);
let read = GenericPayload::read(0x40, 4); // read 4 bytes at 0x40
```

A target inspects `payload.command()` and `payload.address()`, services the access via
`payload.data()` / `payload.data_mut()`, and reports the outcome with
`payload.set_response_status(ResponseStatus::Ok)`. Byte-enables, streaming width, and
DMI hints round out the payload, but most models start with command + address + data.

For the loosely-timed path you hand a target a `&mut GenericPayload` directly. For the
approximately-timed path a transaction is shared across phases as a
`Txn = Rc<RefCell<GenericPayload>>` drawn from a `TxnPool` — covered in the
[AT chapter](at.md).

## Sockets

An **initiator** issues transactions; a **target** services them. Each owns a socket,
and you `bind` the initiator's socket to the target's:

```rust
# use systemrs::prelude::*;
# let sim = Sim::new();
let target = TargetSocket::new(&sim, "mem");
let isock = InitiatorSocket::new(&sim, "cpu");
isock.bind(&sim, &target);
```

Sockets are `Copy` handles like every other object. The binding is resolved at the
elaboration barrier, so order does not matter. A target supplies the *behaviour* by
registering a callback (`register_b_transport`, or the AT `register_nb_transport_fw`);
`Memory` is a ready-made target that registers a read/write/latency callback for you.

## A reference target: `Memory`

`Memory` models a byte-addressable RAM with a per-access latency. Connect it to a target
socket and it services reads and writes:

```rust
# use systemrs::prelude::*;
# let sim = Sim::new();
# let target = TargetSocket::new(&sim, "mem");
let mem = Memory::new(256, SimTime::from_ns(5)); // 256 bytes, 5 ns/access
mem.connect(&sim, &target);
mem.load(0, &[1, 2, 3, 4]);                       // backdoor preload (no latency)
let _ = mem.read_u32(0);                          // backdoor read
```

The [next chapter](lt.md) drives a transaction through this end to end.

> **Go deeper:** design report §3.8 (generic payload), §3.10 (sockets), §6d (TLM-2 API
> design).
