//! Game facade orchestration over the transport runtime.

use crate::config::{
    GameClientConfig, GameHostClientConfig, GameProtocol, GameRuntimeConfig, GameServerConfig,
};
use crate::envelope::{decode, encode_application, DecodedEnvelope};
use crate::event::{GameEvent, GameMessage};
use crate::join;
use rnet_core::{ErrorCode, Event, EventType, Handle, Result, RnetError, Transport};
use rnet_protocol::control::SecurityMode;
use rnet_transport::{
    AuthRequest, ClientConfig, ClientSecurity, HostClientConfig, NetworkRuntime, SecurityChange,
    ServerConfig,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

/// Optional network metadata. Business message typing remains inside `payload`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GameSendOptions {
    pub sequence: Option<u32>,
    pub tick: Option<u32>,
}

/// High-level runtime whose normal send/receive path uses only session handles and payload bytes.
pub struct GameRuntime {
    pub(crate) network: NetworkRuntime,
    maximum_envelope_len: usize,
    server_protocols: Mutex<HashMap<Handle, GameProtocol>>,
}

impl GameRuntime {
    /// Creates a server-only or externally authenticated game runtime.
    pub fn new(config: GameRuntimeConfig) -> Result<Self> {
        Self::build(config, None)
    }

    /// Creates a runtime capable of making client connections with pinned server trust.
    pub fn new_with_client_security(
        config: GameRuntimeConfig,
        client_security: ClientSecurity,
    ) -> Result<Self> {
        Self::build(config, Some(client_security))
    }

    fn build(config: GameRuntimeConfig, client_security: Option<ClientSecurity>) -> Result<Self> {
        if config.network.max_body_len < join::HEADER_LEN {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "maximum body length cannot contain the game join header",
            ));
        }
        let maximum_envelope_len = config.network.max_body_len;
        let network = NetworkRuntime::new_with_client_security(config.network, client_security)?;
        Ok(Self {
            network,
            maximum_envelope_len,
            server_protocols: Mutex::new(HashMap::new()),
        })
    }

    /// Starts a listener. The selected transport remains immutable for every accepted session.
    pub fn listen(&self, config: GameServerConfig) -> Result<Handle> {
        if !config.protocol.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "game protocol ID and version must be positive",
            ));
        }
        let endpoint = self.network.listen(ServerConfig {
            transport: config.transport,
            bind_addr: config.bind_addr,
            local_key: config.local_key,
            initial_security: if config.initial_encryption {
                SecurityMode::Encrypted
            } else {
                SecurityMode::Plaintext
            },
        })?;
        self.server_protocols
            .lock()
            .expect("game protocol table poisoned")
            .insert(endpoint, config.protocol);
        Ok(endpoint)
    }

    /// Starts a numeric-address client connection without exposing an encryption choice.
    pub fn connect(&self, config: GameClientConfig) -> Result<Handle> {
        let join_payload = join::encode(
            config.protocol,
            &config.join_ticket,
            self.maximum_envelope_len.min(60 * 1024),
        )?;
        // Datagram clients normally do not care which local interface or ephemeral port is used.
        // Select a wildcard of the matching address family so the public API can keep that detail
        // optional without creating an IPv4/IPv6 mismatch.
        let bind_addr = config.bind_addr.or_else(|| {
            (config.transport != Transport::Tcp).then(|| {
                SocketAddr::new(
                    if config.remote_addr.is_ipv4() {
                        std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
                    } else {
                        std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
                    },
                    0,
                )
            })
        });
        self.network.connect(ClientConfig {
            transport: config.transport,
            bind_addr,
            remote_addr: config.remote_addr,
            join_payload,
        })
    }

    /// Resolves a hostname and joins without exposing address-candidate or encryption plumbing.
    pub fn connect_host(&self, config: GameHostClientConfig) -> Result<Handle> {
        let join_payload = join::encode(
            config.protocol,
            &config.join_ticket,
            self.maximum_envelope_len.min(60 * 1024),
        )?;
        self.network.connect_host(HostClientConfig {
            transport: config.transport,
            host: config.host,
            port: config.port,
            join_payload,
        })
    }

    /// Closes an endpoint and releases its game protocol policy.
    pub fn close_endpoint(&self, endpoint: Handle) -> Result<()> {
        self.network.close_endpoint(endpoint)?;
        self.server_protocols
            .lock()
            .expect("game protocol table poisoned")
            .remove(&endpoint);
        Ok(())
    }

    pub fn endpoint_local_addr(&self, endpoint: Handle) -> Result<SocketAddr> {
        self.network.endpoint_local_addr(endpoint)
    }

    pub fn auth_decide(&self, session: Handle, accept: bool) -> Result<()> {
        self.network.auth_decide(session, accept)
    }

    /// Sends opaque business bytes. Message typing belongs to the serialized payload.
    pub fn send(&self, session: Handle, payload: &[u8]) -> Result<()> {
        self.send_with_options(session, payload, GameSendOptions::default())
    }

    /// Sends opaque business bytes with optional network-owned tick/sequence metadata.
    pub fn send_with_options(
        &self,
        session: Handle,
        payload: &[u8],
        options: GameSendOptions,
    ) -> Result<()> {
        let envelope = encode_application(
            payload,
            options.sequence,
            options.tick,
            self.maximum_envelope_len,
        )?;
        self.network.send_payload(session, &envelope)
    }

    /// Changes business-data encryption on an established server-side session.
    ///
    /// Clients cannot initiate this transition; the transport runtime enforces the server role and
    /// carries the change over its always-authenticated control path.
    pub fn set_encryption(&self, session: Handle, enabled: bool) -> Result<()> {
        self.network.set_security_mode(
            session,
            if enabled {
                SecurityMode::Encrypted
            } else {
                SecurityMode::Plaintext
            },
        )
    }

    /// Rotates session keys on the server without changing the business-data encryption mode.
    /// Like mode changes, the request travels through the authenticated control path.
    pub fn rekey(&self, session: Handle) -> Result<()> {
        self.network.rekey_session(session)
    }

    /// Polls typed game events. Unsupported internal controls fail closed until implemented.
    pub fn poll(&self, capacity: usize, timeout: Duration) -> Vec<GameEvent> {
        let events = self.network.poll_events(capacity, timeout);
        let mut output = Vec::with_capacity(events.len());
        for event in events {
            let endpoint = event.endpoint;
            let session = event.session;
            match self.convert_event(event) {
                Ok(Some(event)) => output.push(event),
                Ok(None) => {}
                Err(_) => {
                    if session != 0 {
                        let _ = self
                            .network
                            .close_session(session, ErrorCode::ProtocolError);
                    }
                    output.push(GameEvent::ProtocolViolation { endpoint, session });
                }
            }
        }
        output
    }

    fn convert_event(&self, event: Event) -> Result<Option<GameEvent>> {
        let converted = match event.event_type {
            EventType::RuntimeStarted => GameEvent::RuntimeStarted,
            EventType::EndpointOpened => GameEvent::EndpointOpened {
                endpoint: event.endpoint,
            },
            EventType::EndpointError => GameEvent::EndpointError {
                endpoint: event.endpoint,
                status: event.status,
            },
            EventType::AuthRequest => {
                let request = AuthRequest::from_event(&event)?;
                let expected = self
                    .server_protocols
                    .lock()
                    .expect("game protocol table poisoned")
                    .get(&event.endpoint)
                    .copied();
                let actual = join::decode(request.join_payload);
                match (expected, actual) {
                    (Some(expected), Ok(actual))
                        if expected.protocol_id == actual.protocol.protocol_id
                            && expected.version == actual.protocol.version =>
                    {
                        GameEvent::AuthRequest {
                            endpoint: event.endpoint,
                            session: event.session,
                            client_public_key: request.client_public_key,
                            join_ticket: actual.ticket.to_vec(),
                            build_id: actual.protocol.build_id,
                            capabilities: actual.protocol.capabilities,
                        }
                    }
                    _ => {
                        // Denial happens while the transport still awaits application approval.
                        // Invalid joins never reach game authentication or consume a ready session.
                        let _ = self.network.auth_decide(event.session, false);
                        GameEvent::ProtocolRejected {
                            endpoint: event.endpoint,
                            session: event.session,
                            reason: ErrorCode::ProtocolError,
                        }
                    }
                }
            }
            EventType::SessionOpened => GameEvent::SessionReady {
                endpoint: event.endpoint,
                session: event.session,
            },
            EventType::SessionClosed => GameEvent::SessionClosed {
                endpoint: event.endpoint,
                session: event.session,
                reason: event.status,
            },
            EventType::Message => {
                // Values other than zero indicate a legacy or non-game peer. Accepting them would
                // reintroduce application routing fields that the game contract intentionally owns
                // inside its opaque payload.
                if event.msg_type != 0
                    || event.stream_id != 0
                    || event.request_id != 0
                    || event.status != ErrorCode::Ok
                {
                    return Err(RnetError::new(
                        ErrorCode::ProtocolError,
                        "game message contains legacy routing metadata",
                    ));
                }
                match decode(&event.data, self.maximum_envelope_len)? {
                    DecodedEnvelope::Application {
                        sequence,
                        tick,
                        payload,
                    } => GameEvent::Message(GameMessage {
                        endpoint: event.endpoint,
                        session: event.session,
                        sequence,
                        tick,
                        payload,
                    }),
                    // A future control state machine must explicitly consume each kind. Silently
                    // ignoring a known kind today would let peers believe heartbeat, resume, or
                    // protocol synchronization succeeded when none of those paths is active.
                    DecodedEnvelope::Control { .. } => {
                        return Err(RnetError::new(
                            ErrorCode::ProtocolError,
                            "unsupported game control for this runtime version",
                        ));
                    }
                }
            }
            EventType::Writable => GameEvent::Writable {
                endpoint: event.endpoint,
                session: event.session,
            },
            EventType::RuntimeStopped => GameEvent::RuntimeStopped,
            EventType::JoinFailed => GameEvent::JoinFailed {
                endpoint: event.endpoint,
                session: event.session,
                reason: event.status,
            },
            EventType::SecurityChanged => {
                let change = SecurityChange::from_event(&event)?;
                GameEvent::SecurityChanged {
                    endpoint: event.endpoint,
                    session: event.session,
                    encrypted: change.mode == SecurityMode::Encrypted,
                    epoch: change.epoch,
                    operation: change.operation,
                }
            }
        };
        Ok(Some(converted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{encode_control, ControlKind};
    use rnet_core::Transport;
    use rnet_security::Keypair;

    #[test]
    fn unimplemented_internal_controls_are_not_silently_accepted() {
        let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");
        let mut event = Event::simple(EventType::Message);
        event.session = 1;
        event.data = encode_control(ControlKind::Heartbeat, b"", 1024)
            .expect("control")
            .to_vec();

        let error = runtime
            .convert_event(event)
            .expect_err("unhandled control must fail closed");
        assert_eq!(error.code(), ErrorCode::ProtocolError);
    }

    #[test]
    fn transient_endpoint_error_does_not_erase_listener_protocol_gate() {
        let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");
        let endpoint = runtime
            .listen(GameServerConfig {
                transport: Transport::Tcp,
                bind_addr: "127.0.0.1:0".parse().expect("address"),
                local_key: Keypair::generate().expect("key"),
                initial_encryption: true,
                protocol: GameProtocol::new(12, 1),
            })
            .expect("listener");
        let mut error = Event::simple(EventType::EndpointError);
        error.endpoint = endpoint;
        error.status = ErrorCode::IoError;

        assert!(matches!(
            runtime.convert_event(error),
            Ok(Some(GameEvent::EndpointError { .. }))
        ));
        assert_eq!(
            runtime
                .server_protocols
                .lock()
                .expect("protocol table")
                .get(&endpoint),
            Some(&GameProtocol::new(12, 1))
        );

        runtime.close_endpoint(endpoint).expect("close listener");
        assert!(!runtime
            .server_protocols
            .lock()
            .expect("protocol table")
            .contains_key(&endpoint));
    }

    #[test]
    fn runtime_rejects_body_limits_that_cannot_carry_game_join_metadata() {
        let mut config = GameRuntimeConfig::production();
        config.network.max_body_len = 33;

        let error = GameRuntime::new(config)
            .err()
            .expect("impossible game handshake must fail at construction");
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }
}
