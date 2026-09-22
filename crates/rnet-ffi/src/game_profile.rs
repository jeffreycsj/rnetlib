//! Stable C helpers for game scenario profiles; profiles only expand into existing config fields.

use crate::registry::{ffi_status, invalid_argument};
use rnet_game::GameProfile;

pub const RNET_GAME_PROFILE_REALTIME: u32 = 1;
pub const RNET_GAME_PROFILE_RELIABLE_REALTIME: u32 = 2;
pub const RNET_GAME_PROFILE_SESSION: u32 = 3;

#[no_mangle]
/// Resolves a game profile to its immutable transport and encrypted server default.
///
/// # Safety
/// Both output pointers must be non-null and writable for one `uint32_t`.
pub unsafe extern "C" fn rnet_game_profile_defaults(
    profile: u32,
    out_transport: *mut u32,
    out_initial_encryption: *mut u32,
) -> i32 {
    ffi_status(|| {
        if out_transport.is_null() || out_initial_encryption.is_null() {
            return invalid_argument("game profile outputs must be non-null");
        }
        let transport = GameProfile::try_from(profile)?.transport();
        unsafe {
            out_transport.write(transport as u32);
            out_initial_encryption.write(1);
        }
        Ok(())
    })
}
