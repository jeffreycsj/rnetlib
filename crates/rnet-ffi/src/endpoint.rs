use crate::abi::RnetEndpointConfig;
use crate::abi::RnetEndpointMode;
use crate::abi::RnetJoinConfig;
use crate::abi::RnetKeypair;
use crate::abi::RnetListenerConfig;
use crate::abi::RnetSlice;
use crate::abi::{RnetClientConfigV2, RnetServerConfigV2};
use crate::registry::copy_key;
use crate::registry::ffi_status;
use crate::registry::invalid_argument;
use crate::registry::parse_address;
use crate::registry::runtime_entry;
use crate::registry::validate_struct;
use crate::registry::with_borrowed_slice;
use rnet_core::ErrorCode;
use rnet_core::RnetError;
use rnet_core::Transport;
use rnet_security::Keypair;
use rnet_transport::{EndpointConfig, HostClientConfig, SecurityMode, ServerConfig};
use std::mem::size_of;
use std::net::SocketAddr;

#[no_mangle]
/// Opens a server listener; `config.transport` selects TCP, UDP, or KCP once.
///
/// # Safety
/// `config`, its slices, and `out` must be valid for this call.
pub unsafe extern "C" fn rnet_server_open_v2(
    runtime: u64,
    config: *const RnetServerConfigV2,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("server config and out must be non-null");
        }
        let config = unsafe { *config };
        validate_struct(
            config.struct_size,
            config.abi_version,
            size_of::<RnetServerConfigV2>(),
        )?;
        if config.reserved != 0 {
            return invalid_argument("server reserved field must be zero");
        }
        let private = unsafe { copy_key(config.local_private_key) }?;
        let local_key = Keypair::from_private(&private)
            .map_err(|error| RnetError::new(ErrorCode::CryptoError, error.to_string()))?;
        let endpoint = runtime_entry(runtime)?.network.listen(ServerConfig {
            transport: Transport::try_from(config.transport)?,
            bind_addr: unsafe { parse_address(config.bind_host, config.bind_port) }?,
            local_key,
            initial_security: SecurityMode::try_from(
                u8::try_from(config.initial_security).map_err(|_| {
                    RnetError::new(ErrorCode::InvalidArgument, "invalid security mode")
                })?,
            )?,
        })?;
        unsafe { out.write(endpoint) };
        Ok(())
    })
}

#[no_mangle]
/// Starts a client connection. Security is negotiated automatically from runtime trust policy.
///
/// # Safety
/// `config`, its slices, and `out` must be valid for this call.
pub unsafe extern "C" fn rnet_client_connect_v2(
    runtime: u64,
    config: *const RnetClientConfigV2,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("client config and out must be non-null");
        }
        let config = unsafe { *config };
        validate_struct(
            config.struct_size,
            config.abi_version,
            size_of::<RnetClientConfigV2>(),
        )?;
        if config.reserved0 != 0 || config.reserved1 != 0 {
            return invalid_argument("client reserved fields must be zero");
        }
        let transport = Transport::try_from(config.transport)?;
        let host = unsafe {
            with_borrowed_slice(config.remote_host, |host| {
                std::str::from_utf8(host)
                    .map(str::to_owned)
                    .map_err(|_| RnetError::new(ErrorCode::InvalidArgument, "host is not UTF-8"))
            })
        }?;
        let join_payload =
            unsafe { with_borrowed_slice(config.join_payload, |payload| Ok(payload.to_vec())) }?;
        let endpoint = runtime_entry(runtime)?
            .network
            .connect_host(HostClientConfig {
                transport,
                host,
                port: config.remote_port,
                join_payload,
            })?;
        unsafe { out.write(endpoint) };
        Ok(())
    })
}

#[no_mangle]
/// Generates a Noise static keypair in caller-owned storage.
///
/// # Safety
/// `out` must point to writable memory for one `RnetKeypair`.
pub unsafe extern "C" fn rnet_keypair_generate(out: *mut RnetKeypair) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("keypair output is null");
        }
        let generated = Keypair::generate()
            .map_err(|error| RnetError::new(ErrorCode::CryptoError, error.to_string()))?;
        let mut keypair = RnetKeypair::default();
        keypair.private_key.copy_from_slice(&generated.private);
        keypair.public_key.copy_from_slice(&generated.public);
        unsafe { out.write(keypair) };
        Ok(())
    })
}

#[no_mangle]
/// Restores a Noise static keypair from a caller-owned 32-byte private key.
///
/// # Safety
/// `private_key` must reference readable memory for this call and `out` must point to writable
/// memory for one `RnetKeypair`.
pub unsafe extern "C" fn rnet_keypair_from_private(
    private_key: RnetSlice,
    out: *mut RnetKeypair,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("keypair output is null");
        }
        let private = unsafe { copy_key(private_key)? };
        let restored = Keypair::from_private(&private)
            .map_err(|error| RnetError::new(ErrorCode::CryptoError, error.to_string()))?;
        let mut keypair = RnetKeypair::default();
        keypair.private_key.copy_from_slice(&restored.private);
        keypair.public_key.copy_from_slice(&restored.public);
        unsafe { out.write(keypair) };
        Ok(())
    })
}

#[no_mangle]
/// Opens an authenticated server endpoint whose transport is fixed by `config`.
///
/// # Safety
/// `config`, its borrowed slices, and `out` must remain valid for this call.
pub unsafe extern "C" fn rnet_listener_open(
    runtime: u64,
    config: *const RnetListenerConfig,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("listener config and out must be non-null");
        }
        let config = unsafe { *config };
        validate_struct(
            config.struct_size,
            config.abi_version,
            size_of::<RnetListenerConfig>(),
        )?;
        let transport = Transport::try_from(config.transport)?;
        let private = unsafe { copy_key(config.local_private_key)? };
        let bind_addr = unsafe { parse_address(config.bind_host, config.bind_port) }?;
        let local_key = Keypair {
            private: private.to_vec(),
            public: Vec::new(),
        };
        let endpoint_config = match transport {
            Transport::Tcp => EndpointConfig::secure_tcp_listener(bind_addr, local_key),
            Transport::Udp => EndpointConfig::secure_udp_listener(bind_addr, local_key),
            Transport::Kcp => EndpointConfig::secure_kcp_listener(bind_addr, local_key),
        };
        let endpoint = runtime_entry(runtime)?
            .network
            .open_endpoint(endpoint_config)?;
        unsafe { out.write(endpoint) };
        Ok(())
    })
}

#[no_mangle]
/// Starts an authenticated client join attempt.
///
/// # Safety
/// `config`, its borrowed slices, and `out` must remain valid for this call.
pub unsafe extern "C" fn rnet_client_join(
    runtime: u64,
    config: *const RnetJoinConfig,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("join config and out must be non-null");
        }
        let config = unsafe { *config };
        validate_struct(
            config.struct_size,
            config.abi_version,
            size_of::<RnetJoinConfig>(),
        )?;
        let transport = Transport::try_from(config.transport)?;
        let private = unsafe { copy_key(config.local_private_key)? };
        let peer = unsafe { copy_key(config.expected_server_public_key)? };
        let join_payload =
            unsafe { with_borrowed_slice(config.join_payload, |bytes| Ok(bytes.to_vec())) }?;
        let remote_addr = unsafe { parse_address(config.remote_host, config.remote_port) }?;
        let bind_addr = SocketAddr::new(
            if remote_addr.is_ipv4() {
                "0.0.0.0".parse().expect("valid wildcard IPv4")
            } else {
                "::".parse().expect("valid wildcard IPv6")
            },
            0,
        );
        let local_key = Keypair {
            private: private.to_vec(),
            public: Vec::new(),
        };
        let endpoint_config = match transport {
            Transport::Tcp => EndpointConfig::secure_tcp_client(
                remote_addr,
                local_key,
                peer.to_vec(),
                join_payload,
            ),
            Transport::Udp => EndpointConfig::secure_udp_client(
                bind_addr,
                remote_addr,
                local_key,
                peer.to_vec(),
                join_payload,
            ),
            Transport::Kcp => EndpointConfig::secure_kcp_client(
                bind_addr,
                remote_addr,
                local_key,
                peer.to_vec(),
                join_payload,
            ),
        };
        let endpoint = runtime_entry(runtime)?
            .network
            .open_endpoint(endpoint_config)?;
        unsafe { out.write(endpoint) };
        Ok(())
    })
}

#[no_mangle]
/// Opens a transport endpoint.
///
/// # Safety
/// `config`, its non-empty host slices, and `out` must be valid for the duration of the call.
pub unsafe extern "C" fn rnet_endpoint_open(
    runtime: u64,
    config: *const RnetEndpointConfig,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("endpoint config and out must be non-null");
        }
        let config = unsafe { *config };
        validate_struct(
            config.struct_size,
            config.abi_version,
            size_of::<RnetEndpointConfig>(),
        )?;
        let entry = runtime_entry(runtime)?;
        let transport = Transport::try_from(config.transport)?;
        let endpoint_config = match (transport, config.mode) {
            (Transport::Tcp, mode) if mode == RnetEndpointMode::Listener as u32 => {
                EndpointConfig::tcp_listener(unsafe {
                    parse_address(config.bind_host, config.bind_port)
                }?)
            }
            (Transport::Tcp, mode) if mode == RnetEndpointMode::Client as u32 => {
                EndpointConfig::tcp_client(unsafe {
                    parse_address(config.remote_host, config.remote_port)
                }?)
            }
            (Transport::Udp, mode) if mode == RnetEndpointMode::Datagram as u32 => {
                let remote = if config.remote_port == 0 {
                    None
                } else {
                    Some(unsafe { parse_address(config.remote_host, config.remote_port) }?)
                };
                EndpointConfig::udp(
                    unsafe { parse_address(config.bind_host, config.bind_port) }?,
                    remote,
                )
            }
            (Transport::Kcp, _) => {
                EndpointConfig::kcp(unsafe { parse_address(config.bind_host, config.bind_port) }?)
            }
            _ => return invalid_argument("mode is incompatible with transport"),
        };
        let endpoint = entry.network.open_endpoint(endpoint_config)?;
        unsafe { out.write(endpoint) };
        Ok(())
    })
}

#[no_mangle]
/// Returns the bound local port of an endpoint.
///
/// # Safety
/// `out_port` must point to writable memory for one `u16`.
pub unsafe extern "C" fn rnet_endpoint_local_port(
    runtime: u64,
    endpoint: u64,
    out_port: *mut u16,
) -> i32 {
    ffi_status(|| {
        if out_port.is_null() {
            return invalid_argument("out_port is null");
        }
        let port = runtime_entry(runtime)?
            .network
            .endpoint_local_addr(endpoint)?
            .port();
        unsafe { out_port.write(port) };
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn rnet_endpoint_close(runtime: u64, endpoint: u64) -> i32 {
    ffi_status(|| runtime_entry(runtime)?.network.close_endpoint(endpoint))
}
