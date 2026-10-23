//! Fault injection uses a real game runtime and child client, not mocked transport.
use super::*;
use rnet_core::Transport;
use rnet_game::{GameEvent, GameProtocol, GameRuntime, GameRuntimeConfig, GameServerConfig};
use rnet_security::Keypair;
use std::os::unix::fs::{symlink, PermissionsExt};

#[test]
fn unsafe_or_malformed_credentials_fail_before_join() {
    if !resources_available() {
        return;
    }
    let fixture = Fixture::new();
    let keys = fixture.keys();
    let token = keys.join("token");
    fs::set_permissions(&token, fs::Permissions::from_mode(0o644)).unwrap();
    let output = client(&keys, "tcp", "127.0.0.1:9", "1").output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("permissions"));
    fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&token, [0_u8; 31]).unwrap();
    let output = client(&keys, "tcp", "127.0.0.1:9", "1").output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("32_raw_bytes"));
    fs::rename(&token, keys.join("original-token")).unwrap();
    symlink(keys.join("original-token"), &token).unwrap();
    let output = client(&keys, "tcp", "127.0.0.1:9", "1").output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("symlink"));
}

#[test]
fn client_reports_backpressure_without_counting_rejection_as_delivery() {
    if !resources_available() {
        return;
    }
    let fixture = Fixture::new();
    let keys = fixture.keys();
    let mut server = Process::start(&mut server(&keys, "tcp"), &fixture.0, "server");
    let address = server.address();
    let mut command = client(&keys, "tcp", &address, "17");
    command.args(["--queue-messages", "1"]);
    let mut child = Process::start(&mut command, &fixture.0, "pressure");
    let text = child.wait(true);
    let line = text
        .lines()
        .find(|line| line.starts_with("status=completed role=client "))
        .unwrap();
    assert!(count(line, "would_block") > 0, "{text}");
    assert!(count(line, "rejected") > 0, "{text}");
    assert_eq!(
        count(line, "offered"),
        count(line, "admitted") + count(line, "rejected")
    );
    assert_eq!(count(line, "admitted"), count(line, "received"));
    server.wait(true);
}

#[test]
fn duplicate_generator_identity_is_rejected_even_after_finishing() {
    if !resources_available() {
        return;
    }
    let fixture = Fixture::new();
    let keys = fixture.keys();
    let mut server = Process::start(&mut server(&keys, "tcp"), &fixture.0, "server");
    let address = server.address();
    Process::start(
        &mut client(&keys, "tcp", &address, "17"),
        &fixture.0,
        "first",
    )
    .wait(true);
    let text = Process::start(
        &mut client(&keys, "tcp", &address, "17"),
        &fixture.0,
        "duplicate",
    )
    .wait(false);
    assert!(text.contains("join"), "{text}");
    let text = server.wait(true);
    let line = text
        .lines()
        .find(|line| line.starts_with("status=completed role=server "))
        .unwrap();
    assert!(count(line, "auth_rejected") > 0);
    assert_eq!(count(line, "finished_flows"), 2);
}

#[test]
fn bad_echo_blackhole_and_missing_finish_ack_fail_with_partial_evidence() {
    if !resources_available() {
        return;
    }
    let fixture = Fixture::new();
    let keys = fixture.keys();
    for (name, transport, fault, reason) in [
        ("corrupt", Transport::Tcp, 0, "probe_payload"),
        (
            "tcp-blackhole",
            Transport::Tcp,
            1,
            "client_reliable_echo_drain_incomplete",
        ),
        (
            "kcp-blackhole",
            Transport::Kcp,
            1,
            "client_reliable_echo_drain_incomplete",
        ),
        ("udp-no-ack", Transport::Udp, 2, "client_finish_ack_timeout"),
    ] {
        let runtime = GameRuntime::new(GameRuntimeConfig::production()).unwrap();
        let endpoint = runtime
            .listen(GameServerConfig {
                transport,
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                local_key: Keypair::from_private(&fs::read(keys.join("server.key")).unwrap())
                    .unwrap(),
                initial_encryption: true,
                protocol: GameProtocol::new(0x524e_5052_4f42_4501, 1, 1),
            })
            .unwrap();
        let address = runtime.endpoint_local_addr(endpoint).unwrap().to_string();
        let transport_name = match transport {
            Transport::Tcp => "tcp",
            Transport::Udp => "udp",
            Transport::Kcp => "kcp",
        };
        let mut child = Process::start(
            &mut client(&keys, transport_name, &address, "19"),
            &fixture.0,
            name,
        );
        let deadline = Instant::now() + Duration::from_secs(8);
        while child.child.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "fault client hung: {}",
                child.text()
            );
            for event in runtime.poll(256, Duration::from_millis(1)) {
                match event {
                    GameEvent::AuthRequest { session, .. } => {
                        runtime.auth_decide(session, true).unwrap()
                    }
                    GameEvent::Message(message) if fault == 0 => {
                        let mut payload = message.payload.to_vec();
                        payload[32] ^= 1;
                        runtime.send(message.session, &payload).unwrap();
                    }
                    GameEvent::Message(message)
                        if fault == 2 && message.payload.starts_with(b"RNPD") =>
                    {
                        runtime.send(message.session, &message.payload).unwrap();
                    }
                    _ => {}
                }
            }
        }
        let text = child.wait(false);
        assert!(text.contains(reason), "{name}: {text}");
        let line = text
            .lines()
            .find(|line| line.starts_with("status=failed role=client "))
            .unwrap();
        assert!(count(line, "admitted") > 0, "{text}");
        assert_eq!(
            count(line, "offered"),
            count(line, "admitted") + count(line, "rejected")
        );
        if fault == 1 {
            assert_eq!(count(line, "received"), 0);
            assert!(count(line, "pending_messages") > 0);
        }
        runtime.stop(Duration::ZERO).unwrap();
    }
}
