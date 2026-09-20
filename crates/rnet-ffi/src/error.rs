//! Thread-local diagnostic text for synchronous C ABI failures.

use crate::registry::last_error_ptr;
use std::ffi::c_char;

/// Returns diagnostic text for the most recent failed RNet call on the current thread.
///
/// The pointer is never null and remains valid until another RNet function is called on the same
/// thread. Callers that need longer ownership must copy the UTF-8, NUL-terminated string.
#[no_mangle]
pub extern "C" fn rnet_last_error_message() -> *const c_char {
    last_error_ptr()
}
