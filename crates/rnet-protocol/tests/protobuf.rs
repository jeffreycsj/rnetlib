use prost::Message;
use rnet_core::ErrorCode;
use rnet_protocol::{decode_protobuf, encode_protobuf};

#[derive(Clone, PartialEq, Message)]
struct ControlMessage {
    #[prost(uint32, tag = "1")]
    version: u32,
}

#[test]
fn protobuf_round_trip_uses_caller_selected_message_type() {
    let message = ControlMessage { version: 7 };
    let bytes = encode_protobuf(&message, 64).unwrap();
    let decoded: ControlMessage = decode_protobuf(&bytes, 64).unwrap();
    assert_eq!(decoded, message);
}

#[test]
fn protobuf_decoder_ignores_unknown_fields() {
    let mut bytes = encode_protobuf(&ControlMessage { version: 3 }, 64)
        .unwrap()
        .to_vec();
    bytes.extend_from_slice(&[0x10, 0x63]);

    let decoded: ControlMessage = decode_protobuf(&bytes, 64).unwrap();
    assert_eq!(decoded.version, 3);
}

#[test]
fn protobuf_size_limit_is_checked_before_decode() {
    let error = decode_protobuf::<ControlMessage>(&[0; 9], 8).unwrap_err();
    assert_eq!(error.code(), ErrorCode::MessageTooLarge);
}
