//! Public endpoint operations for current-wire version negotiation.

use crate::config::{GameClientConfig, GameHostClientConfig, GameServerConfig};
use crate::join;
use crate::range_state::ClientPending;
use crate::resume::TICKET_LEN;
use crate::runtime::GameRuntime;
use ring::rand::{SecureRandom, SystemRandom};
use rnet_core::{ErrorCode, Handle, Result, RnetError};
use rnet_protocol::control::SecurityMode;
use rnet_transport::ServerConfig;

fn nonce() -> Result<[u8; 16]> {
    let mut nonce = [0; 16];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| RnetError::new(ErrorCode::CryptoError, "protocol nonce RNG failed"))?;
    if nonce == [0; 16] {
        return Err(RnetError::new(
            ErrorCode::CryptoError,
            "protocol nonce RNG returned zero",
        ));
    }
    Ok(nonce)
}

impl GameRuntime {
    /// Starts a listener supporting the inclusive application version range.
    /// Transport is fixed for its sessions; initial encryption remains server-controlled.
    /// `config` supplies the local address, server identity and business protocol policy.
    /// Returns an endpoint handle, not a player session; poll and answer authorization events.
    /// Invalid protocol ranges, forbidden plaintext mode, exhausted endpoint budgets and bind
    /// failures return errors. Port zero is resolved via [`Self::endpoint_local_addr`].
    pub fn listen(&self, config: GameServerConfig) -> Result<Handle> {
        if !config.protocol.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid protocol range",
            ));
        }
        let _registration = self
            .endpoint_registration
            .lock()
            .expect("endpoint registration poisoned");
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
        self.range
            .lock()
            .expect("range state poisoned")
            .server_endpoints
            .insert(endpoint, config.protocol);
        self.endpoint_transports
            .lock()
            .expect("game endpoint table poisoned")
            .insert(endpoint, config.transport);
        Ok(endpoint)
    }

    /// Starts a numeric-address join using a fresh authenticated negotiation nonce.
    /// Success returns an endpoint, not a ready session; both peers must continue polling.
    /// `config` is consumed, including the login ticket. Client identity and trusted server key
    /// must have been configured at runtime creation. Protocol/size/security validation and
    /// initial socket failures are returned; later join failures arrive through polling.
    pub fn connect(&self, config: GameClientConfig) -> Result<Handle> {
        if !config.protocol.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid protocol range",
            ));
        }
        let nonce = nonce()?;
        let payload = join::encode_range(
            config.protocol,
            nonce,
            &config.join_ticket,
            self.maximum_envelope_len.min(60 * 1024),
        )?;
        let _registration = self
            .endpoint_registration
            .lock()
            .expect("endpoint registration poisoned");
        let protocol = config.protocol;
        let endpoint = self.connect_prepared(config, payload, None)?;
        self.range
            .lock()
            .expect("range state poisoned")
            .client_endpoints
            .insert(endpoint, ClientPending { protocol, nonce });
        Ok(endpoint)
    }

    /// Hostname variant of `connect` with the same authenticated version selection.
    /// `config.host` is a hostname or IP, not an HTTP URL; `config.port` is separate and nonzero.
    /// DNS resolution can block up to the configured DNS deadline and can fail synchronously.
    /// A returned endpoint still needs authorization and game negotiation before it can send.
    pub fn connect_host(&self, config: GameHostClientConfig) -> Result<Handle> {
        if !config.protocol.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid protocol range",
            ));
        }
        let nonce = nonce()?;
        let payload = join::encode_range(
            config.protocol,
            nonce,
            &config.join_ticket,
            self.maximum_envelope_len.min(60 * 1024),
        )?;
        let _registration = self
            .endpoint_registration
            .lock()
            .expect("endpoint registration poisoned");
        let protocol = config.protocol;
        let endpoint = self.connect_host_prepared(config, payload, None)?;
        self.range
            .lock()
            .expect("range state poisoned")
            .client_endpoints
            .insert(endpoint, ClientPending { protocol, nonce });
        Ok(endpoint)
    }

    /// Resumes within this runtime; the ticket pins the original selected version.
    /// The old handle is local mapping metadata, not authorization. The server verifies its
    /// ticket and authorizes again; a still-live old session is replaced only after game readiness.
    /// `config` supplies the new connection and fresh application credential; `old_session` is
    /// the nonzero previous local handle, even if disconnected. `ticket` is borrowed for this
    /// call and must be the opaque server-issued resume credential, not a login ticket.
    /// Returns a new endpoint; completion is [`crate::GameEvent::SessionResumed`]. Malformed
    /// inputs fail synchronously; expiry, replay or server rejection may fail asynchronously.
    /// The issuing server runtime must still exist. A failed attempt can consume the one-use
    /// ticket even when the still-live old connection is preserved.
    pub fn connect_resume(
        &self,
        config: GameClientConfig,
        old_session: Handle,
        ticket: &[u8],
    ) -> Result<Handle> {
        if old_session == 0 || ticket.len() != TICKET_LEN || !config.protocol.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid range resume request",
            ));
        }
        let nonce = nonce()?;
        let payload = join::encode_range_resume(
            config.protocol,
            nonce,
            &config.join_ticket,
            ticket,
            self.maximum_envelope_len.min(60 * 1024),
        )?;
        let _registration = self
            .endpoint_registration
            .lock()
            .expect("endpoint registration poisoned");
        let protocol = config.protocol;
        let endpoint = self.connect_prepared(config, payload, Some(old_session))?;
        self.range
            .lock()
            .expect("range state poisoned")
            .client_endpoints
            .insert(endpoint, ClientPending { protocol, nonce });
        Ok(endpoint)
    }

    /// Hostname variant of single-runtime wire-v4 resume.
    /// `config` follows [`Self::connect_host`]; `old_session` and borrowed `ticket` follow
    /// [`Self::connect_resume`]. Resolution errors are synchronous; a returned endpoint is not
    /// proof of successful recovery. Server-side runtime restart invalidates old tickets.
    pub fn connect_host_resume(
        &self,
        config: GameHostClientConfig,
        old_session: Handle,
        ticket: &[u8],
    ) -> Result<Handle> {
        if old_session == 0 || ticket.len() != TICKET_LEN || !config.protocol.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid range resume request",
            ));
        }
        let nonce = nonce()?;
        let payload = join::encode_range_resume(
            config.protocol,
            nonce,
            &config.join_ticket,
            ticket,
            self.maximum_envelope_len.min(60 * 1024),
        )?;
        let _registration = self
            .endpoint_registration
            .lock()
            .expect("endpoint registration poisoned");
        let protocol = config.protocol;
        let endpoint = self.connect_host_prepared(config, payload, Some(old_session))?;
        self.range
            .lock()
            .expect("range state poisoned")
            .client_endpoints
            .insert(endpoint, ClientPending { protocol, nonce });
        Ok(endpoint)
    }

    /// Returns the negotiated version for a live wire-v4 session, including pending server auth.
    /// `session` is the local session handle; `InvalidHandle` also means no selection exists yet.
    /// A selected version is not proof of authorization or readiness to send business data.
    pub fn selected_protocol_version(&self, session: Handle) -> Result<u32> {
        self.range
            .lock()
            .expect("range state poisoned")
            .sessions
            .get(&session)
            .and_then(|state| state.selected)
            .ok_or_else(|| {
                RnetError::new(
                    ErrorCode::InvalidHandle,
                    "no selected range version for session",
                )
            })
    }
}
