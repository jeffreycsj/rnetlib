use rnet_core::ErrorCode;
use rnet_protocol::control::{
    decode_control, decode_protected, decode_record, encode_control, encode_protected,
    encode_record, Control, ControlKind, ProtectedKind, ProtectedMessage, Record, RecordKind,
    SecurityMode,
};

#[test]
fn security_records_round_trip_without_exposing_business_framing() {
    let records = [
        Record::new(RecordKind::ClientHello, 0, b""),
        Record::new(RecordKind::ServerHello, 0, &[SecurityMode::Encrypted as u8]),
        Record::new(RecordKind::Handshake, 0, b"noise"),
        Record::new(RecordKind::Protected, 7, b"ciphertext"),
        Record::new(RecordKind::PlainData, 8, b"frame"),
    ];

    for record in records {
        let encoded = encode_record(&record, 1024).expect("encode record");
        assert_eq!(
            decode_record(&encoded, 1024).expect("decode record"),
            record
        );
    }
}

#[test]
fn protected_messages_keep_auth_data_and_controls_separate() {
    let messages = [
        ProtectedMessage::new(ProtectedKind::AuthDecision, &[1]),
        ProtectedMessage::new(ProtectedKind::Data, b"business-frame"),
        ProtectedMessage::new(
            ProtectedKind::Control,
            &encode_control(Control::switch(
                ControlKind::SwitchPropose,
                2,
                SecurityMode::Plaintext,
            )),
        ),
    ];
    for message in messages {
        let encoded = encode_protected(&message, 1024).expect("encode protected message");
        assert_eq!(
            decode_protected(&encoded, 1024).expect("decode protected message"),
            message
        );
    }

    assert!(decode_protected(&[0xff, 0, 0, 0, 0], 1024).is_err());
    assert_eq!(
        decode_protected(&[ProtectedKind::Data as u8, 0, 0, 0, 8], 4)
            .expect_err("declared payload exceeds limit")
            .code(),
        ErrorCode::MessageTooLarge
    );
}

#[test]
fn security_controls_round_trip_for_switch_and_rekey_barriers() {
    let controls = [
        Control::switch(ControlKind::SwitchPropose, 3, SecurityMode::Encrypted),
        Control::barrier(ControlKind::SwitchReady, 3),
        Control::barrier(ControlKind::SwitchCommit, 3),
        Control::barrier(ControlKind::SwitchAck, 3),
        Control::barrier(ControlKind::RekeyPropose, 4),
        Control::barrier(ControlKind::RekeyReady, 4),
        Control::barrier(ControlKind::RekeyCommit, 4),
        Control::barrier(ControlKind::RekeyAck, 4),
    ];

    for control in controls {
        let encoded = encode_control(control);
        assert_eq!(decode_control(&encoded).expect("decode control"), control);
    }
}

#[test]
fn malformed_security_records_are_rejected_before_allocation() {
    let valid = encode_record(&Record::new(RecordKind::Protected, 1, b"ciphertext"), 1024)
        .expect("encode record");

    for length in 0..valid.len() {
        let error = decode_record(&valid[..length], 1024).expect_err("truncated record");
        assert_eq!(error.code(), ErrorCode::ProtocolError);
    }

    let mut unknown_kind = valid.clone();
    unknown_kind[6] = 0xff;
    assert_eq!(
        decode_record(&unknown_kind, 1024)
            .expect_err("unknown kind")
            .code(),
        ErrorCode::ProtocolError
    );

    assert_eq!(
        decode_record(&valid, 4)
            .expect_err("oversized record")
            .code(),
        ErrorCode::MessageTooLarge
    );
}

#[test]
fn malformed_controls_and_zero_epoch_transitions_are_rejected() {
    for bytes in [&[][..], &[0xff][..], &[ControlKind::SwitchReady as u8][..]] {
        assert!(decode_control(bytes).is_err());
    }

    let encoded = encode_control(Control::barrier(ControlKind::SwitchCommit, 0));
    assert!(decode_control(&encoded).is_err());
}
