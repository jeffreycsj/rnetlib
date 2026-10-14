use crate::abi::RnetKeypair;
use crate::abi::RnetSlice;
use crate::abi::{RnetClientConfig, RnetServerConfig};
use crate::registry::copy_key;
use crate::registry::ffi_status;
use crate::registry::invalid_argument;
use crate::registry::parse_address;
use crate::registry::runtime_entry;
use crate::registry::with_borrowed_slice;
use rnet_core::ErrorCode;
use rnet_core::RnetError;
use rnet_core::Transport;
use rnet_security::Keypair;
use rnet_transport::{HostClientConfig, SecurityMode, ServerConfig};

#[no_mangle]
/// Opens a server listener; `config.transport` selects TCP, UDP, or KCP once.
///
/// # Safety
/// `config`, its slices, and `out` must be valid for this call.
pub unsafe extern "C" fn rnet_server_listen(
    runtime: u64,
    config: *const RnetServerConfig,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("server config and out must be non-null");
        }
        let config = unsafe { crate::registry::read_config(config) }?;
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
pub unsafe extern "C" fn rnet_client_connect(
    runtime: u64,
    config: *const RnetClientConfig,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("client config and out must be non-null");
        }
        let config = unsafe { crate::registry::read_config(config) }?;
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
