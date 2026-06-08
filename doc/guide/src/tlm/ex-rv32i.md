# Worked example: an RV32I hart

`cargo run --example rv32i_hart` runs a small **RISC-V CPU** — an RV32I integer hart —
that fetches, decodes, and executes a program, with *every* memory access routed through
loosely-timed transport to a memory target. It is the LT path under realistic load, and
a vivid demonstration of why processes are stackful coroutines.

The hart is one `SC_THREAD`: a fetch-decode-execute loop. Included from the example
source:

```rust,ignore
{{#include ../../../../crates/systemrs-examples/src/rv32i.rs:hart}}
```

The key line is `bus.read(pc, 4)` — the instruction fetch. That call goes through a
`SocketBus` adapter into `isock.b_transport(...)`, which (if the memory models latency)
`cx.wait`s. So a `wait` happens **several call frames deep**, inside what looks like an
ordinary `read`. In a coroutine-free design that would force `async` all the way up
through `bus.read`, `step`, and the loop; here the thread simply suspends and resumes,
its registers and program counter intact on its own stack. The decode/execute logic
(`step`) is plain Rust that knows nothing about simulation — it just calls `read`/`write`
on a `Bus` trait, which the testbench can back with either a socket or a plain `Vec` for
fast ISA unit tests.

Wire it up like any LT initiator/target pair:

```rust,ignore
let mem = Memory::new(4096, SimTime::from_ns(2));
let target = TargetSocket::new(&sim, "mem");
mem.connect(&sim, &target);
let isock = InitiatorSocket::new(&sim, "hart.isock");
isock.bind(&sim, &target);

mem.load(0, &rv32i::program_sum_1_to_n(100, RESULT_ADDR));
rv32i::build_hart(&sim, isock, /* entry */ 0, SimTime::from_ns(1));
sim.run_until(SimTime::from_us(100));
// the program computes sum(1..=100) = 5050 and stores it in memory
```

The integration test runs exactly this and checks the result in memory via a backdoor
read — the hart halts on an `ecall`, the run reaches starvation, and the sum is there.

> **Go deeper:** design report §6a (stackful coroutines and `wait` from depth), §6d
> (LT transport). Full source: `crates/systemrs-examples/src/rv32i.rs`.
