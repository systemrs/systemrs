//! Example 5: a real-time sensor-monitoring digital twin.
//!
//! A purely externally-driven model that ties the whole digital-twin layer together
//! (`doc/systemrs-design.md` §6f): it sits **parked** until a sensor reading is
//! injected from outside, wakes to process it, then parks again — never exiting on
//! idle. Each reading is calibrated and perturbed by a **seeded** noise model, the
//! processed value is broadcast on an [`AnalysisPort`] for live monitoring, and the
//! injections are recorded to a [`Journal`] so the run **replays byte-identically**.
//!
//! - **External input + suspend-on-starvation.** Readings arrive on an mpsc inbox
//!   from a producer thread; [`attach_external_input`] parks the sim between them.
//! - **Real-time pacing.** A [`RealTimePacer`] throttles the per-sample time advance
//!   to wall clock, so a burst of queued readings is processed at a steady cadence
//!   rather than instantly.
//! - **Deterministic replay.** The recorded journal + the RNG seed reproduce the exact
//!   processed-value sequence with no live thread and no pacing.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use systemrs::Journal;
use systemrs::prelude::*;

/// A processed reading: `(simulation-time units, calibrated value)`.
pub type Reading = (u64, i64);

/// Configuration for a [`build_sensor`] twin.
#[derive(Clone, Copy)]
pub struct SensorParams {
    /// The modelled time between samples (paced to wall clock in live mode).
    pub sample_period: SimTime,

    /// Measurement-noise half-range in LSBs (`0` = noiseless). Each reading is
    /// perturbed by a seeded value in `[-noise, +noise]`.
    pub noise: u64,

    /// The RNG seed (recorded in the journal; restore it to replay faithfully).
    pub seed: u64,
}

impl Default for SensorParams {
    fn default() -> Self {
        SensorParams {
            sample_period: SimTime::from_us(100),
            noise: 4,
            seed: 0x005E_1150,
        }
    }
}

/// Calibrates a raw 10-bit ADC count to a signed, zero-centred reading.
fn calibrate(raw: u64) -> i64 {
    i64::try_from(raw).unwrap_or(0) - 512
}

/// Injects a raw reading into the running twin: queue it and wake the sensor process.
/// Used as the injector for both the live inbox and the journal replayer, so the two
/// paths are identical.
///
/// # Arguments
///
/// * `cx` - A kernel handle.
/// * `sample` - The event the sensor process waits on.
/// * `raw` - The raw reading.
pub fn inject(cx: &Ctx, sample: EventId, raw: u64) {
    cx.service::<RefCell<VecDeque<u64>>>()
        .borrow_mut()
        .push_back(raw);
    cx.notify(sample);
}

/// Builds the sensor twin into `sim`: the RNG service, the inbox queue, and the sensor
/// process. Returns the [`AnalysisPort`] that broadcasts each processed [`Reading`]
/// (bind a meter/recorder to it).
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
/// * `params` - The twin configuration (seeds the RNG).
/// * `sample` - The event the sensor process waits on (also notified by [`inject`]).
///
/// # Returns
///
/// The shared processed-reading [`AnalysisPort`].
pub fn build_sensor(
    sim: &Sim,
    params: &SensorParams,
    sample: EventId,
) -> Rc<AnalysisPort<Reading>> {
    Rng::install(sim, params.seed);
    sim.register_service(Rc::new(RefCell::new(VecDeque::<u64>::new())));
    let port = Rc::new(AnalysisPort::<Reading>::new());
    sim.register_service(Rc::clone(&port));

    let period = params.sample_period;
    let noise = params.noise;
    sim.add_thread("sensor.twin", &[], true, move |cx| {
        loop {
            cx.wait_event(sample); // park here when caught up (suspend-on-starvation)
            loop {
                let raw = cx
                    .service::<RefCell<VecDeque<u64>>>()
                    .borrow_mut()
                    .pop_front();
                let Some(raw) = raw else { break };
                cx.wait(period); // model the sample interval (paced in live mode)
                let rng = Rng::from_ctx(cx);
                let span = i64::try_from(noise).unwrap_or(0);
                let draw = i64::try_from(rng.gen_range(0, noise * 2 + 1)).unwrap_or(0);
                let value = calibrate(raw) + (draw - span);
                cx.service::<AnalysisPort<Reading>>()
                    .write(&(cx.now().units(), value));
            }
        }
    });
    port
}

/// A recorder subscriber that collects every broadcast [`Reading`].
struct Collector(Rc<RefCell<Vec<Reading>>>);

impl AnalysisWrite<Reading> for Collector {
    fn write(&self, reading: &Reading) {
        self.0.borrow_mut().push(*reading);
    }
}

/// Runs the twin **live**: a producer thread injects `readings` `gap` apart, the twin
/// parks between them, and (if `pace_scale` is set) a [`RealTimePacer`] throttles the
/// per-sample advance to wall clock. Returns the processed readings and the recorded
/// journal.
///
/// # Arguments
///
/// * `params` - The twin configuration.
/// * `readings` - The raw readings the producer will inject, in order.
/// * `gap` - Wall-clock spacing between injected readings.
/// * `pace_scale` - `Some(scale)` installs a pacer (wall-ns per sim-ns); `None` runs
///   as fast as possible.
///
/// # Returns
///
/// `(processed readings, journal)`.
pub fn run_live(
    params: SensorParams,
    readings: &[u64],
    gap: Duration,
    pace_scale: Option<f64>,
) -> (Vec<Reading>, Journal) {
    let sim = Sim::new();
    let sample = sim.alloc_event();
    let port = build_sensor(&sim, &params, sample);

    let collected = Rc::new(RefCell::new(Vec::new()));
    let collector = Rc::new(Collector(Rc::clone(&collected)));
    port.bind(&collector);

    if let Some(scale) = pace_scale {
        RealTimePacer::new(scale, params.sample_period).install(&sim);
    }

    let (recorder, sender, stop, journal) =
        journal_input(params.seed, move |cx, raw| inject(cx, sample, raw));
    attach_external_input(&sim, recorder, stop.clone());

    let readings = readings.to_vec();
    let producer = thread::spawn(move || {
        for raw in readings {
            thread::sleep(gap);
            let _ = sender.send(raw);
        }
        thread::sleep(gap);
        stop.stop();
    });

    sim.run_until(SimTime::INF);
    producer.join().expect("producer");

    let log = collected.borrow().clone();
    let recorded = journal.borrow().clone();
    (log, recorded)
}

/// Runs the twin in **replay**: the recorded `journal` drives the injections with no
/// live thread and no pacing. Restore `params.seed = journal.seed` for a faithful
/// reproduction.
///
/// # Arguments
///
/// * `params` - The twin configuration (its `seed` drives the RNG on replay).
/// * `journal` - A journal recorded by [`run_live`].
///
/// # Returns
///
/// The processed readings.
pub fn run_replay(params: SensorParams, journal: Journal) -> Vec<Reading> {
    let sim = Sim::new();
    let sample = sim.alloc_event();
    let port = build_sensor(&sim, &params, sample);

    let collected = Rc::new(RefCell::new(Vec::new()));
    let collector = Rc::new(Collector(Rc::clone(&collected)));
    port.bind(&collector);

    JournalReplayer::new(journal, move |cx, raw| inject(cx, sample, raw)).install(&sim);
    sim.run_until(SimTime::INF);

    collected.borrow().clone()
}

#[cfg(test)]
mod tests {
    use super::{Reading, SensorParams, run_live, run_replay};
    use std::time::Duration;

    /// The values of a processed-reading sequence (ignoring timestamps).
    fn values(log: &[Reading]) -> Vec<i64> {
        log.iter().map(|r| r.1).collect()
    }

    /// The live twin parks between externally-injected readings and processes each.
    #[test]
    fn live_parks_then_processes_each_reading() {
        let (log, journal) = run_live(
            SensorParams::default(),
            &[600, 700, 800],
            Duration::from_millis(5),
            None,
        );
        assert_eq!(log.len(), 3, "every injected reading was processed");
        assert_eq!(journal.records.len(), 3, "every injection was journaled");
    }

    /// Replaying the journal with the recorded seed reproduces the processed-value
    /// sequence byte-for-byte (deterministic replay).
    #[test]
    fn replay_reproduces_values() {
        let params = SensorParams::default();
        let (live, journal) = run_live(
            params,
            &[600, 700, 800, 900],
            Duration::from_millis(4),
            None,
        );
        let replay = run_replay(params, journal);
        assert_eq!(
            values(&live),
            values(&replay),
            "replay matches the live run"
        );
        assert!(!values(&live).is_empty());
    }

    /// The RNG seed is load-bearing: replaying the same journal with a different seed
    /// yields a different processed-value sequence (the noise model diverges).
    #[test]
    fn seed_is_load_bearing() {
        let params = SensorParams::default();
        let (live, journal) = run_live(
            params,
            &[600, 700, 800, 900],
            Duration::from_millis(4),
            None,
        );
        let other = SensorParams {
            seed: params.seed ^ 0xFFFF,
            ..params
        };
        let replay = run_replay(other, journal);
        assert_ne!(values(&live), values(&replay), "a different seed diverges");
    }
}
