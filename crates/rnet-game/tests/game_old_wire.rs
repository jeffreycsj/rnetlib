//! Obsolete SDK payloads must fail even inside a valid current Noise handshake.
use rnet_core::Transport;
use rnet_game::{GameEvent, GameProtocol, GameRuntime, GameRuntimeConfig, GameServerConfig};
use rnet_security::Keypair;
use rnet_transport::{ClientConfig, ClientSecurity, NetworkRuntime, RuntimeConfig};
use std::time::{Duration, Instant};

#[test]
fn old_join_and_resume_wire_never_reach_business_authorization() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        for resume in [false, true] {
            let key = Keypair::generate().unwrap();
            let client = NetworkRuntime::new_with_client_security(
                RuntimeConfig::default(),
                Some(ClientSecurity::pinned(
                    Keypair::generate().unwrap(),
                    key.public.clone(),
                )),
            )
            .unwrap();
            let server = GameRuntime::new(GameRuntimeConfig::default()).unwrap();
            let listener = server
                .listen(GameServerConfig {
                    transport,
                    bind_addr: "127.0.0.1:0".parse().unwrap(),
                    local_key: key,
                    initial_encryption: true,
                    protocol: GameProtocol::new(42, 3),
                })
                .unwrap();
            // Literal old-format fixture, not an encoder that could silently follow new wire.
            let mut payload = if resume { b"RGR3" } else { b"RGV3" }.to_vec();
            payload.extend_from_slice(&42u64.to_be_bytes());
            payload.extend_from_slice(&3u32.to_be_bytes());
            payload.extend_from_slice(&11u64.to_be_bytes());
            payload.extend_from_slice(&9u64.to_be_bytes());
            payload.extend_from_slice(&64u16.to_be_bytes());
            if resume {
                payload.extend_from_slice(&48u16.to_be_bytes());
            }
            payload.extend_from_slice(&[7; 64]);
            if resume {
                payload.extend_from_slice(&[9; 48]);
            }
            client
                .connect(ClientConfig {
                    transport,
                    bind_addr: None,
                    remote_addr: server.endpoint_local_addr(listener).unwrap(),
                    join_payload: payload,
                })
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut rejected = false;
            while !rejected && Instant::now() < deadline {
                for event in server.poll(32, Duration::from_millis(10)) {
                    assert!(
                        !matches!(
                            event,
                            GameEvent::AuthRequest { .. }
                                | GameEvent::ResumeRequest { .. }
                                | GameEvent::SessionReady { .. }
                                | GameEvent::SessionResumed { .. }
                        ),
                        "old SDK reached game policy: {event:?}"
                    );
                    rejected |= matches!(event, GameEvent::ProtocolRejected { endpoint, .. } if endpoint == listener);
                }
                client.poll_events(32, Duration::ZERO);
            }
            assert!(
                rejected,
                "old wire not rejected: {transport:?}, resume={resume}"
            );
        }
    }
}
