use super::envelope::{
    decode, encode_application, encode_control, ControlKind, DecodedEnvelope, HEADER_LEN,
};
use rnet_core::ErrorCode;

#[test]
fn application_payload_round_trips_without_a_public_message_type() {
    let payload = b"protobuf owns its business message type";

    let encoded = encode_application(payload, None, None, 1024).expect("encode application");

    assert_eq!(encoded[0], 0, "application kind is an internal body prefix");
    assert_eq!(&encoded[HEADER_LEN..], payload);
    assert_eq!(
        decode(&encoded, 1024).expect("decode application"),
        DecodedEnvelope::Application {
            sequence: None,
            tick: None,
            payload: payload.as_slice().into(),
        }
    );
}

#[test]
fn optional_sequence_and_tick_are_owned_by_the_network_envelope() {
    let encoded = encode_application(b"input", Some(u32::MAX), Some(42), 1024)
        .expect("encode application metadata");

    assert_eq!(
        decode(&encoded, 1024).expect("decode application metadata"),
        DecodedEnvelope::Application {
            sequence: Some(u32::MAX),
            tick: Some(42),
            payload: b"input".as_slice().into(),
        }
    );
}

#[test]
fn controls_use_internal_kinds_instead_of_the_business_api() {
    for kind in [
        ControlKind::Heartbeat,
        ControlKind::ClockSync,
        ControlKind::Resume,
        ControlKind::Protocol,
    ] {
        let encoded = encode_control(kind, b"control", 1024).expect("encode control");
        assert_ne!(encoded[0], 0);
        assert_eq!(
            decode(&encoded, 1024).expect("decode control"),
            DecodedEnvelope::Control {
                kind,
                payload: b"control".as_slice().into(),
            }
        );
    }
}

#[test]
fn malformed_or_unsupported_envelopes_are_rejected_before_payload_access() {
    let mut cases = vec![
        Vec::new(),
        vec![0; HEADER_LEN - 1],
        vec![0xff, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 0],
        vec![0, 0x80, 0, 12, 0, 0, 0, 0, 0, 0, 0, 0],
        vec![0, 0, 0, 11, 0, 0, 0, 0, 0, 0, 0, 0],
        vec![0, 0, 0, 13, 0, 0, 0, 0, 0, 0, 0, 0],
    ];
    let mut control_with_application_flags = vec![1, 1, 0, 12];
    control_with_application_flags.resize(HEADER_LEN, 0);
    cases.push(control_with_application_flags);

    for input in cases {
        assert_eq!(
            decode(&input, 1024).expect_err("malformed envelope").code(),
            ErrorCode::ProtocolError
        );
    }
}

#[test]
fn configured_length_limit_is_enforced_on_encode_and_decode() {
    assert_eq!(
        encode_application(b"too large", None, None, HEADER_LEN)
            .expect_err("encode length limit")
            .code(),
        ErrorCode::MessageTooLarge
    );

    let encoded = encode_application(b"payload", None, None, 1024).expect("encode");
    assert_eq!(
        decode(&encoded, encoded.len() - 1)
            .expect_err("decode length limit")
            .code(),
        ErrorCode::MessageTooLarge
    );
}
