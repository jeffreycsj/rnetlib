use rnet_core::ErrorCode;
use rnet_protocol::{encode_frame, FrameCodec, HEADER_LEN, MAGIC, VERSION};

#[test]
fn incremental_parser_waits_for_header_and_body() {
    let encoded = encode_frame(11, 3, 99, 0, b"hello", 1024).unwrap();
    let mut codec = FrameCodec::new(1024).unwrap();

    assert!(codec.push(&encoded[..HEADER_LEN - 1]).unwrap().is_empty());
    assert!(codec
        .push(&encoded[HEADER_LEN - 1..HEADER_LEN + 2])
        .unwrap()
        .is_empty());
    let frames = codec.push(&encoded[HEADER_LEN + 2..]).unwrap();

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].header.msg_type, 11);
    assert_eq!(frames[0].header.stream_id, 3);
    assert_eq!(frames[0].header.request_id, 99);
    assert_eq!(&frames[0].body[..], b"hello");
}

#[test]
fn incremental_parser_extracts_multiple_sticky_frames() {
    let first = encode_frame(1, 0, 10, 0, b"a", 1024).unwrap();
    let second = encode_frame(2, 0, 20, 0, b"bc", 1024).unwrap();
    let mut bytes = first.to_vec();
    bytes.extend_from_slice(&second);

    let frames = FrameCodec::new(1024).unwrap().push(&bytes).unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].header.msg_type, 1);
    assert_eq!(frames[1].header.msg_type, 2);
    assert_eq!(&frames[1].body[..], b"bc");
}

#[test]
fn exact_maximum_body_is_allowed_and_one_byte_more_is_rejected() {
    assert!(encode_frame(1, 0, 0, 0, &[0; 8], 8).is_ok());
    let error = encode_frame(1, 0, 0, 0, &[0; 9], 8).unwrap_err();
    assert_eq!(error.code(), ErrorCode::MessageTooLarge);
}

#[test]
fn parser_rejects_oversized_length_before_body_arrives() {
    let mut header = encode_frame(1, 0, 0, 0, b"", 1024).unwrap().to_vec();
    header[16..20].copy_from_slice(&9_u32.to_be_bytes());

    let error = FrameCodec::new(8).unwrap().push(&header).unwrap_err();
    assert_eq!(error.code(), ErrorCode::MessageTooLarge);
}

#[test]
fn parser_rejects_invalid_magic_version_and_flags() {
    let valid = encode_frame(1, 0, 0, 0, b"", 8).unwrap();
    let cases = [
        (0..4, 0_u32.to_be_bytes().to_vec()),
        (4..6, (VERSION + 1).to_be_bytes().to_vec()),
        (6..8, 1_u16.to_be_bytes().to_vec()),
    ];

    for (range, replacement) in cases {
        let mut bytes = valid.to_vec();
        bytes[range].copy_from_slice(&replacement);
        let error = FrameCodec::new(8).unwrap().push(&bytes).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ProtocolError);
    }
    assert_eq!(u32::from_be_bytes(valid[..4].try_into().unwrap()), MAGIC);
}

#[test]
fn zero_maximum_body_configuration_is_rejected() {
    assert_eq!(
        FrameCodec::new(0).unwrap_err().code(),
        ErrorCode::InvalidArgument
    );
}
