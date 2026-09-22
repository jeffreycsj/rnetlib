use std::process::Command;

const MIN_AVAILABLE_KIB: u64 = 15 * 1024 * 1024;

fn available_memory_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn is_memory_guard_refusal(stderr: &str) -> bool {
    stderr
        .trim()
        .strip_prefix("Error: \"soak stopped: MemAvailable ")
        .and_then(|value| value.strip_suffix(" KiB is below 15 GiB\""))
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|available| available < MIN_AVAILABLE_KIB)
}

fn report_counter(line: &str, name: &str) -> u64 {
    line.split_whitespace()
        .find_map(|field| field.strip_prefix(name))
        .unwrap_or_else(|| panic!("missing report field {name}: {line}"))
        .parse()
        .unwrap_or_else(|_| panic!("invalid report field {name}: {line}"))
}

#[test]
fn only_a_real_memory_guard_refusal_is_skippable() {
    assert!(is_memory_guard_refusal(
        "Error: \"soak stopped: MemAvailable 14813880 KiB is below 15 GiB\"\n"
    ));
    assert!(!is_memory_guard_refusal(
        "Error: \"soak stopped: MemAvailable 15728640 KiB is below 15 GiB\"\n"
    ));
    assert!(!is_memory_guard_refusal("Error: \"join failed\"\n"));
}

#[test]
fn game_soak_smoke_covers_all_transports_when_memory_available() {
    // The probe's memory floor protects the machine. On a smaller CI runner this smoke test
    // has no valid execution environment; resource rejection is not a network failure.
    if let Some(available) = available_memory_kib().filter(|value| *value < MIN_AVAILABLE_KIB) {
        eprintln!(
            "SKIPPED game soak smoke: MemAvailable {available} KiB is below the 15 GiB guard"
        );
        return;
    }
    for transport in ["tcp", "udp", "kcp"] {
        let output = Command::new(env!("CARGO_BIN_EXE_game_soak"))
            .args([transport, "1", "1", "64", "20"])
            .output()
            .expect("run game soak probe");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() && is_memory_guard_refusal(&stderr) {
            eprintln!("SKIPPED remaining game soak smoke: {transport} hit the 15 GiB guard");
            return;
        }
        assert!(
            output.status.success(),
            "{transport} soak failed: stdout={stdout} stderr={stderr}"
        );
        assert!(stdout.contains("status=completed"), "{transport}: {stdout}");
        assert!(stdout.contains(&format!("transport={transport}")));
        assert!(stdout.contains("rtt_samples="));
        assert!(stdout.contains("rtt_p999_us="));
        assert!(stdout.contains("server_received="));
        assert!(stdout.contains("client_received="));
        assert!(stdout.contains("echo_queued="));
        assert!(stdout.contains("scheduled_queue_messages="));
        assert!(stdout.contains("scheduled_queue_delay_p999_us="));
        assert!(stdout.contains("realtime_queue_messages="));
        assert!(stdout.contains("send_queue_p999_us="));
        assert!(stdout.contains("event_queue_p999_us="));
        assert!(stdout.contains("closed_sessions_total="));
        assert!(stdout.contains("cpu_percent="));
        let completed = stdout
            .lines()
            .find(|line| line.starts_with("status=completed"))
            .expect("completed report line");
        let sent = report_counter(completed, "origin_sent=");
        assert_eq!(report_counter(completed, "server_received="), sent);
        assert_eq!(report_counter(completed, "echo_queued="), sent);
        assert_eq!(report_counter(completed, "client_received="), sent);
        assert_eq!(report_counter(completed, "closed_sessions_total="), 0);
    }
}
