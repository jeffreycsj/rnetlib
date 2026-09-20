use std::ffi::{c_void, CStr};
use std::mem::size_of;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use rnet::{
    rnet_abi_version, rnet_buffer_release, rnet_client_join, rnet_endpoint_local_port,
    rnet_endpoint_open, rnet_keypair_from_private, rnet_keypair_generate, rnet_last_error_message,
    rnet_latency_snapshot, rnet_listener_open, rnet_metrics_log_interval_set, rnet_poll_events,
    rnet_poll_events_ex, rnet_runtime_create, rnet_runtime_destroy, rnet_runtime_stop, rnet_send,
    rnet_session_auth_decide, rnet_session_close, rnet_session_send, rnet_session_send_ex,
    RnetConfig, RnetEndpointConfig, RnetEndpointMode, RnetEvent, RnetEventType, RnetJoinConfig,
    RnetKeypair, RnetLatencyMetric, RnetListenerConfig, RnetLogger, RnetSendOptions, RnetSlice,
    RNET_ABI_VERSION, RNET_E_CANCELLED, RNET_E_INVALID_ARGUMENT, RNET_E_INVALID_HANDLE,
    RNET_E_INVALID_STATE, RNET_E_NOT_SUPPORTED, RNET_E_WOULD_BLOCK, RNET_LATENCY_EVENT_QUEUE,
    RNET_LATENCY_LOGGER_CALLBACK, RNET_LATENCY_SEND_QUEUE, RNET_LOG_INFO, RNET_OK,
    RNET_TRANSPORT_KCP, RNET_TRANSPORT_TCP, RNET_TRANSPORT_UDP,
};
use rnet::{
    rnet_config_v3_init, rnet_config_v4_init, rnet_config_v5_init, rnet_latency_snapshot_v2,
    rnet_metrics_snapshot_v2, rnet_metrics_snapshot_v3, rnet_runtime_create_v3,
    rnet_runtime_create_v4, rnet_runtime_create_v5, RnetConfigV3, RnetConfigV4, RnetConfigV5,
    RnetLatencyMetricV2, RnetLoggerV2, RnetMetricsV2, RnetMetricsV3,
};

#[test]
fn v3_runtime_defaults_are_secure_and_constructible() {
    let mut config = RnetConfigV3::default();
    assert_eq!(unsafe { rnet_config_v3_init(&mut config) }, RNET_OK);
    assert_eq!(config.allow_plaintext_business_data, 0);
    assert_eq!(config.allow_legacy_unauthenticated_endpoints, 0);
    assert!(config.max_runtime_queued_bytes >= config.max_session_queued_bytes);
    let mut runtime = 0;
    assert_eq!(
        unsafe { rnet_runtime_create_v3(&config, std::ptr::null(), &mut runtime) },
        RNET_OK
    );
    assert_eq!(rnet_runtime_stop(runtime, 100), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn v4_runtime_adds_tcp_tuning_without_changing_v3() {
    let mut config = RnetConfigV4::default();
    assert_eq!(unsafe { rnet_config_v4_init(&mut config) }, RNET_OK);
    assert_eq!(config.tcp_nodelay, 1);
    config.tcp_send_buffer_bytes = 64 * 1024;
    config.tcp_recv_buffer_bytes = 64 * 1024;
    let mut runtime = 0;
    assert_eq!(
        unsafe { rnet_runtime_create_v4(&config, std::ptr::null(), &mut runtime) },
        RNET_OK
    );
    assert_eq!(rnet_runtime_stop(runtime, 100), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn v5_runtime_adds_global_resource_limits_and_v3_gauges() {
    let mut config = RnetConfigV5::default();
    assert_eq!(unsafe { rnet_config_v5_init(&mut config) }, RNET_OK);
    assert!(config.max_endpoints > 0);
    assert!(config.max_pending_handshakes > 0);
    config.max_endpoints = 4;
    config.max_pending_handshakes = 2;
    let mut runtime = 0;
    assert_eq!(
        unsafe { rnet_runtime_create_v5(&config, std::ptr::null(), &mut runtime) },
        RNET_OK
    );
    let mut metrics = RnetMetricsV3::default();
    assert_eq!(
        unsafe { rnet_metrics_snapshot_v3(runtime, &mut metrics) },
        RNET_OK
    );
    assert_eq!(metrics.current_endpoints, 0);
    assert_eq!(metrics.current_sessions, 0);
    assert_eq!(metrics.pending_handshakes, 0);
    assert_eq!(metrics.peak_pending_handshakes, 0);
    assert_eq!(metrics.admission_rejected_by_reason, [0; 8]);
    assert_eq!(rnet_runtime_stop(runtime, 100), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn v2_observability_exposes_resource_gauges_close_reasons_and_p999() {
    let config = RnetConfigV3::default();
    let mut runtime = 0;
    assert_eq!(
        unsafe { rnet_runtime_create_v3(&config, std::ptr::null(), &mut runtime) },
        RNET_OK
    );
    let mut metrics = RnetMetricsV2::default();
    assert_eq!(
        unsafe { rnet_metrics_snapshot_v2(runtime, &mut metrics) },
        RNET_OK
    );
    assert_eq!(metrics.queued_send_bytes, 0);
    assert_eq!(metrics.session_closed_by_reason, [0; 19]);
    let mut latencies = [RnetLatencyMetricV2::default(); 8];
    let mut count = 0;
    assert_eq!(
        unsafe {
            rnet_latency_snapshot_v2(
                runtime,
                latencies.as_mut_ptr(),
                latencies.len(),
                &mut count,
                1,
            )
        },
        RNET_OK
    );
    assert_eq!(count, 8);
    assert!(latencies
        .iter()
        .all(|metric| metric.p99_us <= metric.p999_us));
    assert_eq!(rnet_runtime_stop(runtime, 100), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn logger_callback_can_query_metrics_and_cannot_recursively_stop() {
    CALLBACK_METRICS_STATUS.store(i32::MAX, Ordering::SeqCst);
    CALLBACK_STOP_STATUS.store(i32::MAX, Ordering::SeqCst);
    let logger = RnetLoggerV2 {
        struct_size: size_of::<RnetLoggerV2>() as u32,
        abi_version: RNET_ABI_VERSION,
        log: Some(reentrant_log_v2),
        user_data: std::ptr::null_mut(),
        min_level: RNET_LOG_INFO,
    };
    let config = RnetConfigV4 {
        logger_v2: &logger,
        ..RnetConfigV4::default()
    };
    let mut runtime = 0;
    assert_eq!(
        unsafe { rnet_runtime_create_v4(&config, std::ptr::null(), &mut runtime) },
        RNET_OK
    );

    let (finished, done) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = finished.send(rnet_runtime_stop(runtime, 100));
    });
    assert_eq!(done.recv_timeout(Duration::from_secs(1)).unwrap(), RNET_OK);
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline && CALLBACK_STOP_STATUS.load(Ordering::SeqCst) == i32::MAX {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(CALLBACK_METRICS_STATUS.load(Ordering::SeqCst), RNET_OK);
    assert_eq!(
        CALLBACK_STOP_STATUS.load(Ordering::SeqCst),
        RNET_E_INVALID_STATE
    );
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn structured_logger_v2_receives_runtime_identity_and_event_name() {
    CAPTURED_LOGS_V2
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .clear();
    let logger = RnetLoggerV2 {
        struct_size: size_of::<RnetLoggerV2>() as u32,
        abi_version: RNET_ABI_VERSION,
        log: Some(capture_log_v2),
        user_data: std::ptr::null_mut(),
        min_level: RNET_LOG_INFO,
    };
    let config = RnetConfigV4 {
        logger_v2: &logger,
        allow_legacy_unauthenticated_endpoints: 1,
        ..RnetConfigV4::default()
    };
    let mut runtime = 0;
    assert_eq!(
        unsafe { rnet_runtime_create_v4(&config, std::ptr::null(), &mut runtime) },
        RNET_OK
    );

    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline
        && !CAPTURED_LOGS_V2
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .iter()
            .any(|record| record.event_name == "runtime_created" && record.runtime == runtime)
    {
        thread::sleep(Duration::from_millis(5));
    }
    let logs = CAPTURED_LOGS_V2.get().unwrap().lock().unwrap();
    assert!(logs.iter().any(|record| {
        record.timestamp > 1_600_000_000_000
            && record.event_name == "runtime_created"
            && record.runtime == runtime
            && record.message == "runtime created"
    }));
    drop(logs);

    let (_listener, client, _server_session, client_session) = open_tcp_pair(runtime);
    assert_eq!(
        rnet_session_close(runtime, client_session, RNET_E_CANCELLED),
        RNET_OK
    );
    let mut events = [RnetEvent::default(); 8];
    let _ = unsafe { rnet_poll_events(runtime, events.as_mut_ptr(), events.len(), 100) };
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline
        && !CAPTURED_LOGS_V2
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .iter()
            .any(|record| {
                record.event_name == "session_closed"
                    && record.endpoint == client
                    && record.session == client_session
                    && record.transport == RNET_TRANSPORT_TCP
                    && record.error_code == RNET_E_CANCELLED
            })
    {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(CAPTURED_LOGS_V2
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .iter()
        .any(|record| {
            record.event_name == "session_closed"
                && record.endpoint == client
                && record.session == client_session
                && record.transport == RNET_TRANSPORT_TCP
                && record.error_code == RNET_E_CANCELLED
        }));

    assert_eq!(rnet_runtime_stop(runtime, 100), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn synchronous_ffi_errors_preserve_diagnostic_text() {
    let status = unsafe { rnet_runtime_create(std::ptr::null(), std::ptr::null_mut()) };
    assert_eq!(status, RNET_E_INVALID_ARGUMENT);

    let text = unsafe { CStr::from_ptr(rnet_last_error_message()) }
        .to_str()
        .unwrap();
    assert!(text.contains("config and out must be non-null"));
}

use rnet::{
    rnet_client_connect_v2, rnet_runtime_create_v2, rnet_server_open_v2, rnet_session_rekey,
    rnet_session_security_set, RnetClientConfigV2, RnetClientSecurity, RnetServerConfigV2,
    RNET_SECURITY_ENCRYPTED, RNET_SECURITY_PLAINTEXT,
};

static CAPTURED_LOGS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static CALLBACK_METRICS_STATUS: AtomicI32 = AtomicI32::new(i32::MAX);
static CALLBACK_STOP_STATUS: AtomicI32 = AtomicI32::new(i32::MAX);
struct CapturedLogV2 {
    timestamp: u64,
    event_name: String,
    runtime: u64,
    endpoint: u64,
    session: u64,
    transport: u32,
    error_code: i32,
    message: String,
}
static CAPTURED_LOGS_V2: OnceLock<Mutex<Vec<CapturedLogV2>>> = OnceLock::new();

unsafe extern "C" fn capture_log(
    _user_data: *mut c_void,
    _level: u32,
    _target: *const u8,
    _target_len: usize,
    message: *const u8,
    message_len: usize,
) {
    let message = unsafe { std::slice::from_raw_parts(message, message_len) };
    CAPTURED_LOGS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(String::from_utf8_lossy(message).into_owned());
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn capture_log_v2(
    _user_data: *mut c_void,
    timestamp_unix_ms: u64,
    _level: u32,
    event_name: *const u8,
    event_name_len: usize,
    runtime: u64,
    endpoint: u64,
    session: u64,
    transport: u32,
    error_code: i32,
    _correlation_id: u64,
    message: *const u8,
    message_len: usize,
) {
    let event_name = unsafe { std::slice::from_raw_parts(event_name, event_name_len) };
    let message = unsafe { std::slice::from_raw_parts(message, message_len) };
    CAPTURED_LOGS_V2
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(CapturedLogV2 {
            timestamp: timestamp_unix_ms,
            event_name: String::from_utf8_lossy(event_name).into_owned(),
            runtime,
            endpoint,
            session,
            transport,
            error_code,
            message: String::from_utf8_lossy(message).into_owned(),
        });
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn reentrant_log_v2(
    _user_data: *mut c_void,
    _timestamp_unix_ms: u64,
    _level: u32,
    event_name: *const u8,
    event_name_len: usize,
    runtime: u64,
    _endpoint: u64,
    _session: u64,
    _transport: u32,
    _error_code: i32,
    _correlation_id: u64,
    _message: *const u8,
    _message_len: usize,
) {
    let event_name = unsafe { std::slice::from_raw_parts(event_name, event_name_len) };
    if event_name == b"runtime_stopping" {
        let mut metrics = RnetMetricsV2::default();
        CALLBACK_METRICS_STATUS.store(
            unsafe { rnet_metrics_snapshot_v2(runtime, &mut metrics) },
            Ordering::SeqCst,
        );
        CALLBACK_STOP_STATUS.store(rnet_runtime_stop(runtime, 0), Ordering::SeqCst);
    }
}

fn bytes(value: &[u8]) -> RnetSlice {
    RnetSlice {
        ptr: value.as_ptr(),
        len: value.len(),
    }
}

#[test]
fn v2_names_are_transport_neutral_and_client_security_is_runtime_scoped() {
    let mut server_key = RnetKeypair::default();
    let mut client_key = RnetKeypair::default();
    assert_eq!(unsafe { rnet_keypair_generate(&mut server_key) }, RNET_OK);
    assert_eq!(unsafe { rnet_keypair_generate(&mut client_key) }, RNET_OK);

    let base = RnetConfig::default();
    let client_security = RnetClientSecurity::pinned(
        bytes(&client_key.private_key),
        bytes(&server_key.public_key),
    );
    let mut runtime = 0;
    assert_eq!(
        unsafe { rnet_runtime_create_v2(&base, &client_security, &mut runtime) },
        RNET_OK
    );

    let host = b"127.0.0.1";
    let server = RnetServerConfigV2::new(
        RNET_TRANSPORT_TCP,
        bytes(host),
        0,
        bytes(&server_key.private_key),
        RNET_SECURITY_PLAINTEXT,
    );
    let mut listener = 0;
    assert_eq!(
        unsafe { rnet_server_open_v2(runtime, &server, &mut listener) },
        RNET_OK
    );
    let mut port = 0;
    assert_eq!(
        unsafe { rnet_endpoint_local_port(runtime, listener, &mut port) },
        RNET_OK
    );
    let client = RnetClientConfigV2::new(
        RNET_TRANSPORT_TCP,
        bytes(b"localhost"),
        port,
        bytes(b"join"),
    );
    let mut endpoint = 0;
    assert_eq!(
        unsafe { rnet_client_connect_v2(runtime, &client, &mut endpoint) },
        RNET_OK
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut server_session = 0;
    let mut server_opened = false;
    while Instant::now() < deadline && !server_opened {
        let mut events = [RnetEvent::default(); 8];
        let count = unsafe { rnet_poll_events(runtime, events.as_mut_ptr(), events.len(), 20) };
        for event in &events[..count] {
            if event.event_type == RnetEventType::AuthRequest as u32 {
                server_session = event.session;
                assert_eq!(rnet_session_auth_decide(runtime, event.session, 1), RNET_OK);
                if event.buffer_token != 0 {
                    assert_eq!(rnet_buffer_release(runtime, event.buffer_token), RNET_OK);
                }
            } else if event.event_type == RnetEventType::SessionOpened as u32
                && event.endpoint == listener
            {
                server_opened = true;
            }
        }
    }
    assert_ne!(server_session, 0);
    assert_eq!(
        rnet_session_security_set(runtime, server_session, RNET_SECURITY_ENCRYPTED),
        RNET_OK
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let mut events = [RnetEvent::default(); 8];
        let count = unsafe { rnet_poll_events(runtime, events.as_mut_ptr(), events.len(), 20) };
        let changed = events[..count]
            .iter()
            .any(|event| event.event_type == RnetEventType::SecurityChanged as u32);
        for event in &events[..count] {
            if event.buffer_token != 0 {
                assert_eq!(rnet_buffer_release(runtime, event.buffer_token), RNET_OK);
            }
        }
        if changed {
            break;
        }
    }
    assert_eq!(rnet_session_rekey(runtime, server_session), RNET_OK);
    assert_eq!(rnet_runtime_stop(runtime, 0), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

fn create_runtime() -> u64 {
    let config = RnetConfig::default();
    let mut runtime = 0;
    assert_eq!(
        unsafe { rnet_runtime_create(&config, &mut runtime) },
        RNET_OK
    );
    assert_ne!(runtime, 0);
    runtime
}

fn open_tcp_pair(runtime: u64) -> (u64, u64, u64, u64) {
    let (listener_config, _listener_host) =
        endpoint_config(RNET_TRANSPORT_TCP, RnetEndpointMode::Listener, 0, 0);
    let mut listener = 0;
    assert_eq!(
        unsafe { rnet_endpoint_open(runtime, &listener_config, &mut listener) },
        RNET_OK
    );
    let mut port = 0;
    assert_eq!(
        unsafe { rnet_endpoint_local_port(runtime, listener, &mut port) },
        RNET_OK
    );
    let (client_config, _client_host) =
        endpoint_config(RNET_TRANSPORT_TCP, RnetEndpointMode::Client, 0, port);
    let mut client = 0;
    assert_eq!(
        unsafe { rnet_endpoint_open(runtime, &client_config, &mut client) },
        RNET_OK
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut client_session = 0;
    let mut server_session = 0;
    while Instant::now() < deadline && (client_session == 0 || server_session == 0) {
        let mut events = [RnetEvent::default(); 8];
        let count = unsafe { rnet_poll_events(runtime, events.as_mut_ptr(), events.len(), 50) };
        for event in &events[..count] {
            if event.event_type == RnetEventType::SessionOpened as u32 {
                if event.endpoint == client {
                    client_session = event.session;
                } else if event.endpoint == listener {
                    server_session = event.session;
                }
            }
        }
    }
    assert_ne!(client_session, 0);
    assert_ne!(server_session, 0);
    (listener, client, server_session, client_session)
}

fn endpoint_config(
    transport: u32,
    mode: RnetEndpointMode,
    bind_port: u16,
    remote_port: u16,
) -> (RnetEndpointConfig, Vec<u8>) {
    let host = b"127.0.0.1".to_vec();
    let slice = RnetSlice {
        ptr: host.as_ptr(),
        len: host.len(),
    };
    (
        RnetEndpointConfig {
            struct_size: size_of::<RnetEndpointConfig>() as u32,
            abi_version: RNET_ABI_VERSION,
            transport,
            mode: mode as u32,
            bind_host: slice,
            bind_port,
            reserved0: 0,
            remote_host: slice,
            remote_port,
            reserved1: 0,
        },
        host,
    )
}

#[test]
fn abi_version_and_struct_validation_are_stable() {
    assert_eq!(rnet_abi_version(), RNET_ABI_VERSION);
    assert_eq!(
        unsafe { rnet_runtime_create(std::ptr::null(), std::ptr::null_mut()) },
        RNET_E_INVALID_ARGUMENT
    );

    let invalid = RnetConfig {
        struct_size: 0,
        ..RnetConfig::default()
    };
    let mut runtime = 0;
    assert_eq!(
        unsafe { rnet_runtime_create(&invalid, &mut runtime) },
        RNET_E_INVALID_ARGUMENT
    );
}

#[test]
fn keypair_can_be_restored_through_the_c_abi() {
    let mut generated = RnetKeypair::default();
    assert_eq!(unsafe { rnet_keypair_generate(&mut generated) }, RNET_OK);

    let mut restored = RnetKeypair::default();
    assert_eq!(
        unsafe { rnet_keypair_from_private(bytes(&generated.private_key), &mut restored) },
        RNET_OK
    );
    assert_eq!(restored.private_key, generated.private_key);
    assert_eq!(restored.public_key, generated.public_key);

    assert_eq!(
        unsafe { rnet_keypair_from_private(bytes(&generated.private_key[..31]), &mut restored) },
        RNET_E_INVALID_ARGUMENT
    );
}

#[test]
fn runtime_must_stop_before_destroy_and_stop_is_idempotent() {
    let runtime = create_runtime();
    assert_eq!(rnet_runtime_destroy(runtime), RNET_E_INVALID_STATE);
    assert_eq!(rnet_runtime_stop(runtime, 0), RNET_OK);
    assert_eq!(rnet_runtime_stop(runtime, 0), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn poll_ex_distinguishes_timeout_from_api_errors() {
    let runtime = create_runtime();
    let mut count = usize::MAX;
    assert_eq!(
        unsafe { rnet_poll_events_ex(runtime, std::ptr::null_mut(), 0, 0, &mut count) },
        RNET_OK
    );
    assert_eq!(count, 0);
    assert_eq!(
        unsafe { rnet_poll_events_ex(u64::MAX, std::ptr::null_mut(), 0, 0, &mut count) },
        RNET_E_INVALID_HANDLE
    );
    assert_eq!(
        unsafe { rnet_poll_events_ex(runtime, std::ptr::null_mut(), 1, 0, &mut count) },
        RNET_E_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { rnet_poll_events_ex(runtime, std::ptr::null_mut(), 0, 0, std::ptr::null_mut()) },
        RNET_E_INVALID_ARGUMENT
    );
    assert_eq!(rnet_runtime_stop(runtime, 0), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn simplified_send_zeros_legacy_fields_and_ex_only_sets_correlation() {
    let runtime = create_runtime();
    let (_listener, _client, server_session, client_session) = open_tcp_pair(runtime);

    assert_eq!(
        unsafe { rnet_session_send(runtime, client_session, 71, bytes(b"simple")) },
        RNET_OK
    );
    let options = RnetSendOptions {
        struct_size: size_of::<RnetSendOptions>() as u32,
        abi_version: RNET_ABI_VERSION,
        correlation_id: 991,
        flags: 0,
        reserved: 0,
    };
    assert_eq!(
        unsafe { rnet_session_send_ex(runtime, client_session, 72, bytes(b"advanced"), &options) },
        RNET_OK
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut received = Vec::new();
    while Instant::now() < deadline && received.len() < 2 {
        let mut events = [RnetEvent::default(); 8];
        let mut count = 0;
        assert_eq!(
            unsafe {
                rnet_poll_events_ex(runtime, events.as_mut_ptr(), events.len(), 50, &mut count)
            },
            RNET_OK
        );
        for event in events[..count].iter().copied() {
            if event.event_type == RnetEventType::Message as u32 {
                assert_eq!(event.session, server_session);
                received.push((event.msg_type, event.stream_id, event.request_id));
                assert_eq!(rnet_buffer_release(runtime, event.buffer_token), RNET_OK);
            }
        }
    }
    assert_eq!(received, vec![(71, 0, 0), (72, 0, 991)]);

    assert_eq!(rnet_runtime_stop(runtime, 0), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn closing_one_session_is_observable_and_does_not_close_the_endpoint() {
    let runtime = create_runtime();
    let (_listener, _client, server_session, client_session) = open_tcp_pair(runtime);
    assert_eq!(
        rnet_session_close(runtime, client_session, RNET_E_CANCELLED),
        RNET_OK
    );
    assert_eq!(
        rnet_session_close(runtime, client_session, RNET_E_CANCELLED),
        RNET_E_INVALID_HANDLE
    );
    let mut metrics = RnetMetricsV3::default();
    assert_eq!(
        unsafe { rnet_metrics_snapshot_v3(runtime, &mut metrics) },
        RNET_OK
    );
    assert_eq!(metrics.session_closed_by_reason[18], 1);

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut closed = false;
    while Instant::now() < deadline && !closed {
        let mut events = [RnetEvent::default(); 8];
        let count = unsafe { rnet_poll_events(runtime, events.as_mut_ptr(), events.len(), 50) };
        closed = events[..count].iter().any(|event| {
            event.event_type == RnetEventType::SessionClosed as u32
                && event.session == client_session
                && event.status == RNET_E_CANCELLED
        });
        for event in &events[..count] {
            if event.buffer_token != 0 {
                assert_eq!(rnet_buffer_release(runtime, event.buffer_token), RNET_OK);
            }
        }
    }
    assert!(closed);
    assert_eq!(
        unsafe { rnet_session_send(runtime, client_session, 1, bytes(b"closed")) },
        RNET_E_INVALID_HANDLE
    );

    let peer_send = unsafe { rnet_session_send(runtime, server_session, 1, bytes(b"late")) };
    assert!(peer_send == RNET_OK || peer_send == RNET_E_INVALID_HANDLE);
    let mut events = [RnetEvent::default(); 8];
    let mut count = 0;
    assert_eq!(
        unsafe { rnet_poll_events_ex(runtime, events.as_mut_ptr(), events.len(), 100, &mut count) },
        RNET_OK
    );
    assert!(!events[..count].iter().any(|event| {
        event.event_type == RnetEventType::Message as u32 && event.session == client_session
    }));
    for event in &events[..count] {
        if event.buffer_token != 0 {
            assert_eq!(rnet_buffer_release(runtime, event.buffer_token), RNET_OK);
        }
    }

    assert_eq!(rnet_runtime_stop(runtime, 0), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn latency_snapshot_reports_send_and_event_queue_percentiles() {
    let runtime = create_runtime();
    let (_listener, _client, _server_session, client_session) = open_tcp_pair(runtime);
    assert_eq!(
        unsafe { rnet_session_send(runtime, client_session, 88, bytes(b"latency")) },
        RNET_OK
    );
    thread::sleep(Duration::from_millis(2));
    let mut events = [RnetEvent::default(); 8];
    let mut event_count = 0;
    assert_eq!(
        unsafe {
            rnet_poll_events_ex(
                runtime,
                events.as_mut_ptr(),
                events.len(),
                100,
                &mut event_count,
            )
        },
        RNET_OK
    );
    for event in &events[..event_count] {
        if event.buffer_token != 0 {
            assert_eq!(rnet_buffer_release(runtime, event.buffer_token), RNET_OK);
        }
    }

    let mut required = 0;
    assert_eq!(
        unsafe { rnet_latency_snapshot(runtime, std::ptr::null_mut(), 0, &mut required) },
        RNET_OK
    );
    assert!(required >= 2);
    let mut metrics = vec![RnetLatencyMetric::default(); required];
    assert_eq!(
        unsafe {
            rnet_latency_snapshot(runtime, metrics.as_mut_ptr(), metrics.len(), &mut required)
        },
        RNET_OK
    );
    let send = metrics
        .iter()
        .find(|metric| metric.kind == RNET_LATENCY_SEND_QUEUE)
        .expect("send queue latency");
    let events = metrics
        .iter()
        .find(|metric| metric.kind == RNET_LATENCY_EVENT_QUEUE)
        .expect("event queue latency");
    assert!(send.sample_count >= 1);
    assert!(events.sample_count >= 1);
    assert!(send.p50_us <= send.p90_us && send.p90_us <= send.p95_us);
    assert!(send.p95_us <= send.p99_us && send.p99_us <= send.max_us.next_power_of_two());

    let mut v2_count = 0;
    assert_eq!(
        unsafe { rnet_latency_snapshot_v2(runtime, std::ptr::null_mut(), 0, &mut v2_count, 1) },
        RNET_OK
    );
    let mut v2_metrics = vec![RnetLatencyMetricV2::default(); v2_count];
    assert_eq!(
        unsafe {
            rnet_latency_snapshot_v2(
                runtime,
                v2_metrics.as_mut_ptr(),
                v2_metrics.len(),
                &mut v2_count,
                1,
            )
        },
        RNET_OK
    );
    assert!(v2_metrics
        .iter()
        .find(|metric| metric.kind == RNET_LATENCY_SEND_QUEUE)
        .is_some_and(|metric| metric.sample_count >= 1));

    assert_eq!(rnet_runtime_stop(runtime, 0), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn configured_latency_summary_is_emitted_through_the_async_logger() {
    CAPTURED_LOGS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .clear();
    let logger = RnetLogger {
        struct_size: size_of::<RnetLogger>() as u32,
        abi_version: RNET_ABI_VERSION,
        log: Some(capture_log),
        user_data: std::ptr::null_mut(),
        min_level: RNET_LOG_INFO,
    };
    let config = RnetConfig {
        logger: &logger,
        ..RnetConfig::default()
    };
    let mut runtime = 0;
    assert_eq!(
        unsafe { rnet_runtime_create(&config, &mut runtime) },
        RNET_OK
    );
    assert_eq!(rnet_metrics_log_interval_set(runtime, 1), RNET_OK);
    thread::sleep(Duration::from_millis(2));
    let mut count = 0;
    assert_eq!(
        unsafe { rnet_poll_events_ex(runtime, std::ptr::null_mut(), 0, 0, &mut count) },
        RNET_OK
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline
        && !CAPTURED_LOGS
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .iter()
            .any(|message| message.contains("latency_summary") && message.contains("p99_us"))
    {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(CAPTURED_LOGS
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .iter()
        .any(|message| message.contains("latency_summary") && message.contains("p99_us")));
    let mut metrics = [RnetLatencyMetric::default(); 8];
    let mut metric_count = 0;
    assert_eq!(
        unsafe {
            rnet_latency_snapshot(
                runtime,
                metrics.as_mut_ptr(),
                metrics.len(),
                &mut metric_count,
            )
        },
        RNET_OK
    );
    let callback = metrics[..metric_count]
        .iter()
        .find(|metric| metric.kind == RNET_LATENCY_LOGGER_CALLBACK)
        .expect("logger callback latency");
    assert!(callback.sample_count >= 1);
    assert_eq!(rnet_runtime_stop(runtime, 0), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn kcp_is_explicitly_reported_as_not_supported() {
    let runtime = create_runtime();
    let (config, _host) = endpoint_config(RNET_TRANSPORT_KCP, RnetEndpointMode::Datagram, 0, 0);
    let mut endpoint = 0;
    assert_eq!(
        unsafe { rnet_endpoint_open(runtime, &config, &mut endpoint) },
        RNET_E_NOT_SUPPORTED
    );
    assert_eq!(rnet_runtime_stop(runtime, 0), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn tcp_message_crosses_ffi_and_buffer_requires_one_release() {
    let runtime = create_runtime();
    let (listener_config, _listener_host) =
        endpoint_config(RNET_TRANSPORT_TCP, RnetEndpointMode::Listener, 0, 0);
    let mut listener = 0;
    assert_eq!(
        unsafe { rnet_endpoint_open(runtime, &listener_config, &mut listener) },
        RNET_OK
    );
    let mut port = 0;
    assert_eq!(
        unsafe { rnet_endpoint_local_port(runtime, listener, &mut port) },
        RNET_OK
    );
    assert_ne!(port, 0);

    let (client_config, _client_host) =
        endpoint_config(RNET_TRANSPORT_TCP, RnetEndpointMode::Client, 0, port);
    let mut client = 0;
    assert_eq!(
        unsafe { rnet_endpoint_open(runtime, &client_config, &mut client) },
        RNET_OK
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut client_session = 0;
    let mut server_session = 0;
    while Instant::now() < deadline && (client_session == 0 || server_session == 0) {
        let mut events = [RnetEvent::default(); 8];
        let count = unsafe { rnet_poll_events(runtime, events.as_mut_ptr(), events.len(), 50) };
        for event in &events[..count] {
            if event.event_type == RnetEventType::SessionOpened as u32 {
                if event.endpoint == client {
                    client_session = event.session;
                } else if event.endpoint == listener {
                    server_session = event.session;
                }
            }
        }
    }
    assert_ne!(client_session, 0);
    assert_ne!(server_session, 0);

    let payload = b"ffi-payload";
    assert_eq!(
        unsafe {
            rnet_send(
                runtime,
                client_session,
                33,
                4,
                RnetSlice {
                    ptr: payload.as_ptr(),
                    len: payload.len(),
                },
                55,
            )
        },
        RNET_OK
    );

    let mut message = None;
    while Instant::now() < deadline && message.is_none() {
        let mut events = [RnetEvent::default(); 8];
        let count = unsafe { rnet_poll_events(runtime, events.as_mut_ptr(), events.len(), 50) };
        message = events[..count]
            .iter()
            .copied()
            .find(|event| event.event_type == RnetEventType::Message as u32);
    }
    let message = message.expect("message event");
    assert_eq!(message.session, server_session);
    assert_eq!(message.msg_type, 33);
    assert_eq!(message.stream_id, 4);
    assert_eq!(message.request_id, 55);
    assert_ne!(message.buffer_token, 0);
    let received = unsafe { std::slice::from_raw_parts(message.data, message.data_len) };
    assert_eq!(received, payload);
    assert_eq!(rnet_buffer_release(runtime, message.buffer_token), RNET_OK);
    assert_ne!(rnet_buffer_release(runtime, message.buffer_token), RNET_OK);

    assert_eq!(rnet_runtime_stop(runtime, 10), RNET_OK);
    thread::sleep(Duration::from_millis(10));
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn secure_listener_and_client_join_cross_the_c_abi() {
    let runtime = create_runtime();
    let mut server_key = RnetKeypair::default();
    let mut client_key = RnetKeypair::default();
    assert_eq!(unsafe { rnet_keypair_generate(&mut server_key) }, RNET_OK);
    assert_eq!(unsafe { rnet_keypair_generate(&mut client_key) }, RNET_OK);
    let host = b"127.0.0.1";
    let listener_config = RnetListenerConfig {
        struct_size: size_of::<RnetListenerConfig>() as u32,
        abi_version: RNET_ABI_VERSION,
        transport: RNET_TRANSPORT_TCP,
        reserved0: 0,
        bind_host: bytes(host),
        bind_port: 0,
        reserved1: 0,
        local_private_key: bytes(&server_key.private_key),
    };
    let mut listener = 0;
    assert_eq!(
        unsafe { rnet_listener_open(runtime, &listener_config, &mut listener) },
        RNET_OK
    );
    let mut port = 0;
    assert_eq!(
        unsafe { rnet_endpoint_local_port(runtime, listener, &mut port) },
        RNET_OK
    );
    let ticket = b"ffi-ticket";
    let join_config = RnetJoinConfig {
        struct_size: size_of::<RnetJoinConfig>() as u32,
        abi_version: RNET_ABI_VERSION,
        transport: RNET_TRANSPORT_TCP,
        reserved0: 0,
        remote_host: bytes(host),
        remote_port: port,
        reserved1: 0,
        local_private_key: bytes(&client_key.private_key),
        expected_server_public_key: bytes(&server_key.public_key),
        join_payload: bytes(ticket),
    };
    let mut client = 0;
    assert_eq!(
        unsafe { rnet_client_join(runtime, &join_config, &mut client) },
        RNET_OK
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut auth_session = 0;
    while Instant::now() < deadline && auth_session == 0 {
        let mut events = [RnetEvent::default(); 8];
        let count = unsafe { rnet_poll_events(runtime, events.as_mut_ptr(), events.len(), 50) };
        for event in &events[..count] {
            if event.event_type == RnetEventType::AuthRequest as u32 {
                assert_eq!(event.data_len, 32 + ticket.len());
                let data = unsafe { std::slice::from_raw_parts(event.data, event.data_len) };
                assert_eq!(&data[32..], ticket);
                assert_eq!(rnet_buffer_release(runtime, event.buffer_token), RNET_OK);
                auth_session = event.session;
            }
        }
    }
    assert_ne!(auth_session, 0);
    assert_eq!(rnet_session_auth_decide(runtime, auth_session, 1), RNET_OK);

    let mut client_session = 0;
    while Instant::now() < deadline && client_session == 0 {
        let mut events = [RnetEvent::default(); 8];
        let count = unsafe { rnet_poll_events(runtime, events.as_mut_ptr(), events.len(), 50) };
        for event in &events[..count] {
            if event.event_type == RnetEventType::SessionOpened as u32 && event.endpoint == client {
                client_session = event.session;
            }
        }
    }
    assert_ne!(client_session, 0);
    assert_eq!(
        unsafe { rnet_send(runtime, client_session, 3, 0, bytes(b"secure"), 7) },
        RNET_OK
    );
    assert_eq!(rnet_runtime_stop(runtime, 0), RNET_OK);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}

#[test]
fn secure_udp_and_kcp_join_cross_the_c_abi() {
    for transport in [RNET_TRANSPORT_UDP, RNET_TRANSPORT_KCP] {
        let runtime = create_runtime();
        let mut server_key = RnetKeypair::default();
        let mut client_key = RnetKeypair::default();
        assert_eq!(unsafe { rnet_keypair_generate(&mut server_key) }, RNET_OK);
        assert_eq!(unsafe { rnet_keypair_generate(&mut client_key) }, RNET_OK);
        let host = b"127.0.0.1";
        let listener_config = RnetListenerConfig {
            struct_size: size_of::<RnetListenerConfig>() as u32,
            abi_version: RNET_ABI_VERSION,
            transport,
            reserved0: 0,
            bind_host: bytes(host),
            bind_port: 0,
            reserved1: 0,
            local_private_key: bytes(&server_key.private_key),
        };
        let mut listener = 0;
        assert_eq!(
            unsafe { rnet_listener_open(runtime, &listener_config, &mut listener) },
            RNET_OK
        );
        let mut port = 0;
        assert_eq!(
            unsafe { rnet_endpoint_local_port(runtime, listener, &mut port) },
            RNET_OK
        );
        let join_config = RnetJoinConfig {
            struct_size: size_of::<RnetJoinConfig>() as u32,
            abi_version: RNET_ABI_VERSION,
            transport,
            reserved0: 0,
            remote_host: bytes(host),
            remote_port: port,
            reserved1: 0,
            local_private_key: bytes(&client_key.private_key),
            expected_server_public_key: bytes(&server_key.public_key),
            join_payload: bytes(b"datagram-ticket"),
        };
        let mut client = 0;
        assert_eq!(
            unsafe { rnet_client_join(runtime, &join_config, &mut client) },
            RNET_OK
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut client_session = 0;
        while Instant::now() < deadline && client_session == 0 {
            let mut events = [RnetEvent::default(); 16];
            let count = unsafe { rnet_poll_events(runtime, events.as_mut_ptr(), events.len(), 50) };
            for event in &events[..count] {
                if event.event_type == RnetEventType::AuthRequest as u32 {
                    if event.buffer_token != 0 {
                        assert_eq!(rnet_buffer_release(runtime, event.buffer_token), RNET_OK);
                    }
                    assert_eq!(rnet_session_auth_decide(runtime, event.session, 1), RNET_OK);
                } else if event.event_type == RnetEventType::SessionOpened as u32
                    && event.endpoint == client
                {
                    client_session = event.session;
                }
            }
        }
        assert_ne!(client_session, 0, "transport {transport} did not join");
        assert_eq!(rnet_runtime_stop(runtime, 0), RNET_OK);
        assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
    }
}

#[test]
fn destroy_rejects_concurrent_poll_and_succeeds_after_it_returns() {
    let runtime = create_runtime();
    assert_eq!(rnet_runtime_stop(runtime, 0), RNET_OK);
    let mut events = [RnetEvent::default(); 8];
    while unsafe { rnet_poll_events(runtime, events.as_mut_ptr(), events.len(), 0) } != 0 {}

    let poller = thread::spawn(move || {
        let mut event = RnetEvent::default();
        unsafe { rnet_poll_events(runtime, &mut event, 1, 200) }
    });
    thread::sleep(Duration::from_millis(20));
    assert_eq!(rnet_runtime_destroy(runtime), RNET_E_WOULD_BLOCK);
    assert_eq!(poller.join().unwrap(), 0);
    assert_eq!(rnet_runtime_destroy(runtime), RNET_OK);
}
