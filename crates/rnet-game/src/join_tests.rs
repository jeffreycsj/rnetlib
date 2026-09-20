use crate::config::GameProtocol;
use crate::join::{decode, encode};
use rnet_core::ErrorCode;

#[test]
fn authenticated_join_metadata_round_trips_without_exposing_ticket_prefix() {
    let protocol = GameProtocol {
        protocol_id: 0x1234_5678,
        version: 9,
        build_id: 42,
        capabilities: 0b101,
    };
    let encoded = encode(protocol, b"opaque-ticket", 1024).expect("encode join");
    let decoded = decode(&encoded).expect("decode join");
    assert_eq!(decoded.protocol, protocol);
    assert_eq!(decoded.ticket, b"opaque-ticket");
}

#[test]
fn malformed_join_lengths_and_magic_are_rejected() {
    let encoded = encode(GameProtocol::new(1, 1), b"ticket", 1024).expect("encode join");
    let mut invalid_cases = vec![
        Vec::new(),
        encoded[..33].to_vec(),
        encoded[..encoded.len() - 1].to_vec(),
    ];
    let mut wrong_magic = encoded.clone();
    wrong_magic[0] ^= 1;
    invalid_cases.push(wrong_magic);
    let mut trailing = encoded.clone();
    trailing.push(0);
    invalid_cases.push(trailing);
    let mut zero_version = encoded.clone();
    zero_version[12..16].fill(0);
    invalid_cases.push(zero_version);

    for input in invalid_cases {
        assert_eq!(
            decode(&input).expect_err("invalid join").code(),
            ErrorCode::ProtocolError
        );
    }
}

#[test]
fn ticket_size_is_bounded_before_allocation() {
    assert_eq!(
        encode(GameProtocol::new(1, 1), &[0; 10], 40)
            .expect_err("join is too large")
            .code(),
        ErrorCode::MessageTooLarge
    );
    assert_eq!(
        encode(GameProtocol::new(0, 1), b"", 40)
            .expect_err("zero ID is reserved")
            .code(),
        ErrorCode::InvalidArgument
    );
}
