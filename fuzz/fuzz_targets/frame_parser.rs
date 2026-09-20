#![no_main]

use libfuzzer_sys::fuzz_target;
use rnet_protocol::FrameCodec;

fuzz_target!(|data: &[u8]| {
    if let Ok(mut codec) = FrameCodec::new(1024 * 1024) {
        for chunk in data.chunks(7) {
            let _ = codec.push(chunk);
        }
    }
});

