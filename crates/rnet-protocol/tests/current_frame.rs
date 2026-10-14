use rnet_core::ErrorCode;
use rnet_protocol::{decode_datagram, encode_frame, FrameCodec};

// Fixed vectors, independent of the encoder, pin the current network-byte-order layout.
const CURRENT: &[u8] = &[
    0x52, 0x4e, 0x45, 0x54, 0, 2, 0, 0, 0, 0, 0, 3, 1, 2, 3, 4, 5, 6, 7, 8, 0, 0xff, 0x80,
];
const OLD: &[u8] = &[
    0x52, 0x4e, 0x45, 0x54, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 1,
    0xca, 0xfe,
];

#[test]
fn current_compact_frame_decodes_at_every_stream_split() {
    let expected = decode_datagram(CURRENT, 3).expect("current 20-byte header");
    assert_eq!(expected.body.as_ref(), &[0, 0xff, 0x80]);
    assert_eq!(expected.header.correlation_id, 0x0102_0304_0506_0708);
    assert_eq!(
        encode_frame(0x0102_0304_0506_0708, 0, &[0, 0xff, 0x80], 3)
            .unwrap()
            .as_ref(),
        CURRENT
    );
    for split in 0..=CURRENT.len() {
        let mut codec = FrameCodec::new(3).unwrap();
        let mut frames = codec.push(&CURRENT[..split]).unwrap();
        frames.extend(codec.push(&CURRENT[split..]).unwrap());
        assert_eq!(frames, vec![expected.clone()], "split={split}");
    }
}

#[test]
fn empty_frame_uses_only_current_header_and_preserves_zero_correlation() {
    let encoded = encode_frame(0, 0, b"", 1).unwrap();
    assert_eq!(
        encoded.as_ref(),
        &[0x52, 0x4e, 0x45, 0x54, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    let frame = decode_datagram(&encoded, 1).unwrap();
    assert!(frame.body.is_empty());
    assert_eq!(frame.header.correlation_id, 0);
}

#[test]
fn previous_frame_layout_is_rejected_instead_of_reinterpreted() {
    let error = decode_datagram(OLD, 64).expect_err("old frame must not decode");
    assert_eq!(error.code(), ErrorCode::ProtocolError);
    assert!(error.to_string().contains("version"));
    let error = FrameCodec::new(64)
        .unwrap()
        .push(OLD)
        .expect_err("old stream frame");
    assert_eq!(error.code(), ErrorCode::ProtocolError);
    assert!(error.to_string().contains("version"));
}

#[test]
fn compact_datagram_requires_exactly_one_complete_frame() {
    for length in 0..CURRENT.len() {
        assert_eq!(
            decode_datagram(&CURRENT[..length], 3).unwrap_err().code(),
            ErrorCode::ProtocolError
        );
    }
    let mut extra = CURRENT.to_vec();
    extra.push(0);
    assert_eq!(
        decode_datagram(&extra, 3).unwrap_err().code(),
        ErrorCode::ProtocolError
    );
    assert_eq!(
        decode_datagram(CURRENT, 2).unwrap_err().code(),
        ErrorCode::MessageTooLarge
    );
}
