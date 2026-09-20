#![no_main]

use libfuzzer_sys::fuzz_target;
use rnet_protocol::control::{decode_control, decode_protected, decode_record};

fuzz_target!(|data: &[u8]| {
    let limit = data.len().min(64 * 1024);
    let _ = decode_record(data, limit);
    let _ = decode_protected(data, limit);
    let _ = decode_control(data);
});
