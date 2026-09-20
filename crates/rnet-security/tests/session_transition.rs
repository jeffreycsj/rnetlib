use rnet_protocol::control::{ControlKind, SecurityMode};
use rnet_security::transition::{Effect, Role, SecurityController};

#[test]
fn server_controls_plaintext_to_encrypted_and_back() {
    let mut server = SecurityController::new(Role::Server, SecurityMode::Plaintext, 1);
    let mut client = SecurityController::new(Role::Client, SecurityMode::Plaintext, 1);

    for target in [SecurityMode::Encrypted, SecurityMode::Plaintext] {
        let propose = server.begin_switch(target).expect("server proposes switch");
        assert_eq!(propose.kind, ControlKind::SwitchPropose);
        assert!(
            client.begin_switch(target).is_err(),
            "client cannot initiate"
        );

        let ready = client.handle(propose).expect("client prepares");
        assert_eq!(
            client.mode(),
            if target == SecurityMode::Encrypted {
                SecurityMode::Plaintext
            } else {
                SecurityMode::Encrypted
            }
        );
        let commit = server
            .handle(ready.response.expect("ready response"))
            .expect("server commits");
        assert_eq!(commit.after_response, Effect::None);
        let ack = client
            .handle(commit.response.expect("commit response"))
            .expect("client activates");
        assert_eq!(ack.before_response, Effect::SetMode(target));
        assert_eq!(client.mode(), target);
        let complete = server
            .handle(ack.response.expect("ack response"))
            .expect("server completes");
        assert_eq!(complete.before_response, Effect::SetMode(target));
        assert_eq!(server.mode(), target);
        assert_eq!(server.epoch(), client.epoch());
    }
}

#[test]
fn rekey_effects_happen_on_opposite_sides_of_the_commit_send() {
    let mut server = SecurityController::new(Role::Server, SecurityMode::Encrypted, 9);
    let mut client = SecurityController::new(Role::Client, SecurityMode::Encrypted, 9);

    let propose = server.begin_rekey().expect("server proposes rekey");
    let ready = client.handle(propose).expect("client prepares");
    let commit = server
        .handle(ready.response.expect("ready"))
        .expect("server sends commit");
    assert_eq!(commit.before_response, Effect::None);
    assert_eq!(commit.after_response, Effect::Rekey);

    let ack = client
        .handle(commit.response.expect("commit"))
        .expect("client rekeys before ack");
    assert_eq!(ack.before_response, Effect::Rekey);
    assert_eq!(ack.after_response, Effect::None);
    let complete = server
        .handle(ack.response.expect("ack"))
        .expect("server completes rekey");
    assert_eq!(complete.before_response, Effect::AdvanceEpoch);
    assert_eq!(server.epoch(), client.epoch());
}

#[test]
fn stale_out_of_order_and_concurrent_transitions_are_rejected() {
    let mut server = SecurityController::new(Role::Server, SecurityMode::Encrypted, 3);
    let mut client = SecurityController::new(Role::Client, SecurityMode::Encrypted, 3);

    let propose = server
        .begin_switch(SecurityMode::Plaintext)
        .expect("proposal");
    assert!(
        server.begin_rekey().is_err(),
        "only one transition may be active"
    );

    let mut stale = propose;
    stale.epoch = 3;
    assert!(client.handle(stale).is_err());

    let mut skipped = propose;
    skipped.epoch = 5;
    assert!(client.handle(skipped).is_err());
}
