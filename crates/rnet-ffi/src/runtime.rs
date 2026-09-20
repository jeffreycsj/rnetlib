use crate::abi::RNET_ABI_VERSION;
use crate::abi::{RnetClientSecurity, RnetConfig, RnetConfigV3};
use crate::abi_config::{RnetConfigV4, RnetConfigV5};
use crate::observe::log_entry;
use crate::observe::{build_logger, build_logger_v2};
use crate::registry::copy_key;
use crate::registry::ffi_status;
use crate::registry::invalid_argument;
use crate::registry::invalid_state;
use crate::registry::runtime_entry;
use crate::registry::runtimes;
use crate::registry::validate_struct;
use crate::registry::RuntimeEntry;
use crate::registry::IN_LOG_CALLBACK;
use rnet_core::BufferStore;
use rnet_core::ErrorCode;
use rnet_core::Lifecycle;
use rnet_core::RnetError;
use rnet_observe::LogLevel;
use rnet_security::Keypair;
use rnet_transport::RuntimeConfig;
use rnet_transport::{ClientSecurity, NetworkRuntime};
use std::cell::Cell;
use std::mem::size_of;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

#[no_mangle]
pub extern "C" fn rnet_abi_version() -> u32 {
    RNET_ABI_VERSION
}

#[no_mangle]
/// Initializes a caller-owned configuration structure.
///
/// # Safety
/// `config` must be null or point to writable memory for one `RnetConfig`.
pub unsafe extern "C" fn rnet_config_init(config: *mut RnetConfig) -> i32 {
    ffi_status(|| {
        if config.is_null() {
            return invalid_argument("config is null");
        }
        unsafe { config.write(RnetConfig::default()) };
        Ok(())
    })
}

#[no_mangle]
/// Initializes the secure-by-default production configuration.
///
/// # Safety
/// `config` must point to writable memory for one `RnetConfigV3`.
pub unsafe extern "C" fn rnet_config_v3_init(config: *mut RnetConfigV3) -> i32 {
    ffi_status(|| {
        if config.is_null() {
            return invalid_argument("config is null");
        }
        unsafe { config.write(RnetConfigV3::default()) };
        Ok(())
    })
}

#[no_mangle]
/// Initializes the socket-tunable, secure-by-default production configuration.
///
/// # Safety
/// `config` must point to writable memory for one `RnetConfigV4`.
pub unsafe extern "C" fn rnet_config_v4_init(config: *mut RnetConfigV4) -> i32 {
    ffi_status(|| {
        if config.is_null() {
            return invalid_argument("config is null");
        }
        unsafe { config.write(RnetConfigV4::default()) };
        Ok(())
    })
}

#[no_mangle]
/// Initializes a V5 configuration with production-safe resource limits.
///
/// # Safety
/// `config` must point to writable memory for one `RnetConfigV5`.
pub unsafe extern "C" fn rnet_config_v5_init(config: *mut RnetConfigV5) -> i32 {
    ffi_status(|| {
        if config.is_null() {
            return invalid_argument("config must be non-null");
        }
        unsafe { config.write(RnetConfigV5::default()) };
        Ok(())
    })
}

#[no_mangle]
/// Creates a runtime from a versioned configuration.
///
/// # Safety
/// `config` and `out` must point to readable/writable instances for the call. Any logger
/// pointer in `config` must remain valid for the runtime lifetime.
pub unsafe extern "C" fn rnet_runtime_create(config: *const RnetConfig, out: *mut u64) -> i32 {
    unsafe { create_runtime(config, std::ptr::null(), out) }
}

#[no_mangle]
/// Creates a runtime with optional runtime-scoped client identity and peer verification.
///
/// # Safety
/// All non-null pointers must remain valid for the call. A verification callback and its
/// `user_data` must remain valid until the runtime is destroyed and must be thread-safe.
pub unsafe extern "C" fn rnet_runtime_create_v2(
    config: *const RnetConfig,
    client: *const RnetClientSecurity,
    out: *mut u64,
) -> i32 {
    unsafe { create_runtime(config, client, out) }
}

#[no_mangle]
/// Creates a runtime using the production configuration and optional client identity.
///
/// # Safety
/// All non-null pointers must be valid for the call. Client verification callbacks and logger
/// callbacks must remain valid and thread-safe until the runtime is destroyed.
pub unsafe extern "C" fn rnet_runtime_create_v3(
    config: *const RnetConfigV3,
    client: *const RnetClientSecurity,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("config and out must be non-null");
        }
        let config = unsafe { *config };
        validate_struct(
            config.struct_size,
            config.abi_version,
            size_of::<RnetConfigV3>(),
        )?;
        let runtime_config = runtime_config_v3(config)?;
        unsafe { register_runtime(runtime_config, config.logger, config.logger_v2, client, out) }
    })
}

#[no_mangle]
/// Creates a production runtime with explicit TCP socket tuning.
///
/// # Safety
/// All non-null pointers and callbacks follow the lifetime rules of `rnet_runtime_create_v3`.
pub unsafe extern "C" fn rnet_runtime_create_v4(
    config: *const RnetConfigV4,
    client: *const RnetClientSecurity,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("config and out must be non-null");
        }
        let config = unsafe { *config };
        validate_struct(
            config.struct_size,
            config.abi_version,
            size_of::<RnetConfigV4>(),
        )?;
        let base = config.v3_fields();
        let mut runtime_config = runtime_config_v3(base)?;
        runtime_config.tcp_nodelay = config.tcp_nodelay != 0;
        runtime_config.tcp_send_buffer_bytes = optional_usize(config.tcp_send_buffer_bytes)?;
        runtime_config.tcp_recv_buffer_bytes = optional_usize(config.tcp_recv_buffer_bytes)?;
        unsafe { register_runtime(runtime_config, config.logger, config.logger_v2, client, out) }
    })
}

#[no_mangle]
/// Creates a production runtime with global endpoint and pending-handshake limits.
///
/// # Safety
/// All non-null pointers and callbacks follow the lifetime rules of `rnet_runtime_create_v3`.
pub unsafe extern "C" fn rnet_runtime_create_v5(
    config: *const RnetConfigV5,
    client: *const RnetClientSecurity,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("config and out must be non-null");
        }
        let config = unsafe { *config };
        validate_struct(
            config.struct_size,
            config.abi_version,
            size_of::<RnetConfigV5>(),
        )?;
        let runtime_config = runtime_config_v5(config)?;
        unsafe { register_runtime(runtime_config, config.logger, config.logger_v2, client, out) }
    })
}

pub(crate) fn runtime_config_v5(config: RnetConfigV5) -> rnet_core::Result<RuntimeConfig> {
    let socket = config.v4_fields();
    let mut runtime_config = runtime_config_v3(socket.v3_fields())?;
    runtime_config.tcp_nodelay = socket.tcp_nodelay != 0;
    runtime_config.tcp_send_buffer_bytes = optional_usize(socket.tcp_send_buffer_bytes)?;
    runtime_config.tcp_recv_buffer_bytes = optional_usize(socket.tcp_recv_buffer_bytes)?;
    runtime_config.max_endpoints = config.max_endpoints as usize;
    runtime_config.max_pending_handshakes = config.max_pending_handshakes as usize;
    Ok(runtime_config)
}

fn runtime_config_v3(config: RnetConfigV3) -> rnet_core::Result<RuntimeConfig> {
    let mut runtime_config = RuntimeConfig::production();
    runtime_config.worker_threads = config.worker_threads as usize;
    runtime_config.event_queue_capacity = config.event_queue_capacity as usize;
    runtime_config.write_queue_capacity = config.write_queue_capacity as usize;
    runtime_config.max_body_len = config.max_body_len as usize;
    runtime_config.max_datagram_size = config.max_datagram_size as usize;
    runtime_config.max_event_bytes = usize::try_from(config.max_event_bytes)
        .map_err(|_| RnetError::new(ErrorCode::InvalidArgument, "max_event_bytes overflow"))?;
    runtime_config.max_runtime_queued_bytes = usize::try_from(config.max_runtime_queued_bytes)
        .map_err(|_| RnetError::new(ErrorCode::InvalidArgument, "runtime byte budget overflow"))?;
    runtime_config.max_session_queued_bytes = usize::try_from(config.max_session_queued_bytes)
        .map_err(|_| RnetError::new(ErrorCode::InvalidArgument, "session byte budget overflow"))?;
    runtime_config.max_sessions_per_endpoint = config.max_sessions_per_endpoint as usize;
    runtime_config.max_sessions_per_ip = config.max_sessions_per_ip as usize;
    runtime_config.handshake_rate_per_ip = config.handshake_rate_per_ip;
    runtime_config.handshake_burst_per_ip = config.handshake_burst_per_ip;
    runtime_config.ipv6_admission_prefix_bits = u8::try_from(config.ipv6_admission_prefix_bits)
        .map_err(|_| RnetError::new(ErrorCode::InvalidArgument, "invalid IPv6 prefix width"))?;
    runtime_config.handshake_timeout = Duration::from_millis(config.handshake_timeout_ms);
    runtime_config.connect_timeout = Duration::from_millis(config.connect_timeout_ms);
    runtime_config.dns_timeout = Duration::from_millis(config.dns_timeout_ms);
    runtime_config.datagram_idle_timeout = Duration::from_millis(config.datagram_idle_timeout_ms);
    runtime_config.security_policy.allow_plaintext_business_data =
        config.allow_plaintext_business_data != 0;
    runtime_config
        .security_policy
        .allow_legacy_unauthenticated_endpoints =
        config.allow_legacy_unauthenticated_endpoints != 0;
    runtime_config.security_policy.rekey_after =
        (config.rekey_after_ms != 0).then(|| Duration::from_millis(config.rekey_after_ms));
    runtime_config.security_policy.rekey_after_bytes =
        (config.rekey_after_bytes != 0).then_some(config.rekey_after_bytes);
    Ok(runtime_config)
}

fn optional_usize(value: u64) -> rnet_core::Result<Option<usize>> {
    if value == 0 {
        Ok(None)
    } else {
        usize::try_from(value)
            .map(Some)
            .map_err(|_| RnetError::new(ErrorCode::InvalidArgument, "socket buffer size overflow"))
    }
}

unsafe fn create_runtime(
    config: *const RnetConfig,
    client: *const RnetClientSecurity,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("config and out must be non-null");
        }
        let config = unsafe { *config };
        validate_struct(
            config.struct_size,
            config.abi_version,
            size_of::<RnetConfig>(),
        )?;
        let runtime_config = RuntimeConfig {
            worker_threads: config.worker_threads as usize,
            event_queue_capacity: config.event_queue_capacity as usize,
            write_queue_capacity: config.write_queue_capacity as usize,
            max_body_len: config.max_body_len as usize,
            max_datagram_size: config.max_datagram_size as usize,
            ..RuntimeConfig::default()
        };
        unsafe { register_runtime(runtime_config, config.logger, std::ptr::null(), client, out) }
    })
}

unsafe fn register_runtime(
    runtime_config: RuntimeConfig,
    logger: *const crate::abi::RnetLogger,
    logger_v2: *const crate::abi::RnetLoggerV2,
    client: *const RnetClientSecurity,
    out: *mut u64,
) -> rnet_core::Result<()> {
    if !logger.is_null() && !logger_v2.is_null() {
        return invalid_argument("logger and logger_v2 are mutually exclusive");
    }
    let logger = if logger_v2.is_null() {
        unsafe { build_logger(logger) }?
    } else {
        unsafe { build_logger_v2(logger_v2) }?
    };
    let client_security = if client.is_null() {
        None
    } else {
        let client = unsafe { *client };
        validate_struct(
            client.struct_size,
            client.abi_version,
            size_of::<RnetClientSecurity>(),
        )?;
        let private = unsafe { copy_key(client.local_private_key)? };
        let local_key = Keypair::from_private(&private)
            .map_err(|error| RnetError::new(ErrorCode::CryptoError, error.to_string()))?;
        if let Some(verify) = client.verify_server {
            let user_data = client.user_data as usize;
            Some(ClientSecurity::with_verifier(
                local_key,
                Arc::new(move |key| unsafe {
                    verify(user_data as *mut _, key.as_ptr(), key.len()) != 0
                }),
            ))
        } else {
            let expected = unsafe { copy_key(client.expected_server_public_key) }?;
            Some(ClientSecurity::pinned(local_key, expected.to_vec()))
        }
    };
    let entry = Arc::new(RuntimeEntry {
        network: NetworkRuntime::new_with_client_security(runtime_config, client_security)?,
        buffers: BufferStore::new(),
        logger: Mutex::new(logger),
        metrics_log_interval_ms: AtomicU64::new(60_000),
        last_metrics_log: Mutex::new(Instant::now()),
        active_calls: AtomicUsize::new(0),
    });
    let handle = runtimes()
        .lock()
        .expect("runtime table poisoned")
        .insert(Arc::clone(&entry));
    log_entry(
        &entry,
        handle,
        "runtime_created",
        LogLevel::Info,
        "runtime created",
    );
    unsafe { out.write(handle) };
    Ok(())
}

#[no_mangle]
pub extern "C" fn rnet_runtime_stop(runtime: u64, drain_timeout_ms: u32) -> i32 {
    ffi_status(|| {
        if IN_LOG_CALLBACK.with(Cell::get) {
            return invalid_state("stop cannot be called from the logger callback");
        }
        let entry = runtime_entry(runtime)?;
        log_entry(
            &entry,
            runtime,
            "runtime_stopping",
            LogLevel::Info,
            "runtime stopping",
        );
        entry
            .network
            .stop(Duration::from_millis(u64::from(drain_timeout_ms)))?;
        let logger = entry.logger.lock().expect("logger lock poisoned").take();
        if let Some(mut logger) = logger {
            logger.shutdown();
        }
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn rnet_runtime_destroy(runtime: u64) -> i32 {
    ffi_status(|| {
        if IN_LOG_CALLBACK.with(Cell::get) {
            return invalid_state("destroy cannot be called from the logger callback");
        }
        let mut table = runtimes().lock().expect("runtime table poisoned");
        let entry = table
            .get(runtime)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid runtime"))?;
        if entry.network.lifecycle() != Lifecycle::Stopped {
            return invalid_state("runtime must be stopped before destroy");
        }
        if entry.buffers.outstanding() != 0 {
            return invalid_state("all borrowed buffers must be released before destroy");
        }
        if entry.active_calls.load(Ordering::Acquire) != 0 {
            return Err(RnetError::new(
                ErrorCode::WouldBlock,
                "runtime still has active API calls",
            ));
        }
        table.remove(runtime);
        Ok(())
    })
}
