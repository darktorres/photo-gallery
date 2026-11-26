//! Laws of the reader-count controller (`pg_jobs::tuner`), over generated windows: the target stays within its bounds
//! whatever the windows say; a stream that improves with the count lowers it only by a probe; the same stream gives the
//! same decisions; and a stationary stream changes the count a bounded number of times.

use std::num::NonZeroUsize;
use std::time::Duration;

use pg_jobs::tuner::{Decision, Prior, Reason, Sample, Tuner, TunerConfig};
use proptest::prelude::*;

/// A whole number as a float (the tests' own conversion).
fn float(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// A read-bound window of 3 s reading `mb_s` MB/s, 20 reads of `latency_ms` each.
fn window(mb_s: f64, latency_ms: f64) -> Sample {
    Sample {
        elapsed: Duration::from_secs(3),
        reads: 20,
        bytes: pg_testkit::whole_units(mb_s * 3e6),
        disk_ns: pg_testkit::whole_units(latency_ms * 20e6),
        work_wait_ns: 6_000_000_000,
        workers: 4,
        active_readers: 2.0,
        files: 20,
        ..Sample::default()
    }
}

fn tuner(start: usize) -> Tuner {
    let start = NonZeroUsize::new(start.max(1)).unwrap_or(NonZeroUsize::MIN);
    Tuner::new(TunerConfig::default(), Prior { start, expected: None, fallback: start })
}

/// The decisions of `tuner` over windows that `curve` gives at its target, one per noise value.
fn run(tuner: &mut Tuner, noise: &[f64], curve: impl Fn(usize) -> f64) -> Vec<Decision> {
    noise.iter().map(|n| tuner.update(&window(curve(tuner.target().get()) * (1.0 + n), 5.0))).collect()
}

/// Any window at all: lengths, reads, waits and pauses as they come.
fn any_sample() -> impl Strategy<Value = Sample> {
    (0u64..10_000, 0u64..200, 0u64..1 << 40, 0u64..1 << 34, 0u64..1 << 34, 0u64..1 << 34, 0u64..100, 0u64..1 << 33)
        .prop_map(|(ms, reads, bytes, disk, wait, capacity, paused, oldest)| Sample {
            elapsed: Duration::from_millis(ms),
            paused: Duration::from_millis(if paused < 90 { 0 } else { paused }),
            reads,
            bytes,
            disk_ns: disk,
            oldest_read_ns: oldest,
            work_wait_ns: wait,
            pool_capacity_wait_ns: capacity,
            workers: 4,
            active_readers: 3.0,
            files: reads,
            ..Sample::default()
        })
}

proptest! {
    #[test]
    fn the_target_stays_within_one_and_the_most(start in 1usize..40, samples in prop::collection::vec(any_sample(), 0..300)) {
        let mut t = tuner(start);
        let max = t.config().max.get();
        prop_assert!(t.target().get() >= 1 && t.target().get() <= max);
        for s in &samples {
            let d = t.update(s);
            prop_assert!(d.target.get() >= 1 && d.target.get() <= max, "{d:?}");
        }
    }

    // Throughput as the cube of the count: every count up reads clearly more (16 over 15 by 21%, more than the 12% gain
    // after the 2% noise), every count down clearly less. The count
    // is lowered only to measure a lower one (an exploration's step, a probe), never backed off, and wherever the tuner
    // settles or reverts a probe it is at the most readers.
    #[test]
    fn a_stream_that_improves_with_the_count_lowers_it_only_to_measure(
        start in 1usize..17,
        noise in prop::collection::vec(-0.02f64..0.02, 1..300),
    ) {
        let mut t = tuner(start);
        let max = t.config().max.get();
        let mut before = t.target().get();
        for d in run(&mut t, &noise, |r| float(r) * float(r) * float(r)) {
            if d.target.get() < before {
                prop_assert!(
                    matches!(d.reason, Reason::Explore | Reason::Probe),
                    "lowered from {} to {} by {:?}", before, d.target, d.reason
                );
            }
            if matches!(d.reason, Reason::Settled | Reason::ProbeReverted) {
                prop_assert_eq!(d.target.get(), max, "{:?}", d);
            }
            before = d.target.get();
        }
    }

    #[test]
    fn the_same_stream_gives_the_same_decisions(
        start in 1usize..17,
        noise in prop::collection::vec(-0.3f64..0.3, 1..200),
        knee in 1usize..17,
    ) {
        let curve = |r: usize| 50.0 * float(r.min(knee));
        let first = run(&mut tuner(start), &noise, curve);
        let second = run(&mut tuner(start), &noise, curve);
        prop_assert_eq!(first, second);
    }

    // A flat stream (a count does not matter) settles, and then only probes: at most 30 changes in 200 windows.
    #[test]
    fn a_stationary_stream_changes_the_count_a_bounded_number_of_times(
        start in 1usize..17,
        noise in prop::collection::vec(-0.02f64..0.02, 200),
    ) {
        let decisions = run(&mut tuner(start), &noise, |_| 100.0);
        let changes = decisions.windows(2).filter(|w| w.first().map(|d| d.target) != w.get(1).map(|d| d.target)).count();
        prop_assert!(changes <= 30, "{} changes", changes);
    }
}
