//! Real sockets verify old logical frames cannot bypass the authenticated receive path.

use crate::state::OutboundKind;
use crate::{
    ClientConfig, ClientSecurity, NetworkRuntime, RuntimeConfig, SecurityMode, SendOptions,
    ServerConfig,
};
use bytes::Bytes;
use rnet_core::{ErrorCode, EventType, Handle, Transport};
use rnet_security::Keypair;
use std::time::{Duration, Instant};

fn connect(runtime: &NetworkRuntime, listener: Handle, transport: Transport) -> (Handle, Handle) {
    let endpoint = runtime
        .connect(ClientConfig {
            transport,
            bind_addr: None,
            remote_addr: runtime.endpoint_local_addr(listener).unwrap(),
            join_payload: Vec::new(),
        })
        .unwrap();
    let mut server = 0;
    let mut client = 0;
    let deadline = Instant::now() + Duration::from_secs(3);
    while server == 0 || client == 0 {
        assert!(Instant::now() < deadline, "authenticated join timed out");
        for event in runtime.poll_events(32, Duration::from_millis(5)) {
            match event.event_type {
                EventType::AuthRequest => runtime.auth_decide(event.session, true).unwrap(),
                EventType::SessionOpened if event.endpoint == endpoint => client = event.session,
                EventType::SessionOpened if event.endpoint == listener => server = event.session,
                EventType::EndpointError | EventType::JoinFailed => {
                    panic!("join failure: {event:?}")
                }
                _ => {}
            }
        }
    }
    (server, client)
}

#[test]
fn old_frames_are_rejected_without_disabling_the_listener_on_all_transports_and_modes() {
    // A valid old-layout empty message, including zero legacy routing fields. This must not
    // become an empty/current message even after the peer has passed Noise and authorization.
    let old = Bytes::from_static(&[
        0x52, 0x4e, 0x45, 0x54, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ]);
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        for mode in [SecurityMode::Encrypted, SecurityMode::Plaintext] {
            let key = Keypair::generate().unwrap();
            let mut config = RuntimeConfig::production();
            config.worker_threads = 1;
            config.security_policy.allow_plaintext_business_data = true;
            let runtime = NetworkRuntime::new_with_client_security(
                config,
                Some(ClientSecurity::pinned(
                    Keypair::generate().unwrap(),
                    key.public.clone(),
                )),
            )
            .unwrap();
            let listener = runtime
                .listen(ServerConfig {
                    transport,
                    bind_addr: "127.0.0.1:0".parse().unwrap(),
                    local_key: key,
                    initial_security: mode,
                })
                .unwrap();
            let (server, client) = connect(&runtime, listener, transport);
            let errors_before = runtime.metrics_snapshot().protocol_errors;
            runtime
                .enqueue_encoded(client, old.clone(), OutboundKind::Data)
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut closed = false;
            while if transport == Transport::Tcp {
                !closed
            } else {
                runtime.metrics_snapshot().protocol_errors == errors_before
            } {
                assert!(
                    Instant::now() < deadline,
                    "old frame not rejected: {transport:?}/{mode:?}"
                );
                for event in runtime.poll_events(32, Duration::from_millis(5)) {
                    assert_ne!(
                        event.event_type,
                        EventType::EndpointError,
                        "listener must survive"
                    );
                    assert_ne!(
                        event.event_type,
                        EventType::Message,
                        "old frame leaked to caller"
                    );
                    if event.event_type == EventType::SessionClosed && event.session == server {
                        assert_eq!(
                            transport,
                            Transport::Tcp,
                            "datagram corruption is dropped, not fatal"
                        );
                        assert_eq!(event.status, ErrorCode::ProtocolError);
                        assert!(String::from_utf8_lossy(&event.data)
                            .contains("unsupported frame version"));
                        closed = true;
                    }
                }
            }
            let (new_server, new_client) = if transport == Transport::Tcp {
                assert_eq!(
                    runtime.send(server, b"stale").unwrap_err().code(),
                    ErrorCode::InvalidHandle
                );
                // A stream framing error is fatal only to that session, not its listener.
                connect(&runtime, listener, transport)
            } else {
                // Datagram parsers reject corrupted frames without letting untrusted traffic
                // terminate an established peer. Even the original session must remain usable.
                (server, client)
            };
            runtime
                .send_with_options(
                    new_client,
                    b"current",
                    SendOptions {
                        correlation_id: u64::MAX,
                    },
                )
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                assert!(Instant::now() < deadline, "current frame not delivered");
                let events = runtime.poll_events(32, Duration::from_millis(5));
                if let Some(event) = events
                    .iter()
                    .find(|e| e.event_type == EventType::Message && e.session == new_server)
                {
                    assert_eq!(event.data, b"current");
                    assert_eq!(event.correlation_id, u64::MAX);
                    assert_eq!(event.integrity_verified, mode == SecurityMode::Encrypted);
                    break;
                }
            }
            if transport != Transport::Tcp {
                connect(&runtime, listener, transport);
            }
        }
    }
}
