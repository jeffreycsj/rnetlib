#![no_main]

use libfuzzer_sys::fuzz_target;
use rnet_protocol::{decode_datagram, encode_frame, FrameCodec};

fuzz_target!(|data: &[u8]| {
    let maximum = 1024 * 1024;
    if let Ok(frame) = decode_datagram(data, maximum) {
        let encoded = encode_frame(
            frame.header.correlation_id,
            frame.header.flags,
            &frame.body,
            maximum,
        )
        .unwrap();
        assert_eq!(encoded.as_ref(), data);
    }
    let mut codec = FrameCodec::new(maximum).unwrap();
    for chunk in data.chunks(7) {
        let Ok(frames) = codec.push(chunk) else { break };
        for frame in frames {
            let encoded = encode_frame(
                frame.header.correlation_id,
                frame.header.flags,
                &frame.body,
                maximum,
            )
            .unwrap();
            assert_eq!(decode_datagram(&encoded, maximum).unwrap(), frame);
        }
    }
});
