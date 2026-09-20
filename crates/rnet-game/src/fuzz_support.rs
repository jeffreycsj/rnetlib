//! Narrow untrusted-input surfaces for out-of-tree fuzz targets.

use crate::{envelope, heartbeat, join};

/// Exercises both game wire parsers at the maximum normal handshake/body budget.
pub fn decode_game_wire(input: &[u8]) {
    if let Ok(envelope::DecodedEnvelope::Control {
        kind: envelope::ControlKind::Heartbeat,
        payload,
    }) = envelope::decode(input, 60 * 1024)
    {
        let _ = heartbeat::decode_heartbeat(&payload);
    }
    let _ = join::decode(input);
}
