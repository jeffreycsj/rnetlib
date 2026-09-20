use rnet::{
    rnet_game_auth_decide, rnet_game_buffer_release, rnet_game_client_connect,
    rnet_game_client_resume_connect, rnet_game_config_init, rnet_game_endpoint_local_port,
    rnet_game_issue_resume_ticket, rnet_game_network_quality, rnet_game_poll_events,
    rnet_game_prometheus_snapshot, rnet_game_runtime_create, rnet_game_runtime_destroy,
    rnet_game_runtime_stop, rnet_game_send, rnet_game_send_latest, rnet_game_server_listen,
    rnet_game_session_close, rnet_runtime_create_v5, rnet_runtime_destroy, rnet_runtime_stop,
    RnetClientSecurity, RnetConfigV5, RnetGameBuffer, RnetGameClientConfig, RnetGameConfig,
    RnetGameEvent, RnetGameQuality, RnetGameServerConfig, RnetSlice, RNET_ABI_VERSION,
    RNET_E_HANDSHAKE_REQUIRED, RNET_E_INVALID_ARGUMENT, RNET_E_INVALID_HANDLE,
    RNET_E_INVALID_STATE, RNET_E_WOULD_BLOCK, RNET_GAME_AUTH_REQUEST, RNET_GAME_MESSAGE,
    RNET_GAME_RESUME_REQUEST, RNET_GAME_RESUME_TICKET, RNET_GAME_SESSION_READY,
    RNET_GAME_SESSION_RESUMED, RNET_OK, RNET_TRANSPORT_TCP,
};
use rnet_security::Keypair;
use std::mem::size_of;
use std::time::{Duration, Instant};

fn slice(bytes: &[u8]) -> RnetSlice {
    RnetSlice {
        ptr: bytes.as_ptr(),
        len: bytes.len(),
    }
}

#[test]
fn game_and_transport_runtime_handles_are_not_interchangeable() {
    let mut game = 0;
    assert_eq!(
        unsafe {
            rnet_game_runtime_create(&RnetGameConfig::default(), std::ptr::null(), &mut game)
        },
        RNET_OK
    );
    let mut transport = 0;
    assert_eq!(
        unsafe {
            rnet_runtime_create_v5(&RnetConfigV5::default(), std::ptr::null(), &mut transport)
        },
        RNET_OK
    );
    assert_eq!(rnet_game_runtime_stop(transport, 0), RNET_E_INVALID_HANDLE);
    assert_eq!(rnet_runtime_stop(game, 0), RNET_E_INVALID_HANDLE);
    assert_eq!(rnet_game_runtime_stop(game, 0), RNET_OK);
    assert_eq!(rnet_game_runtime_destroy(game), RNET_OK);
    assert_eq!(rnet_game_runtime_stop(game, 0), RNET_E_INVALID_HANDLE);
    assert_eq!(rnet_runtime_stop(transport, 0), RNET_OK);
    assert_eq!(rnet_runtime_destroy(transport), RNET_OK);
}

#[test]
fn game_runtime_accepts_production_network_capacity_tuning() {
    let network = RnetConfigV5 {
        max_endpoints: 1,
        ..RnetConfigV5::default()
    };
    let mut rejected = network;
    rejected.allow_legacy_unauthenticated_endpoints = 1;
    let rejected_config = RnetGameConfig {
        network_config: &rejected,
        ..RnetGameConfig::default()
    };
    let mut runtime = 0;
    assert_eq!(
        unsafe { rnet_game_runtime_create(&rejected_config, std::ptr::null(), &mut runtime) },
        RNET_E_INVALID_ARGUMENT
    );
    let config = RnetGameConfig {
        network_config: &network,
        ..RnetGameConfig::default()
    };
    assert_eq!(
        unsafe { rnet_game_runtime_create(&config, std::ptr::null(), &mut runtime) },
        RNET_OK
    );
    let key = Keypair::generate().unwrap();
    let server = RnetGameServerConfig {
        struct_size: size_of::<RnetGameServerConfig>() as u32,
        abi_version: RNET_ABI_VERSION,
        transport: RNET_TRANSPORT_TCP,
        initial_encryption: 1,
        bind_host: slice(b"127.0.0.1"),
        bind_port: 0,
        reserved: 0,
        local_private_key: slice(&key.private),
        protocol_id: 5,
        protocol_version: 1,
    };
    let mut first = 0;
    assert_eq!(
        unsafe { rnet_game_server_listen(runtime, &server, &mut first) },
        RNET_OK
    );
    let mut second = 0;
    assert_eq!(
        unsafe { rnet_game_server_listen(runtime, &server, &mut second) },
        RNET_E_WOULD_BLOCK
    );
    assert_eq!(rnet_game_runtime_stop(runtime, 0), RNET_OK);
    assert_eq!(rnet_game_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn game_c_api_waits_for_authorization_and_sends_opaque_payload() {
    let server_key = Keypair::generate().unwrap();
    let client_key = Keypair::generate().unwrap();
    let client_security =
        RnetClientSecurity::pinned(slice(&client_key.private), slice(&server_key.public));
    let mut config = RnetGameConfig::default();
    assert_eq!(unsafe { rnet_game_config_init(&mut config) }, RNET_OK);
    assert_eq!(config.abi_version, RNET_ABI_VERSION);
    assert_eq!(config.allow_plaintext_business_data, 0);
    config.heartbeat_interval_ms = 20;
    config.heartbeat_timeout_ms = 1_000;
    let mut runtime = 0;
    assert_eq!(
        unsafe { rnet_game_runtime_create(&config, &client_security, &mut runtime) },
        RNET_OK
    );
    let server = RnetGameServerConfig {
        struct_size: size_of::<RnetGameServerConfig>() as u32,
        abi_version: RNET_ABI_VERSION,
        transport: RNET_TRANSPORT_TCP,
        initial_encryption: 1,
        bind_host: slice(b"127.0.0.1"),
        bind_port: 0,
        reserved: 0,
        local_private_key: slice(&server_key.private),
        protocol_id: 31,
        protocol_version: 1,
    };
    let mut listener = 0;
    assert_eq!(
        unsafe { rnet_game_server_listen(runtime, &server, &mut listener) },
        RNET_OK
    );
    let mut port = 0;
    assert_eq!(
        unsafe { rnet_game_endpoint_local_port(runtime, listener, &mut port) },
        RNET_OK
    );
    let client = RnetGameClientConfig {
        struct_size: size_of::<RnetGameClientConfig>() as u32,
        abi_version: RNET_ABI_VERSION,
        transport: RNET_TRANSPORT_TCP,
        reserved: 0,
        remote_host: slice(b"localhost"),
        remote_port: port,
        reserved2: 0,
        join_ticket: slice(b"login-proof"),
        protocol_id: 31,
        protocol_version: 1,
        reserved3: 0,
        build_id: 0,
        capabilities: 0,
    };
    let mut client_endpoint = 0;
    assert_eq!(
        unsafe { rnet_game_client_connect(runtime, &client, &mut client_endpoint) },
        RNET_OK
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut server_session = None;
    let mut client_session = None;
    let mut received = false;
    let mut message_token = 0;
    while Instant::now() < deadline && !received {
        let mut events = [RnetGameEvent::default(); 16];
        let mut count = 0;
        assert_eq!(
            unsafe {
                rnet_game_poll_events(runtime, events.as_mut_ptr(), events.len(), 10, &mut count)
            },
            RNET_OK
        );
        for event in events.iter().take(count) {
            match event.event_type {
                RNET_GAME_AUTH_REQUEST => {
                    assert_eq!(event.endpoint, listener);
                    assert_eq!(
                        unsafe { std::slice::from_raw_parts(event.data, event.data_len) },
                        b"login-proof"
                    );
                    assert_eq!(
                        unsafe { rnet_game_send(runtime, event.session, slice(b"too-early")) },
                        RNET_E_HANDSHAKE_REQUIRED
                    );
                    assert_eq!(rnet_game_auth_decide(runtime, event.session, 1), RNET_OK);
                }
                RNET_GAME_SESSION_READY if event.endpoint == listener => {
                    server_session = Some(event.session);
                }
                RNET_GAME_SESSION_READY if event.endpoint == client_endpoint => {
                    client_session = Some(event.session);
                    assert_eq!(
                        unsafe { rnet_game_send(runtime, event.session, slice(b"opaque-payload")) },
                        RNET_OK
                    );
                }
                RNET_GAME_MESSAGE => {
                    assert_eq!(Some(event.session), server_session);
                    assert_eq!(
                        unsafe { std::slice::from_raw_parts(event.data, event.data_len) },
                        b"opaque-payload"
                    );
                    received = true;
                    message_token = event.buffer_token;
                }
                _ => {}
            }
            if event.buffer_token != 0 && event.buffer_token != message_token {
                assert_eq!(
                    rnet_game_buffer_release(runtime, event.buffer_token),
                    RNET_OK
                );
            }
        }
    }
    assert!(client_session.is_some());
    assert!(received);
    assert_eq!(
        unsafe { rnet_game_send_latest(runtime, client_session.unwrap(), 7, slice(b"latest")) },
        RNET_OK
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut latest_received = false;
    while Instant::now() < deadline && !latest_received {
        let mut events = [RnetGameEvent::default(); 16];
        let mut count = 0;
        assert_eq!(
            unsafe {
                rnet_game_poll_events(runtime, events.as_mut_ptr(), events.len(), 10, &mut count)
            },
            RNET_OK
        );
        for event in events.iter().take(count) {
            if event.event_type == RNET_GAME_MESSAGE && event.session == server_session.unwrap() {
                assert_eq!(
                    unsafe { std::slice::from_raw_parts(event.data, event.data_len) },
                    b"latest"
                );
                latest_received = true;
            }
            if event.buffer_token != 0 {
                assert_eq!(
                    rnet_game_buffer_release(runtime, event.buffer_token),
                    RNET_OK
                );
            }
        }
    }
    assert!(latest_received);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut quality = RnetGameQuality::default();
    while Instant::now() < deadline && quality.available == 0 {
        assert_eq!(
            unsafe { rnet_game_network_quality(runtime, server_session.unwrap(), &mut quality) },
            RNET_OK
        );
        if quality.available == 0 {
            let mut events = [RnetGameEvent::default(); 16];
            let mut count = 0;
            assert_eq!(
                unsafe {
                    rnet_game_poll_events(
                        runtime,
                        events.as_mut_ptr(),
                        events.len(),
                        10,
                        &mut count,
                    )
                },
                RNET_OK
            );
            for event in events.iter().take(count) {
                if event.buffer_token != 0 {
                    assert_eq!(
                        rnet_game_buffer_release(runtime, event.buffer_token),
                        RNET_OK
                    );
                }
            }
        }
    }
    assert_eq!(quality.available, 1);
    assert!(quality.samples >= 1);
    assert_eq!(quality.basis, 1);
    assert_eq!(quality.has_udp_loss, 0);
    assert_eq!(quality.has_kcp_retransmissions, 0);
    let mut prometheus = RnetGameBuffer::default();
    assert_eq!(
        unsafe { rnet_game_prometheus_snapshot(runtime, &mut prometheus) },
        RNET_OK
    );
    let text = unsafe { std::slice::from_raw_parts(prometheus.data, prometheus.len) };
    assert!(std::str::from_utf8(text)
        .unwrap()
        .contains("rnet_game_heartbeat_"));
    assert_eq!(rnet_game_buffer_release(runtime, prometheus.token), RNET_OK);
    assert_eq!(
        unsafe {
            rnet_game_issue_resume_ticket(runtime, server_session.unwrap(), slice(b"player-31"))
        },
        RNET_OK
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut ticket = None;
    while Instant::now() < deadline && ticket.is_none() {
        let mut events = [RnetGameEvent::default(); 16];
        let mut count = 0;
        assert_eq!(
            unsafe {
                rnet_game_poll_events(runtime, events.as_mut_ptr(), events.len(), 10, &mut count)
            },
            RNET_OK
        );
        for event in events.iter().take(count) {
            if event.event_type == RNET_GAME_RESUME_TICKET {
                assert_eq!(event.endpoint, client_endpoint);
                ticket = Some(
                    unsafe { std::slice::from_raw_parts(event.data, event.data_len) }.to_vec(),
                );
            }
            if event.buffer_token != 0 {
                assert_eq!(
                    rnet_game_buffer_release(runtime, event.buffer_token),
                    RNET_OK
                );
            }
        }
    }
    let ticket = ticket.expect("resume ticket");
    assert_eq!(
        rnet_game_session_close(runtime, client_session.unwrap()),
        RNET_OK
    );
    let mut resumed_endpoint = 0;
    assert_eq!(
        unsafe {
            rnet_game_client_resume_connect(
                runtime,
                &client,
                client_session.unwrap(),
                slice(&ticket),
                &mut resumed_endpoint,
            )
        },
        RNET_OK
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut server_mapping = false;
    let mut client_mapping = false;
    while Instant::now() < deadline && (!server_mapping || !client_mapping) {
        let mut events = [RnetGameEvent::default(); 16];
        let mut count = 0;
        assert_eq!(
            unsafe {
                rnet_game_poll_events(runtime, events.as_mut_ptr(), events.len(), 10, &mut count)
            },
            RNET_OK
        );
        for event in events.iter().take(count) {
            if event.event_type == RNET_GAME_RESUME_REQUEST {
                assert_eq!(event.related_session, server_session.unwrap());
                assert_eq!(
                    unsafe { std::slice::from_raw_parts(event.data, event.data_len) },
                    b"player-31"
                );
                assert_eq!(
                    unsafe { std::slice::from_raw_parts(event.aux_data, event.aux_data_len) },
                    b"login-proof"
                );
                assert_eq!(rnet_game_auth_decide(runtime, event.session, 1), RNET_OK);
            } else if event.event_type == RNET_GAME_SESSION_RESUMED {
                if event.endpoint == listener {
                    assert_eq!(event.related_session, server_session.unwrap());
                    server_mapping = true;
                } else if event.endpoint == resumed_endpoint {
                    assert_eq!(event.related_session, client_session.unwrap());
                    client_mapping = true;
                }
            }
            if event.buffer_token != 0 {
                assert_eq!(
                    rnet_game_buffer_release(runtime, event.buffer_token),
                    RNET_OK
                );
            }
            if event.aux_buffer_token != 0 {
                assert_eq!(
                    rnet_game_buffer_release(runtime, event.aux_buffer_token),
                    RNET_OK
                );
            }
        }
    }
    assert!(server_mapping && client_mapping);
    assert_eq!(rnet_game_runtime_stop(runtime, 100), RNET_OK);
    assert_eq!(rnet_game_runtime_destroy(runtime), RNET_E_INVALID_STATE);
    assert_eq!(rnet_game_buffer_release(runtime, message_token), RNET_OK);
    assert_eq!(rnet_game_runtime_destroy(runtime), RNET_OK);
}
