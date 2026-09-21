use crate::config::{GameProtocol, GameProtocolRange};
use crate::join::{
    decode, decode_range, encode, encode_range, encode_range_resume, encode_resume, select_version,
};
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
    assert_eq!(&encoded[..4], b"RGV3", "wire v3 ordinary join marker");
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
    let mut old_version = encoded.clone();
    old_version[..4].copy_from_slice(b"RGJ2");
    invalid_cases.push(old_version);
    let mut old_resume = encoded.clone();
    old_resume[..4].copy_from_slice(b"RGJ3");
    invalid_cases.push(old_resume);
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

#[test]
fn resume_join_uses_explicit_magic_and_keeps_login_ticket_separate() {
    let protocol = GameProtocol::new(42, 2);
    let resume = [9_u8; 48];
    let encoded = encode_resume(protocol, b"login", &resume, 1024).unwrap();
    assert_eq!(&encoded[..4], b"RGR3", "wire v3 resume join marker");
    let decoded = decode(&encoded).unwrap();
    assert_eq!(decoded.ticket, b"login");
    assert_eq!(decoded.resume_ticket, Some(resume.as_slice()));
    let mut truncated = encoded.clone();
    truncated.pop();
    assert!(decode(&truncated).is_err());
    assert_eq!(
        decode(&encode(protocol, b"login", 1024).unwrap())
            .unwrap()
            .resume_ticket,
        None
    );
}

#[test]
fn v4_range_join_is_disjoint_from_exact_v3_and_selects_highest_common_version() {
    let requested = GameProtocolRange {
        protocol_id: 42,
        min_version: 3,
        max_version: 7,
        build_id: 11,
        capabilities: 9,
    };
    let nonce = [0x5a; 16];
    let encoded = encode_range(requested, nonce, b"login", 1024).unwrap();
    assert_eq!(&encoded[..4], b"RGV4");
    assert_eq!(
        decode(&encoded).unwrap_err().code(),
        ErrorCode::ProtocolError
    );
    let decoded = decode_range(&encoded).unwrap();
    assert_eq!(decoded.protocol, requested);
    assert_eq!(decoded.nonce, nonce);
    assert_eq!(decoded.ticket, b"login");
    assert_eq!(
        select_version(requested, GameProtocolRange::new(42, 5, 9)),
        Some(7)
    );
    assert_eq!(
        select_version(requested, GameProtocolRange::new(42, 8, 9)),
        None
    );
    assert_eq!(
        select_version(requested, GameProtocolRange::new(41, 3, 7)),
        None
    );
}

#[test]
fn v4_range_join_rejects_malformed_bounds_lengths_and_nonce() {
    let protocol = GameProtocolRange::new(42, 3, 7);
    let encoded = encode_range(protocol, [1; 16], b"x", 1024).unwrap();
    assert_eq!(
        encode_range(GameProtocolRange::new(42, 0, 7), [1; 16], b"", 1024)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        encode_range(GameProtocolRange::new(42, 8, 7), [1; 16], b"", 1024)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        encode_range(protocol, [0; 16], b"", 1024)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        encode_range(protocol, [1; 16], b"x", encoded.len() - 1)
            .unwrap_err()
            .code(),
        ErrorCode::MessageTooLarge
    );
    for broken in [
        encoded[..encoded.len() - 1].to_vec(),
        {
            let mut x = encoded.clone();
            x.push(0);
            x
        },
        {
            let mut x = encoded.clone();
            x[12..16].fill(0);
            x
        },
        {
            let mut x = encoded.clone();
            x[16..20].copy_from_slice(&2u32.to_be_bytes());
            x
        },
        {
            let mut x = encoded.clone();
            x[36..52].fill(0);
            x
        },
    ] {
        assert_eq!(
            decode_range(&broken).unwrap_err().code(),
            ErrorCode::ProtocolError
        );
    }
}

#[test]
fn v4_resume_marker_keeps_tickets_disjoint_and_rejects_truncation() {
    let protocol = GameProtocolRange::new(99, 2, 8);
    let encoded = encode_range_resume(protocol, [3; 16], b"login", &[7; 48], 1024).unwrap();
    assert_eq!(&encoded[..4], b"RGR4");
    assert!(decode(&encoded).is_err());
    let decoded = decode_range(&encoded).unwrap();
    assert_eq!(decoded.ticket, b"login");
    assert_eq!(decoded.resume_ticket, Some([7; 48].as_slice()));
    assert!(decode_range(&encoded[..encoded.len() - 1]).is_err());
    assert_eq!(
        encode_range_resume(protocol, [3; 16], b"", b"", 1024)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidArgument
    );
}
