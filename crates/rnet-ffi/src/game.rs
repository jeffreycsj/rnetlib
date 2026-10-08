//! C entry points for the high-level game session API.

use crate::abi::{RnetClientSecurity, RnetLogger, RnetSlice};
use crate::game_abi::{
    RnetGameSendOptions, RNET_GAME_PRIORITY_CRITICAL, RNET_GAME_PRIORITY_HIGH,
    RNET_GAME_PRIORITY_LOW, RNET_GAME_PRIORITY_NORMAL,
};
use crate::game_config_abi::{RnetGameClientConfig, RnetGameConfig, RnetGameServerConfig};
use crate::game_registry;
use crate::observe::build_game_logger;
use crate::registry::{
    copy_key, ffi_status, invalid_argument, parse_address, validate_struct, with_borrowed_slice,
};
use crate::runtime::runtime_config;
use rnet_core::{ErrorCode, Result, RnetError, Transport};
use rnet_game::{
    GameHostClientConfig, GamePriority, GameProtocol, GameRuntime, GameRuntimeConfig,
    GameSendOptions, GameServerConfig, RealtimeQueueConfig, ScheduledQueueConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
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
/// Creates a game runtime with queue budgets, optional client identity and structured logging.
///
/// # Safety
/// Configuration and output pointers must be valid for the call. Logger/verifier callbacks
/// and their user data must remain thread-safe and valid until runtime destruction.
pub unsafe extern "C" fn rnet_game_runtime_create(
    config: *const RnetGameConfig,
    client: *const RnetClientSecurity,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("game config and output must be non-null");
        }
        let config = unsafe { crate::registry::read_config(config) }?;
        let defaults = GameRuntimeConfig::production();
        let realtime = RealtimeQueueConfig {
            max_queued_bytes: usize_override(
                config.realtime_max_queued_bytes,
                defaults.realtime_queue.max_queued_bytes,
            )?,
            max_session_queued_bytes: usize_override(
                config.realtime_max_session_queued_bytes,
                defaults.realtime_queue.max_session_queued_bytes,
            )?,
            max_keys_per_session: usize_override(
                config.realtime_max_keys_per_session,
                defaults.realtime_queue.max_keys_per_session,
            )?,
            flush_batch: usize_override(
                config.realtime_flush_batch,
                defaults.realtime_queue.flush_batch,
            )?,
        };
        let scheduled = ScheduledQueueConfig {
            max_queued_bytes: usize_override(
                config.scheduled_max_queued_bytes,
                defaults.scheduled_queue.max_queued_bytes,
            )?,
            max_session_queued_bytes: usize_override(
                config.scheduled_max_session_queued_bytes,
                defaults.scheduled_queue.max_session_queued_bytes,
            )?,
            max_queued_messages: usize_override(
                config.scheduled_max_queued_messages,
                defaults.scheduled_queue.max_queued_messages,
            )?,
            flush_batch: usize_override(
                config.scheduled_flush_batch,
                defaults.scheduled_queue.flush_batch,
            )?,
        };
        unsafe {
            create_game_runtime_from_config(
                config,
                (realtime, scheduled),
                client,
                config.logger,
                out,
            )
        }
    })
}

fn usize_override(value: u64, default: usize) -> Result<usize> {
    if value == 0 {
        return Ok(default);
    }
    usize::try_from(value)
        .map_err(|_| RnetError::new(ErrorCode::InvalidArgument, "game queue limit exceeds usize"))
}

unsafe fn create_game_runtime_from_config(
    config: RnetGameConfig,
    queues: (RealtimeQueueConfig, ScheduledQueueConfig),
    client: *const RnetClientSecurity,
    logger: *const RnetLogger,
    out: *mut u64,
) -> Result<()> {
    if config.reserved != 0 || config.allow_plaintext_business_data > 1 {
        return invalid_argument("invalid game config flags");
    }
    let mut settings = GameRuntimeConfig::production()
        .allow_plaintext_business_data(config.allow_plaintext_business_data == 1);
    settings = settings
        .with_realtime_queue(queues.0)
        .with_scheduled_queue(queues.1);
    if !config.network_config.is_null() {
        let network = unsafe { crate::registry::read_config(config.network_config) }?;
        if !network.logger.is_null() {
            return invalid_argument("use game config.logger instead of network config.logger");
        }
        if network.allow_legacy_unauthenticated_endpoints != 0 {
            return invalid_argument("game runtime forbids legacy unauthenticated endpoints");
        }
        settings.network = runtime_config(network)?;
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
    let game_logger = unsafe { build_game_logger(logger, Arc::clone(&ffi_identity)) }?;
    let runtime = if client.is_null() {
        GameRuntime::new(settings)?
    } else {
        let security = unsafe { client_security(crate::registry::read_config(client)?) }?;
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
        let config = unsafe { crate::registry::read_config(config) }?;
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
    let config = unsafe { crate::registry::read_config(config) }?;
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
/// Sends opaque business bytes with optional network metadata.
///
/// # Safety
/// A nonempty payload and `options` must point to readable memory for the call.
pub unsafe extern "C" fn rnet_game_send_ex(
    runtime: u64,
    session: u64,
    payload: RnetSlice,
    options: *const RnetGameSendOptions,
) -> i32 {
    ffi_status(|| {
        let options = unsafe { crate::registry::read_config(options) }?;
        if options.has_sequence > 1 || options.has_tick > 1 {
            return invalid_argument("game metadata presence flags must be zero or one");
        }
        let priority = match options.priority {
            RNET_GAME_PRIORITY_LOW => GamePriority::Low,
            RNET_GAME_PRIORITY_NORMAL => GamePriority::Normal,
            RNET_GAME_PRIORITY_HIGH => GamePriority::High,
            RNET_GAME_PRIORITY_CRITICAL => GamePriority::Critical,
            _ => return invalid_argument("unknown game send priority"),
        };
        let entry = game_registry::lease(runtime)?;
        unsafe {
            with_borrowed_slice(payload, |bytes| {
                entry.runtime.send_with_options(
                    session,
                    bytes,
                    GameSendOptions {
                        sequence: (options.has_sequence != 0).then_some(options.sequence),
                        tick: (options.has_tick != 0).then_some(options.tick),
                        correlation_id: options.correlation_id,
                        priority,
                        expires_after: (options.expiry_ms != 0)
                            .then(|| Duration::from_millis(options.expiry_ms)),
                    },
                )
            })
        }
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
