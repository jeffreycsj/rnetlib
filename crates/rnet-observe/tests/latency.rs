use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rnet_observe::LatencyHistogram;

#[test]
fn empty_histogram_has_zero_percentiles() {
    let snapshot = LatencyHistogram::default().snapshot();
    assert_eq!(snapshot.sample_count, 0);
    assert_eq!(snapshot.p50_us, 0);
    assert_eq!(snapshot.p90_us, 0);
    assert_eq!(snapshot.p95_us, 0);
    assert_eq!(snapshot.p99_us, 0);
    assert_eq!(snapshot.max_us, 0);
}

#[test]
fn percentiles_are_monotonic_and_max_is_exact() {
    let histogram = LatencyHistogram::default();
    for micros in [1, 10, 100, 1_000, 10_000] {
        histogram.record(Duration::from_micros(micros));
    }
    let snapshot = histogram.snapshot();
    assert_eq!(snapshot.sample_count, 5);
    assert!(snapshot.p50_us <= snapshot.p90_us);
    assert!(snapshot.p90_us <= snapshot.p95_us);
    assert!(snapshot.p95_us <= snapshot.p99_us);
    assert!(snapshot.p99_us <= snapshot.max_us);
    assert_eq!(snapshot.max_us, 10_000);
}

#[test]
fn concurrent_recording_keeps_every_sample() {
    let histogram = Arc::new(LatencyHistogram::default());
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let histogram = Arc::clone(&histogram);
            thread::spawn(move || {
                for micros in 1..=1_000 {
                    histogram.record(Duration::from_micros(micros));
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let snapshot = histogram.snapshot();
    assert_eq!(snapshot.sample_count, 4_000);
    assert_eq!(snapshot.max_us, 1_000);
}

#[test]
fn very_large_values_are_bounded_without_wrapping() {
    let histogram = LatencyHistogram::default();
    histogram.record(Duration::MAX);
    let snapshot = histogram.snapshot();
    assert_eq!(snapshot.sample_count, 1);
    assert_eq!(snapshot.max_us, u64::MAX);
    assert_eq!(snapshot.p99_us, u64::MAX);
}

#[test]
fn rotating_snapshot_reports_p999_and_clears_only_the_window() {
    let histogram = LatencyHistogram::default();
    for micros in 1..=1_000 {
        histogram.record(Duration::from_micros(micros));
    }

    let window = histogram.snapshot_and_reset();

    assert_eq!(window.sample_count, 1_000);
    assert!(window.p99_us <= window.p999_us);
    assert!(window.p999_us <= window.max_us.next_power_of_two());
    assert_eq!(histogram.snapshot_and_reset().sample_count, 0);
}
