//! Canonical game endpoint and negotiation telemetry ABI.

use crate::abi::RnetSlice;
use crate::game_abi::RnetGameTransportLatest;
use crate::game_config_abi::{RnetGameClientConfig, RnetGameServerConfig};
use crate::game_range_abi::RnetGameRangeBuffer;
use crate::game_registry;
use crate::registry::{copy_key, ffi_status, invalid_argument, parse_address, with_borrowed_slice};
use rnet_core::{ErrorCode, Result, RnetError, Transport};
use rnet_game::{GameHostClientConfig, GameProtocol, GameServerConfig};
use rnet_security::Keypair;
use zeroize::Zeroizing;

#[no_mangle]
/// # Safety
/// `config`, its slices, and `out` must remain valid for this call.
pub unsafe extern "C" fn rnet_game_server_listen(
    runtime: u64,
    config: *const RnetGameServerConfig,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("game range server config and output must be non-null");
        }
        let config = unsafe { crate::registry::read_config(config) }?;
        if config.reserved != 0 || config.initial_encryption > 1 {
            return invalid_argument("invalid range server flags");
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
                protocol: GameProtocol::new(
                    config.protocol_id,
                    config.min_version,
                    config.max_version,
                ),
            })?;
        unsafe { out.write(endpoint) };
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// `config`, its slices, and `out` must remain valid for this call.
pub unsafe extern "C" fn rnet_game_client_connect(
    runtime: u64,
    config: *const RnetGameClientConfig,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("game range client config and output must be non-null");
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
/// Resumes with the ticket's original selected version, not a new range maximum.
///
/// # Safety
/// `config`, all slices, and `out` must remain valid for this call.
pub unsafe extern "C" fn rnet_game_client_resume_connect(
    runtime: u64,
    config: *const RnetGameClientConfig,
    old_session: u64,
    resume_ticket: RnetSlice,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if config.is_null() || out.is_null() {
            return invalid_argument("game range client config and output must be non-null");
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
        return invalid_argument("game range client reserved fields must be zero");
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
            min_version: config.min_version,
            max_version: config.max_version,
            build_id: config.build_id,
            capabilities: config.capabilities,
        },
    })
}

#[no_mangle]
/// # Safety
/// `out` must point to writable `uint32_t` storage.
pub unsafe extern "C" fn rnet_game_selected_protocol_version(
    runtime: u64,
    session: u64,
    out: *mut u32,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("selected version output is null");
        }
        let selected = game_registry::lease(runtime)?
            .runtime
            .selected_protocol_version(session)?;
        unsafe { out.write(selected) };
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// `out` must point to writable `uint64_t` storage.
pub unsafe extern "C" fn rnet_game_transport_latest_replacements(
    runtime: u64,
    out: *mut u64,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("latest replacement output is null");
        }
        let replaced = game_registry::lease(runtime)?
            .runtime
            .transport_latest_replacements();
        unsafe { out.write(replaced) };
        Ok(())
    })
}

#[no_mangle]
/// Returns cumulative transport-pending replacement, pickup and admission-failure counters.
///
/// # Safety
/// `out` must point to writable storage for one `RnetGameTransportLatest`.
pub unsafe extern "C" fn rnet_game_transport_latest_snapshot(
    runtime: u64,
    out: *mut RnetGameTransportLatest,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("latest transport snapshot output is null");
        }
        let snapshot = game_registry::lease(runtime)?
            .runtime
            .transport_latest_snapshot();
        unsafe { out.write(snapshot.into()) };
        Ok(())
    })
}

#[no_mangle]
/// Returns runtime-wide wire-v4 early-data gauges, limits, peaks, and rejection counters.
///
/// # Safety
/// `out` must point to writable storage for one `RnetGameRangeBuffer`.
pub unsafe extern "C" fn rnet_game_range_buffer_snapshot(
    runtime: u64,
    out: *mut RnetGameRangeBuffer,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game range buffer output is null");
        }
        let snapshot = game_registry::lease(runtime)?
            .runtime
            .range_buffer_snapshot();
        unsafe { out.write(snapshot.into()) };
        Ok(())
    })
}
