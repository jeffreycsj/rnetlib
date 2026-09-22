//! Additive C entry points for the high-level game session API.

use crate::abi::{RnetClientSecurity, RnetLoggerV2, RnetSlice};
use crate::game_abi::{
    RnetGameBuffer, RnetGameClientConfig, RnetGameClockSync, RnetGameConfig, RnetGameEvent,
    RnetGameMetrics, RnetGameQuality, RnetGameRealtimeQueue, RnetGameServerConfig,
};
use crate::game_events;
use crate::game_registry;
use crate::observe::build_game_logger_v2;
use crate::registry::{
    copy_key, ffi_status, invalid_argument, invalid_state, parse_address, validate_struct,
    with_borrowed_slice, IN_LOG_CALLBACK,
};
use crate::runtime::runtime_config_v5;
use rnet_core::{ErrorCode, Result, RnetError, Transport};
use rnet_game::{
    GameHostClientConfig, GameProtocol, GameRuntime, GameRuntimeConfig, GameServerConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::cell::Cell;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use zeroize::Zeroizing;

#[no_mangle]
/// # Safety
/// `out` must point to writable storage for one complete configuration.
pub unsafe extern "C" fn rnet_game_config_init(out: *mut RnetGameConfig) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game config output is null");
        }
        unsafe { out.write(RnetGameConfig::default()) };
        Ok(())
    })
}

#[no_mangle]
/// Creates a game runtime with production limits and optional pinned/verifier client trust.
///
/// # Safety
/// Config, security slices, and `out` must be valid for the call. A verifier callback and its
/// user data must remain valid and thread-safe until the runtime is destroyed.
pub unsafe extern "C" fn rnet_game_runtime_create(
    config: *const RnetGameConfig,
    client: *const RnetClientSecurity,
    out: *mut u64,
) -> i32 {
    unsafe { create_game_runtime(config, client, std::ptr::null(), out) }
}

#[no_mangle]
/// Adds an asynchronous, bounded game-event logger at creation. `logger` is copied; its
/// callback user data must remain valid until destroy returns. The callback may query metrics
/// but must not stop or destroy the runtime from its dispatch thread.
///
/// # Safety
/// All pointers must be valid for this call; callback user data must remain thread-safe and
/// valid until the runtime is destroyed.
pub unsafe extern "C" fn rnet_game_runtime_create_logged(
    config: *const RnetGameConfig,
    client: *const RnetClientSecurity,
    logger: *const RnetLoggerV2,
    out: *mut u64,
) -> i32 {
    if logger.is_null() {
        return ffi_status(|| invalid_argument("game logger must be non-null"));
    }
    unsafe { create_game_runtime(config, client, logger, out) }
}

unsafe fn create_game_runtime(
    config: *const RnetGameConfig,
    client: *const RnetClientSecurity,
    logger: *const RnetLoggerV2,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("game config and output must be non-null");
        }
        let config = unsafe { *config };
        validate_struct(
            config.struct_size,
            config.abi_version,
            size_of::<RnetGameConfig>(),
        )?;
        if config.reserved != 0 || config.allow_plaintext_business_data > 1 {
            return invalid_argument("invalid game config flags");
        }
        let mut settings = GameRuntimeConfig::production()
            .allow_plaintext_business_data(config.allow_plaintext_business_data == 1);
        if !config.network_config.is_null() {
            let network = unsafe { *config.network_config };
            validate_struct(
                network.struct_size,
                network.abi_version,
                size_of::<crate::abi_config::RnetConfigV5>(),
            )?;
            if !network.logger.is_null() || !network.logger_v2.is_null() {
                return invalid_argument("game logger configuration is not yet supported");
            }
            if network.allow_legacy_unauthenticated_endpoints != 0 {
                return invalid_argument("game runtime forbids legacy unauthenticated endpoints");
            }
            settings.network = runtime_config_v5(network)?;
            settings
                .network
                .security_policy
                .allow_plaintext_business_data = config.allow_plaintext_business_data == 1;
        }
        if config.heartbeat_interval_ms != 0 || config.heartbeat_timeout_ms != 0 {
            settings = settings.with_heartbeat(
                Duration::from_millis(u64::from(config.heartbeat_interval_ms)),
                Duration::from_millis(u64::from(config.heartbeat_timeout_ms)),
            );
        }
        let ffi_identity = Arc::new(AtomicU64::new(0));
        let game_logger = unsafe { build_game_logger_v2(logger, Arc::clone(&ffi_identity)) }?;
        let runtime = if client.is_null() {
            GameRuntime::new(settings)?
        } else {
            let security = unsafe { client_security(*client) }?;
            GameRuntime::new_with_client_security(settings, security)?
        };
        let runtime = if let Some(logger) = game_logger {
            runtime.with_logger(logger)
        } else {
            runtime
        };
        let handle = game_registry::register(runtime);
        ffi_identity.store(handle, Ordering::Release);
        unsafe { out.write(handle) };
        Ok(())
    })
}

unsafe fn client_security(client: RnetClientSecurity) -> Result<ClientSecurity> {
    validate_struct(
        client.struct_size,
        client.abi_version,
        size_of::<RnetClientSecurity>(),
    )?;
    let private = Zeroizing::new(unsafe { copy_key(client.local_private_key) }?);
    let key = Keypair::from_private(private.as_slice())
        .map_err(|error| RnetError::new(ErrorCode::CryptoError, error.to_string()))?;
    if let Some(verify) = client.verify_server {
        let user_data = client.user_data as usize;
        Ok(ClientSecurity::with_verifier(
            key,
            Arc::new(move |server_key| unsafe {
                verify(user_data as *mut _, server_key.as_ptr(), server_key.len()) != 0
            }),
        ))
    } else {
        let expected = unsafe { copy_key(client.expected_server_public_key) }?;
        Ok(ClientSecurity::pinned(key, expected.to_vec()))
    }
}

#[no_mangle]
/// # Safety
/// `config`, its slices, and `out` must remain valid during the call.
pub unsafe extern "C" fn rnet_game_server_listen(
    runtime: u64,
    config: *const RnetGameServerConfig,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("game server config and output must be non-null");
        }
        let config = unsafe { *config };
        validate_struct(
            config.struct_size,
            config.abi_version,
            size_of::<RnetGameServerConfig>(),
        )?;
        if config.reserved != 0 || config.initial_encryption > 1 {
            return invalid_argument("invalid game server flags");
        }
        let private = Zeroizing::new(unsafe { copy_key(config.local_private_key) }?);
        let key = Keypair::from_private(private.as_slice())
            .map_err(|error| RnetError::new(ErrorCode::CryptoError, error.to_string()))?;
        let endpoint = game_registry::lease(runtime)?
            .runtime
            .listen(GameServerConfig {
                transport: Transport::try_from(config.transport)?,
                bind_addr: unsafe { parse_address(config.bind_host, config.bind_port) }?,
                local_key: key,
                initial_encryption: config.initial_encryption == 1,
                protocol: GameProtocol::new(config.protocol_id, config.protocol_version),
            })?;
        unsafe { out.write(endpoint) };
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// `config`, its slices, and `out` must remain valid during the call.
pub unsafe extern "C" fn rnet_game_client_connect(
    runtime: u64,
    config: *const RnetGameClientConfig,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("game client config and output must be non-null");
        }
        let config = unsafe { parse_game_client(config) }?;
        let endpoint = game_registry::lease(runtime)?
            .runtime
            .connect_host(config)?;
        unsafe { out.write(endpoint) };
        Ok(())
    })
}

#[no_mangle]
/// Performs a fresh handshake and requests server-side reauthorization of an old game session.
///
/// # Safety
/// `config`, both slices, and `out` must remain valid during the call. The ticket is a bearer
/// credential and must never be logged by the caller.
pub unsafe extern "C" fn rnet_game_client_resume_connect(
    runtime: u64,
    config: *const RnetGameClientConfig,
    old_session: u64,
    resume_ticket: RnetSlice,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("game client config and output must be non-null");
        }
        let config = unsafe { parse_game_client(config) }?;
        let endpoint = unsafe {
            with_borrowed_slice(resume_ticket, |ticket| {
                game_registry::lease(runtime)?.runtime.connect_host_resume(
                    config,
                    old_session,
                    ticket,
                )
            })
        }?;
        unsafe { out.write(endpoint) };
        Ok(())
    })
}

unsafe fn parse_game_client(config: *const RnetGameClientConfig) -> Result<GameHostClientConfig> {
    let config = unsafe { *config };
    validate_struct(
        config.struct_size,
        config.abi_version,
        size_of::<RnetGameClientConfig>(),
    )?;
    if config.reserved != 0 || config.reserved2 != 0 || config.reserved3 != 0 {
        return invalid_argument("game client reserved fields must be zero");
    }
    let host = unsafe {
        with_borrowed_slice(config.remote_host, |bytes| {
            std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|_| RnetError::new(ErrorCode::InvalidArgument, "host is not UTF-8"))
        })
    }?;
    let join_ticket =
        unsafe { with_borrowed_slice(config.join_ticket, |bytes| Ok(bytes.to_vec())) }?;
    Ok(GameHostClientConfig {
        transport: Transport::try_from(config.transport)?,
        host,
        port: config.remote_port,
        join_ticket,
        protocol: GameProtocol {
            protocol_id: config.protocol_id,
            version: config.protocol_version,
            build_id: config.build_id,
            capabilities: config.capabilities,
        },
    })
}

#[no_mangle]
/// # Safety
/// A nonempty identity must point to readable memory for the call. It is opaque and limited to
/// the game runtime's identity length; the resulting ticket is delivered as an event.
pub unsafe extern "C" fn rnet_game_issue_resume_ticket(
    runtime: u64,
    session: u64,
    identity: RnetSlice,
) -> i32 {
    ffi_status(|| {
        let entry = game_registry::lease(runtime)?;
        unsafe {
            with_borrowed_slice(identity, |bytes| {
                entry.runtime.issue_resume_ticket(session, bytes)
            })
        }
    })
}

#[no_mangle]
/// # Safety
/// `out_port` must point to writable `uint16_t` storage.
pub unsafe extern "C" fn rnet_game_endpoint_local_port(
    runtime: u64,
    endpoint: u64,
    out_port: *mut u16,
) -> i32 {
    ffi_status(|| {
        if out_port.is_null() {
            return invalid_argument("port output is null");
        }
        let port = game_registry::lease(runtime)?
            .runtime
            .endpoint_local_addr(endpoint)?
            .port();
        unsafe { out_port.write(port) };
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn rnet_game_auth_decide(runtime: u64, session: u64, accept: u32) -> i32 {
    ffi_status(|| {
        if accept > 1 {
            return invalid_argument("accept must be zero or one");
        }
        game_registry::lease(runtime)?
            .runtime
            .auth_decide(session, accept == 1)
    })
}

#[no_mangle]
/// # Safety
/// A nonempty payload must point to readable memory for the call.
pub unsafe extern "C" fn rnet_game_send(runtime: u64, session: u64, payload: RnetSlice) -> i32 {
    ffi_status(|| {
        let entry = game_registry::lease(runtime)?;
        unsafe { with_borrowed_slice(payload, |bytes| entry.runtime.send(session, bytes)) }
    })
}

#[no_mangle]
/// Replaces an older unsent snapshot with the same session and key when possible.
///
/// # Safety
/// A nonempty payload must point to readable memory for the call.
pub unsafe extern "C" fn rnet_game_send_latest(
    runtime: u64,
    session: u64,
    key: u64,
    payload: RnetSlice,
) -> i32 {
    ffi_status(|| {
        let entry = game_registry::lease(runtime)?;
        unsafe {
            with_borrowed_slice(payload, |bytes| {
                entry.runtime.send_latest(session, key, bytes)
            })
        }
    })
}

#[no_mangle]
pub extern "C" fn rnet_game_session_close(runtime: u64, session: u64) -> i32 {
    ffi_status(|| {
        game_registry::lease(runtime)?
            .runtime
            .close_session(session)
    })
}

#[no_mangle]
pub extern "C" fn rnet_game_endpoint_close(runtime: u64, endpoint: u64) -> i32 {
    ffi_status(|| {
        game_registry::lease(runtime)?
            .runtime
            .close_endpoint(endpoint)
    })
}

#[no_mangle]
pub extern "C" fn rnet_game_rekey(runtime: u64, session: u64) -> i32 {
    ffi_status(|| game_registry::lease(runtime)?.runtime.rekey(session))
}

#[no_mangle]
pub extern "C" fn rnet_game_security_set(runtime: u64, session: u64, encrypted: u32) -> i32 {
    ffi_status(|| {
        if encrypted > 1 {
            return invalid_argument("encrypted must be zero or one");
        }
        game_registry::lease(runtime)?
            .runtime
            .set_encryption(session, encrypted == 1)
    })
}

#[no_mangle]
/// # Safety
/// `out` must point to writable storage for one `RnetGameQuality`.
pub unsafe extern "C" fn rnet_game_network_quality(
    runtime: u64,
    session: u64,
    out: *mut RnetGameQuality,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game quality output is null");
        }
        let quality = game_registry::lease(runtime)?
            .runtime
            .network_quality(session)?;
        unsafe { out.write(quality.map_or_else(RnetGameQuality::default, Into::into)) };
        Ok(())
    })
}

#[no_mangle]
/// Runtime-local monotonic microseconds, not a wall-clock timestamp.
/// # Safety
/// `out` must point to writable storage for one `u64`.
pub unsafe extern "C" fn rnet_game_clock_micros(runtime: u64, out: *mut u64) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game clock output is null");
        }
        let now = game_registry::lease(runtime)?.runtime.clock_micros();
        unsafe { out.write(now) };
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// `out` must point to writable storage for one `RnetGameClockSync`.
pub unsafe extern "C" fn rnet_game_clock_sync_snapshot(
    runtime: u64,
    session: u64,
    out: *mut RnetGameClockSync,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game clock snapshot output is null");
        }
        let sample = game_registry::lease(runtime)?
            .runtime
            .clock_sync_snapshot(session)?;
        unsafe { out.write(sample.map_or_else(RnetGameClockSync::default, Into::into)) };
        Ok(())
    })
}

#[no_mangle]
/// Returns cumulative, low-cardinality game counters. Snapshots of concurrent counters are
/// not globally atomic; `logger_available` distinguishes absent logging from zero drops.
/// # Safety
/// `out` must point to writable storage for one `RnetGameMetrics`.
pub unsafe extern "C" fn rnet_game_metrics_snapshot(
    runtime: u64,
    out: *mut RnetGameMetrics,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game metrics output is null");
        }
        let entry = game_registry::lease(runtime)?;
        unsafe { out.write(RnetGameMetrics::from_runtime(&entry.runtime)) };
        Ok(())
    })
}

#[no_mangle]
/// Returns runtime-wide `LatestOnly` staging gauges and cumulative loss counters.
///
/// # Safety
/// `out` must point to writable storage for one `RnetGameRealtimeQueue`.
pub unsafe extern "C" fn rnet_game_realtime_queue_snapshot(
    runtime: u64,
    out: *mut RnetGameRealtimeQueue,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game realtime queue output is null");
        }
        let snapshot = game_registry::lease(runtime)?
            .runtime
            .realtime_queue_snapshot();
        unsafe { out.write(snapshot.into()) };
        Ok(())
    })
}

#[no_mangle]
/// Returns a borrowed, token-owned Prometheus text snapshot without player labels.
///
/// # Safety
/// `out` must point to writable storage for one `RnetGameBuffer`. Release a nonzero token with
/// `rnet_game_buffer_release` after the last read; the pointer is invalid afterward.
pub unsafe extern "C" fn rnet_game_prometheus_snapshot(
    runtime: u64,
    out: *mut RnetGameBuffer,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game Prometheus output is null");
        }
        let entry = game_registry::lease(runtime)?;
        let mut buffer = RnetGameBuffer::default();
        if let Some(view) = entry
            .buffers
            .insert(entry.runtime.prometheus_snapshot().into_bytes())
        {
            buffer.data = view.ptr;
            buffer.len = view.len;
            buffer.token = view.token;
        }
        unsafe { out.write(buffer) };
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// `events` must hold `capacity` writable events; `out_count` must be writable. Each nonzero
/// returned buffer token must be released exactly once before runtime destroy.
pub unsafe extern "C" fn rnet_game_poll_events(
    runtime: u64,
    events: *mut RnetGameEvent,
    capacity: usize,
    timeout_ms: u32,
    out_count: *mut usize,
) -> i32 {
    ffi_status(|| {
        if out_count.is_null() || (capacity != 0 && events.is_null()) {
            return invalid_argument("invalid game event output");
        }
        unsafe { out_count.write(0) };
        let entry = game_registry::lease(runtime)?;
        if capacity == 0 {
            // A zero-capacity call is a nonblocking maintenance tick. This keeps authenticated
            // controls and liveness state progressing without requiring a dummy event buffer.
            let _ = entry.runtime.poll(0, Duration::ZERO);
            return Ok(());
        }
        let polled = entry
            .runtime
            .poll(capacity, Duration::from_millis(u64::from(timeout_ms)));
        for (index, event) in polled.into_iter().enumerate() {
            let converted = game_events::encode(event, &entry.buffers);
            unsafe { events.add(index).write(converted) };
            unsafe { out_count.write(index + 1) };
        }
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn rnet_game_buffer_release(runtime: u64, token: u64) -> i32 {
    ffi_status(|| game_registry::lease(runtime)?.buffers.release(token))
}

#[no_mangle]
pub extern "C" fn rnet_game_runtime_stop(runtime: u64, drain_timeout_ms: u32) -> i32 {
    ffi_status(|| {
        if IN_LOG_CALLBACK.with(Cell::get) {
            return invalid_state("cannot stop a game runtime from a logger callback");
        }
        let entry = game_registry::lease(runtime)?;
        entry
            .runtime
            .stop(Duration::from_millis(u64::from(drain_timeout_ms)))?;
        entry.stopped.store(true, Ordering::Release);
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn rnet_game_runtime_destroy(runtime: u64) -> i32 {
    ffi_status(|| {
        if IN_LOG_CALLBACK.with(Cell::get) {
            return invalid_state("cannot destroy a game runtime from a logger callback");
        }
        game_registry::destroy(runtime)
    })
}
