//! Narrow untrusted-input surfaces for out-of-tree fuzz targets.

use crate::{envelope, join};

/// Exercises both game wire parsers at the maximum normal handshake/body budget.
pub fn decode_game_wire(input: &[u8]) {
    let _ = envelope::decode(input, 60 * 1024);
    let _ = join::decode(input);
}
