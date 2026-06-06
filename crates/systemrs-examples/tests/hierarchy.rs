//! Capstone integration tests for Milestone 2: the exit criteria, end-to-end.

use systemrs::prelude::*;
use systemrs_examples::platform::build_platform;

/// EC1/EC2/EC5/EC6: a two-level platform elaborates and names its objects, resolves
/// a socket bind at the barrier, runs the LT transaction, fires the lifecycle
/// callbacks in order, and picks up a module created during the construction
/// fixpoint.
#[test]
fn platform_elaborates_and_runs() {
    let kernel = Kernel::<Building>::new();
    let plat = build_platform(&kernel);
    let kernel = kernel.build();
    kernel.run(SimTime::from_ns(100));

    // EC2: the deferred socket bind resolved at the barrier; the transaction worked.
    assert_eq!(*plat.result.lock().expect("lock"), Some(0xCAFE));
    assert_eq!(plat.mem.read_u32(0x10), 0xCAFE);

    // EC1: unique dot-joined hierarchical names.
    {
        let store = store(kernel.sim());
        let s = store.borrow();
        assert_eq!(s.full_name(plat.top), "top");
        let names: Vec<String> = s
            .children(plat.top)
            .iter()
            .map(|&c| s.full_name(c).to_owned())
            .collect();
        assert!(names.contains(&"top.cpu".to_string()));
        assert!(names.contains(&"top.mem".to_string()));
    }

    // EC6: each module's callbacks fired in phase order.
    // EC5: the probe child created in `before_end_of_elaboration` got its callback.
    {
        let log = plat.log.borrow();
        let at = |t: &str| {
            log.iter()
                .position(|x| x == t)
                .unwrap_or_else(|| panic!("missing callback {t}"))
        };
        assert!(at("cpu:before") < at("cpu:end"));
        assert!(at("cpu:end") < at("cpu:start"));
        assert!(at("mem:before") < at("mem:end"));
        assert!(log.contains(&"probe_child:before".to_string())); // EC5 fixpoint
    }

    kernel.finish(); // fires end_of_simulation exactly once

    // EC6: end_of_simulation fired exactly once per module.
    let log = plat.log.borrow();
    assert_eq!(log.iter().filter(|x| *x == "cpu:eos").count(), 1);
    assert_eq!(log.iter().filter(|x| *x == "mem:eos").count(), 1);
}

/// EC3: a hierarchical port-to-port bind flattens through a parent to the channel
/// (exercised here through the public facade).
#[test]
fn hierarchical_port_to_port_resolves() {
    /// A compile-time interface tag.
    struct Irq;

    let sim = Sim::new();
    let leaf = {
        let store = store(&sim);
        let root = store.borrow().root();
        store
            .borrow_mut()
            .insert(root, "leaf", ObjectKind::PrimChannel)
    };

    let parent = Port::<Irq>::new(&sim, "parent");
    parent.bind_channel(&sim, leaf).expect("bind leaf");
    let child = Port::<Irq>::new(&sim, "child");
    child.bind_parent(&sim, &parent).expect("bind parent");

    assert_eq!(child.complete_binding(&sim).expect("resolve"), vec![leaf]);
}
