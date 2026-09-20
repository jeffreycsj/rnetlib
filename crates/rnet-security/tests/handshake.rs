use rnet_security::{InitiatorHandshake, Keypair, ResponderHandshake, SecurityError};

#[test]
fn key_generation_returns_x25519_key_material() {
    let keypair = Keypair::generate().expect("key generation");
    assert_eq!(keypair.private.len(), 32);
    assert_eq!(keypair.public.len(), 32);
    assert_ne!(keypair.private, keypair.public);
}

#[test]
fn keypair_can_be_restored_from_its_private_key() {
    let generated = Keypair::generate().expect("key generation");
    let restored = Keypair::from_private(&generated.private).expect("restore keypair");

    assert_eq!(restored.private, generated.private);
    assert_eq!(restored.public, generated.public);
}

#[test]
fn keypair_restore_rejects_an_invalid_private_key_length() {
    assert_eq!(Keypair::from_private(&[7; 31]), Err(SecurityError::Crypto));
}

#[test]
fn initiator_requires_a_32_byte_pinned_server_key() {
    let client = Keypair::generate().expect("client key");
    let error = InitiatorHandshake::new(&client.private, &[7; 31])
        .err()
        .expect("invalid pinned key must fail");
    assert_eq!(error, SecurityError::PeerKeyMismatch);
}

#[test]
fn responder_requires_a_32_byte_private_key() {
    let error = ResponderHandshake::new(&[9; 31])
        .err()
        .expect("invalid private key must fail");
    assert_eq!(error, SecurityError::Crypto);
}

#[test]
fn xx_handshake_authenticates_server_and_encrypts_join_and_data() {
    let client_key = Keypair::generate().expect("client key");
    let server_key = Keypair::generate().expect("server key");
    let mut client =
        InitiatorHandshake::new(&client_key.private, &server_key.public).expect("initiator");
    let mut server = ResponderHandshake::new(&server_key.private).expect("responder");

    let first = client.write_first().expect("first message");
    server.read_first(&first).expect("read first");
    let response = server.write_response().expect("response");
    client.read_response(&response).expect("read response");
    let (finish, mut client_transport) = client.finish(b"join-token").expect("finish");
    let (join_payload, peer_key, mut server_transport) =
        server.finish(&finish).expect("server finish");

    assert_eq!(join_payload, b"join-token");
    assert_eq!(peer_key.as_slice(), client_key.public);
    let ciphertext = client_transport.encrypt(b"private-frame").expect("encrypt");
    assert_ne!(ciphertext, b"private-frame");
    assert_eq!(
        server_transport.decrypt(&ciphertext).expect("decrypt"),
        b"private-frame"
    );
}

#[test]
fn handshake_rejects_a_server_that_does_not_match_the_pinned_key() {
    let client_key = Keypair::generate().expect("client key");
    let pinned_server = Keypair::generate().expect("pinned server");
    let actual_server = Keypair::generate().expect("actual server");
    let mut client =
        InitiatorHandshake::new(&client_key.private, &pinned_server.public).expect("initiator");
    let mut server = ResponderHandshake::new(&actual_server.private).expect("responder");

    let first = client.write_first().expect("first message");
    server.read_first(&first).expect("read first");
    let response = server.write_response().expect("response");
    assert_eq!(
        client.read_response(&response),
        Err(SecurityError::PeerKeyMismatch)
    );
}

#[test]
fn transport_rejects_modified_ciphertext() {
    let client_key = Keypair::generate().expect("client key");
    let server_key = Keypair::generate().expect("server key");
    let mut client =
        InitiatorHandshake::new(&client_key.private, &server_key.public).expect("initiator");
    let mut server = ResponderHandshake::new(&server_key.private).expect("responder");
    let first = client.write_first().expect("first");
    server.read_first(&first).expect("read first");
    let response = server.write_response().expect("response");
    client.read_response(&response).expect("read response");
    let (finish, mut client_transport) = client.finish(&[]).expect("finish");
    let (_, _, mut server_transport) = server.finish(&finish).expect("server finish");

    let mut ciphertext = client_transport.encrypt(b"authenticated").expect("encrypt");
    let last = ciphertext.len() - 1;
    ciphertext[last] ^= 1;
    assert_eq!(
        server_transport.decrypt(&ciphertext),
        Err(SecurityError::Crypto)
    );
}

#[test]
fn datagram_transport_accepts_out_of_order_packets_and_rejects_replay() {
    let client_key = Keypair::generate().expect("client key");
    let server_key = Keypair::generate().expect("server key");
    let mut client =
        InitiatorHandshake::new(&client_key.private, &server_key.public).expect("initiator");
    let mut server = ResponderHandshake::new(&server_key.private).expect("responder");
    let first = client.write_first().expect("first");
    server.read_first(&first).expect("read first");
    let response = server.write_response().expect("response");
    client.read_response(&response).expect("read response");
    let (finish, mut client_transport) = client.finish_datagram(&[]).expect("client finish");
    let (_, _, mut server_transport) = server.finish_datagram(&finish).expect("server finish");

    let first_packet = client_transport.encrypt(b"first").expect("first packet");
    let second_packet = client_transport.encrypt(b"second").expect("second packet");
    let third_packet = client_transport.encrypt(b"third").expect("third packet");
    assert_eq!(
        server_transport
            .decrypt(&second_packet)
            .expect("second first"),
        b"second"
    );
    assert_eq!(
        server_transport.decrypt(&first_packet).expect("late first"),
        b"first"
    );
    assert_eq!(
        server_transport.decrypt(&first_packet),
        Err(SecurityError::ReplayDetected)
    );
    let mut tampered = third_packet.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert_eq!(
        server_transport.decrypt(&tampered),
        Err(SecurityError::Crypto)
    );
    assert_eq!(
        server_transport
            .decrypt(&third_packet)
            .expect("valid packet after forgery"),
        b"third"
    );
}
