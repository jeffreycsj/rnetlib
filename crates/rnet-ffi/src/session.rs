use crate::abi::RnetSendOptions;
use crate::abi::RnetSlice;
use crate::registry::ffi_status;
use crate::registry::invalid_argument;
use crate::registry::runtime_entry;
use crate::registry::with_borrowed_slice;
use rnet_core::ErrorCode;
use rnet_transport::{SecurityMode, SendOptions};

#[no_mangle]
pub extern "C" fn rnet_session_auth_decide(runtime: u64, session: u64, accept: u32) -> i32 {
    ffi_status(|| {
        if accept > 1 {
            return invalid_argument("accept must be zero or one");
        }
        runtime_entry(runtime)?
            .network
            .auth_decide(session, accept == 1)
    })
}

#[no_mangle]
/// Requests a server-authoritative live-session security mode change.
pub extern "C" fn rnet_session_security_set(runtime: u64, session: u64, mode: u32) -> i32 {
    ffi_status(|| {
        let mode = u8::try_from(mode)
            .ok()
            .and_then(|value| SecurityMode::try_from(value).ok())
            .ok_or_else(|| {
                rnet_core::RnetError::new(ErrorCode::InvalidArgument, "invalid security mode")
            })?;
        runtime_entry(runtime)?
            .network
            .set_security_mode(session, mode)
    })
}

#[no_mangle]
/// Requests fresh transport keys for an established server-side session.
pub extern "C" fn rnet_session_rekey(runtime: u64, session: u64) -> i32 {
    ffi_status(|| runtime_entry(runtime)?.network.rekey_session(session))
}

#[no_mangle]
/// Enqueues opaque application bytes. The payload schema owns business message types.
///
/// # Safety
/// A non-empty payload must reference readable memory for the duration of the call.
pub unsafe extern "C" fn rnet_session_send(runtime: u64, session: u64, payload: RnetSlice) -> i32 {
    unsafe { rnet_session_send_ex(runtime, session, payload, std::ptr::null()) }
}

#[no_mangle]
/// Enqueues one message with optional advanced metadata.
///
/// # Safety
/// A non-empty payload and a non-null `options` must reference readable memory for the call.
pub unsafe extern "C" fn rnet_session_send_ex(
    runtime: u64,
    session: u64,
    payload: RnetSlice,
    options: *const RnetSendOptions,
) -> i32 {
    ffi_status(|| {
        let correlation_id = if options.is_null() {
            0
        } else {
            let options = unsafe { crate::registry::read_config(options) }?;
            if options.flags != 0 || options.reserved != 0 {
                return invalid_argument("send options contain unsupported flags");
            }
            options.correlation_id
        };
        let entry = runtime_entry(runtime)?;
        unsafe {
            with_borrowed_slice(payload, |payload| {
                entry
                    .network
                    .send_with_options(session, payload, SendOptions { correlation_id })
            })
        }
    })
}

#[no_mangle]
pub extern "C" fn rnet_session_close(runtime: u64, session: u64, reason: i32) -> i32 {
    ffi_status(|| {
        let reason = ErrorCode::try_from(reason)?;
        runtime_entry(runtime)?
            .network
            .close_session(session, reason)
    })
}
