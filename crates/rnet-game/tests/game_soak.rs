use std::process::Command;

#[test]
fn game_soak_smoke_covers_all_transports_and_reports_completion() {
    for transport in ["tcp", "udp", "kcp"] {
        let output = Command::new(env!("CARGO_BIN_EXE_game_soak"))
            .args([transport, "1", "1", "64", "20"])
            .output()
            .expect("run game soak probe");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "{transport} soak failed: stdout={stdout} stderr={stderr}"
        );
        assert!(stdout.contains("status=completed"), "{transport}: {stdout}");
        assert!(stdout.contains(&format!("transport={transport}")));
        assert!(stdout.contains("rtt_samples="));
        assert!(stdout.contains("cpu_percent="));
    }
}
