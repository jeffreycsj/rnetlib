//! Session and endpoint control entry points for the high-level game API.

use crate::game_registry;
use crate::registry::{ffi_status, invalid_argument};

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
