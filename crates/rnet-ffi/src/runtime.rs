use crate::abi::RnetClientSecurity;
use crate::abi::RNET_ABI_VERSION;
use crate::abi_config::RnetConfig;
use crate::observe::build_logger;
use crate::observe::log_entry;
use crate::registry::copy_key;
use crate::registry::ffi_status;
use crate::registry::invalid_argument;
use crate::registry::invalid_state;
use crate::registry::runtime_entry;
use crate::registry::runtimes;
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
/// Initializes the complete configuration with production-safe resource limits.
///
/// # Safety
/// `config` must point to writable memory for one `RnetConfig`.
pub unsafe extern "C" fn rnet_config_init(config: *mut RnetConfig) -> i32 {
    ffi_status(|| {
        if config.is_null() {
            return invalid_argument("config must be non-null");
        }
        unsafe { config.write(RnetConfig::default()) };
        Ok(())
    })
}

#[no_mangle]
/// Creates a production runtime with global endpoint and pending-handshake limits.
///
/// # Safety
/// Configuration/security pointers and `out` must be valid for this call. A logger or verifier
/// callback and its user data must remain thread-safe and valid until runtime destruction.
pub unsafe extern "C" fn rnet_runtime_create(
    config: *const RnetConfig,
    client: *const RnetClientSecurity,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("config and out must be non-null");
        }
        let config = unsafe { crate::registry::read_config(config) }?;
        let runtime_config = runtime_config(config)?;
        unsafe { register_runtime(runtime_config, config.logger, client, out) }
    })
}

pub(crate) fn runtime_config(config: RnetConfig) -> rnet_core::Result<RuntimeConfig> {
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
    runtime_config.tcp_nodelay = config.tcp_nodelay != 0;
    runtime_config.tcp_send_buffer_bytes = optional_usize(config.tcp_send_buffer_bytes)?;
    runtime_config.tcp_recv_buffer_bytes = optional_usize(config.tcp_recv_buffer_bytes)?;
    runtime_config.max_endpoints = config.max_endpoints as usize;
    runtime_config.max_pending_handshakes = config.max_pending_handshakes as usize;
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

unsafe fn register_runtime(
    runtime_config: RuntimeConfig,
    logger: *const crate::abi::RnetLogger,
    client: *const RnetClientSecurity,
    out: *mut u64,
) -> rnet_core::Result<()> {
    let logger = unsafe { build_logger(logger) }?;
    let client_security = if client.is_null() {
        None
    } else {
        let client = unsafe { crate::registry::read_config(client) }?;
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
        let removed = table.remove(runtime);
        drop(table);
        drop(removed);
        Ok(())
    })
}
