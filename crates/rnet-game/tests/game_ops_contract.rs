//! Deployment rules must reference samples emitted by the real facade, not invented metrics.
use rnet_game::{GameRuntime, GameRuntimeConfig};
use std::{collections::HashSet, fs, path::Path, time::Duration};

#[test]
fn operations_metric_selectors_exist_in_actual_runtime_snapshot() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let contract = fs::read_to_string(root.join("examples/operations/metric-selectors.txt"))
        .expect("operations examples must supply their metric contract");
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).unwrap();
    let text = runtime.prometheus_snapshot();
    let samples: HashSet<_> = text
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    let mut required = 0;
    for selector in contract
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        assert!(
            samples.contains(selector),
            "missing runtime sample: {selector}"
        );
        required += 1;
    }
    assert!(
        required >= 20,
        "contract must cover all operational categories"
    );
    assert!(!text.contains("session=\"") && !text.contains("player=\""));
    runtime.stop(Duration::ZERO).unwrap();
}
