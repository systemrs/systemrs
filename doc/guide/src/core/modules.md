# Modules, hierarchy, elaboration

Real models are not flat. A **module** is a named scope in the object hierarchy; its
children — sub-modules, channels, sockets, processes — take hierarchical names from it.
Modules are built during *elaboration* and frozen when the run starts.

## A named scope

The simplest form creates an anonymous scope and populates it:

```rust
# use systemrs::prelude::*;
use systemrs_core::module;

let sim = Sim::new();
let top = module(&sim, "top", |b| {
    b.module("inner", |_inner| { /* children named "top.inner.*" */ }).expect("nested");
}).expect("during elaboration");
let _ = top; // the module's ObjectId in the hierarchy
```

The closure receives a `Builder` (`b`): `b.sim()` gives the simulation for creating
children, and `b.module(...)` / `b.module_with(...)` / `b.thread(...)` / `b.method(...)`
create them in scope. Anything created with `X::new(b.sim(), name)` inside the closure is
named relative to the enclosing module.

## Module instances and the `#[module]` macro

`module_with` registers a module **instance** — a value you construct that carries its
own state and whose `Elaborate` lifecycle callbacks (`before_end_of_elaboration`,
`end_of_elaboration`, `start_of_simulation`, `end_of_simulation`) fire at the barrier.
The `#[module]` attribute macro generates that boilerplate, so a struct can become a
module with its children declared as fields. The [platform tutorial](../tlm/ex-platform.md)
shows a two-level instance hierarchy end to end.

## The elaboration barrier, and the typestate

Construction and run are two phases separated by an **elaboration barrier**. Bindings you
make during construction (e.g. connecting a socket to a target) are *deferred* and
resolved at the barrier — so you can bind a socket before its target is fully built. The
`Sim` enforces "no structure changes after start" at runtime; for a compile-time
guarantee there is a `Kernel<Building>` / `Kernel<Running>` typestate front door whose
`module`/`bind` methods simply do not exist on the running state:

```rust,ignore
running.module("late", |_m| {}); // compile error: no `module` on Kernel<Running>
```

> **Go deeper:** design report §3.4 (modules, objects & elaboration), §6b (modules in
> Rust).
