//! Session and endpoint control entry points for the high-level game API.

use crate::game_registry;
use crate::registry::{ffi_status, invalid_argument};

#[no_mangle]
/// Explicitly closes session, drops staged work and revokes its resume state; keeps listener.
pub extern "C" fn rnet_game_session_close(runtime: u64, session: u64) -> i32 {
    ffi_status(|| {
        game_registry::lease(runtime)?
            .runtime
            .close_session(session)
    })
}

#[no_mangle]
/// Closes endpoint and owned sessions, discarding their staged work/resume state.
pub extern "C" fn rnet_game_endpoint_close(runtime: u64, endpoint: u64) -> i32 {
    ffi_status(|| {
        game_registry::lease(runtime)?
            .runtime
            .close_endpoint(endpoint)
    })
}

#[no_mangle]
/// Server queues key rotation; wait for SECURITY_CHANGED with REKEY operation for local commit.
pub extern "C" fn rnet_game_rekey(runtime: u64, session: u64) -> i32 {
    ffi_status(|| game_registry::lease(runtime)?.runtime.rekey(session))
}

#[no_mangle]
/// Server requests encrypted=0/1; clients automatically follow after the protected barrier.
/// Disabling requires runtime plaintext opt-in; plaintext business data lacks integrity.
/// Wait for SECURITY_CHANGED to observe local commit; admission is not peer completion.
/// An already active mode may be a no-op without a new event.
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
