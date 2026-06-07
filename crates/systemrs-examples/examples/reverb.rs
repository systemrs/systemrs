//! Runnable example: a fixed-point electric-guitar reverb pedal.
//!
//! Run with `cargo run --example reverb`. A short plucked-string signal is streamed
//! in blocks through the reverb pedal over `b_transport`; the per-block peak output
//! level (broadcast on the pedal's `AnalysisPort`) is printed as an ASCII meter, so
//! you can watch the reverb tail ring out after the pluck stops.

// DSP demo math converts sample indices to `f64`; the precision loss is harmless here.
#![allow(clippy::cast_precision_loss)]

use std::cell::Cell;
use std::rc::Rc;

use systemrs::prelude::*;
use systemrs_examples::reverb::{ReverbParams, ReverbPedal, pack_block, unpack_block};

/// Samples per streamed block.
const BLOCK: usize = 64;

/// A meter that prints one ASCII bar per block from the broadcast peak level.
struct Meter {
    /// Block counter (the analysis port carries only the level).
    block: Cell<usize>,
}

impl AnalysisWrite<i64> for Meter {
    fn write(&self, peak: &i64) {
        let n = self.block.get();
        self.block.set(n + 1);
        // Q2.14 full-scale is 1<<14; scale a peak to a 0..40 bar.
        let bars = ((*peak * 40) / 16_384).clamp(0, 40);
        let bars = usize::try_from(bars).unwrap_or(0);
        println!("block {n:>2} |{:<40}|", "#".repeat(bars));
    }
}

/// Builds the pedal, streams a plucked-string signal followed by silence, and prints
/// the metered output so the reverb tail is visible.
fn main() {
    let sim = Sim::new();
    let target = TargetSocket::new(&sim, "pedal");
    let pedal = ReverbPedal::connect(&sim, &target, ReverbParams::default());

    let meter = Rc::new(Meter {
        block: Cell::new(0),
    });
    pedal.level_port().bind(&meter);

    let isock = InitiatorSocket::new(&sim, "guitar");
    isock.bind(&sim, &target);

    sim.add_thread("guitar", &[], true, move |cx| {
        // 4 blocks of a decaying 220 Hz pluck (at a notional 48 kHz), then 20 blocks
        // of silence so the reverb tail rings out.
        let mut idx = 0usize;
        for block in 0..24 {
            let samples: Vec<f64> = (0..BLOCK)
                .map(|i| {
                    let n = idx + i;
                    if block < 4 {
                        let t = n as f64 / 48_000.0;
                        let env = (-t * 8.0).exp(); // ~125 ms decay
                        0.8 * env * (core::f64::consts::TAU * 220.0 * t).sin()
                    } else {
                        0.0
                    }
                })
                .collect();
            idx += BLOCK;

            let mut payload = GenericPayload::write(0, pack_block(&samples));
            let mut delay = SimTime::ZERO;
            isock.b_transport(cx, &mut payload, &mut delay);
            let _out = unpack_block(payload.data()); // the processed block
            cx.wait(SimTime::from_us(1)); // one block-quantum of audio
        }
    });

    println!("Guitar reverb: a pluck (blocks 0–3) ringing out through the tail:");
    sim.run_until(SimTime::from_us(1_000));
    println!("Done at {}.", sim.now());
}
