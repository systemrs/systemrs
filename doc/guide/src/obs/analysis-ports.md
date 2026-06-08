# Analysis ports

The **analysis port** is SystemRS's telemetry backbone — the mechanism a digital twin
uses to observe a model *without perturbing it*. An `AnalysisPort<T>` is a one-to-many
broadcast: a producer `write`s a value and it is delivered to every bound subscriber
**synchronously, immediately, in registration order, with no back-pressure**.

Subscribers implement `AnalysisWrite<T>` and are held *weakly* — binding a subscriber
does not keep it alive, so a tap can come and go without leaking. One write reaches all
of them:

```rust
use systemrs_tlm1::{AnalysisPort, AnalysisWrite};
use std::cell::RefCell;
use std::rc::Rc;

struct Sink(Rc<RefCell<Vec<i32>>>);
impl AnalysisWrite<i32> for Sink {
    fn write(&self, v: &i32) {
        self.0.borrow_mut().push(*v);
    }
}

let log = Rc::new(RefCell::new(Vec::new()));
let port: AnalysisPort<i32> = AnalysisPort::new();
let a = Rc::new(Sink(Rc::clone(&log)));
let b = Rc::new(Sink(Rc::clone(&log)));
port.bind(&a); // hold a/b yourself; the port keeps only a Weak
port.bind(&b);

port.write(&42);
assert_eq!(*log.borrow(), vec![42, 42]);
```

Because delivery is synchronous and in-order, an analysis port is the right tool for a
*scoreboard* (a self-checking subscriber comparing transactions to a reference) or a
live meter. It is non-intrusive: a model exposes a port, and observers attach or detach
freely — turning telemetry on changes nothing about the timeline.

For high-volume telemetry that must never stall the model, `AnalysisFifo<T>` is an
**unbounded** sink: `write` always succeeds, and a consumer drains the buffered values
one delta later. And `AnalysisTriple<T>` stamps a value with the time and delta it was
produced at.

> **Go deeper:** design report §3.7 (TLM-1 analysis ports), §6e (observability for
> twins).
