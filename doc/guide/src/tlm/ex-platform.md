# Worked example: a platform

Real systems are hierarchies of connected components. The `platform` example builds a
small two-level **platform** — a `top` module containing a `cpu` (an initiator) and a
`mem` (a target) — and binds them, demonstrating the elaboration API and deferred socket
binding from the [modules chapter](../core/modules.md).

The elaboration scope, included from the example source:

```rust,ignore
{{#include ../../../../crates/systemrs-examples/src/platform.rs:elaborate}}
```

What this shows:

- **Nested module instances.** `module_with("cpu", …)` and `module_with("mem", …)`
  construct child module *instances* inside the `top` scope; their children are named
  `top.cpu.*` and `top.mem.*`. Each build closure receives a `Builder` and returns the
  module value, whose lifecycle callbacks fire at the barrier.
- **Deferred binding.** `cpu.borrow().isock.bind(t.sim(), &tsock)` connects the CPU's
  initiator socket to the memory's target socket *during construction*. The binding is
  recorded now and **resolved at the elaboration barrier**, so it does not matter that
  the two modules were built moments apart — by the time the run starts, the path is
  live.

The example is driven through the `Kernel<Building>` front door (the compile-time
typestate): you build the platform while the kernel is `Building`, and once it is
`Running` the structural methods are gone — a misuse like binding a socket after start
simply fails to compile. The platform's CPU then issues a `b_transport` to the memory
and the test checks the result, with a callback log proving the elaboration lifecycle
ran in the right order.

> **Go deeper:** design report §3.4 (modules & elaboration), §3.10 (sockets), §6b
> (modules in Rust). Full source: `crates/systemrs-examples/src/platform.rs`.
