use crate::{
    GameClientConfig, GameHostClientConfig, GameProfile, GameProtocol, GameProtocolRange,
    GameRangeClientConfig, GameRangeHostClientConfig, GameRangeServerConfig, GameServerConfig,
};
use rnet_core::Transport;
use rnet_security::Keypair;

#[test]
fn profiles_choose_stable_transports_and_encrypted_server_defaults() {
    for (profile, transport) in [
        (GameProfile::Realtime, Transport::Udp),
        (GameProfile::ReliableRealtime, Transport::Kcp),
        (GameProfile::Session, Transport::Tcp),
    ] {
        assert_eq!(profile.transport(), transport);
        let server = GameServerConfig::for_profile(
            profile,
            "127.0.0.1:0".parse().unwrap(),
            Keypair::generate().unwrap(),
            GameProtocol::new(7, 1),
        );
        assert_eq!(server.transport, transport);
        assert!(server.initial_encryption);
    }
}

#[test]
fn profile_constructors_cover_exact_range_numeric_and_hostname_clients() {
    let profile = GameProfile::ReliableRealtime;
    let remote = "127.0.0.1:9000".parse().unwrap();
    let exact = GameProtocol::new(8, 2);
    let range = GameProtocolRange::new(8, 2, 4);

    assert_eq!(
        GameClientConfig::for_profile(profile, remote, b"join".to_vec(), exact).transport,
        Transport::Kcp
    );
    assert_eq!(
        GameHostClientConfig::for_profile(
            profile,
            "game.example".to_owned(),
            9000,
            b"join".to_vec(),
            exact,
        )
        .transport,
        Transport::Kcp
    );
    assert_eq!(
        GameRangeClientConfig::for_profile(profile, remote, b"join".to_vec(), range).transport,
        Transport::Kcp
    );
    assert_eq!(
        GameRangeHostClientConfig::for_profile(
            profile,
            "game.example".to_owned(),
            9000,
            b"join".to_vec(),
            range,
        )
        .transport,
        Transport::Kcp
    );
    let server = GameRangeServerConfig::for_profile(
        profile,
        "127.0.0.1:0".parse().unwrap(),
        Keypair::generate().unwrap(),
        range,
    );
    assert_eq!(server.transport, Transport::Kcp);
    assert!(server.initial_encryption);
}
