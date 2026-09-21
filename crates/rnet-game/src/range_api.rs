//! Public endpoint operations for explicit wire-v4 version-range negotiation.

use crate::config::{
    GameClientConfig, GameHostClientConfig, GameProtocol, GameRangeClientConfig,
    GameRangeHostClientConfig, GameRangeServerConfig,
};
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
    /// Starts an explicit wire-v4 range listener; exact-version listeners retain wire v3.
    pub fn listen_range(&self, config: GameRangeServerConfig) -> Result<Handle> {
        if !config.protocol.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid protocol range",
            ));
        }
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
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

    /// Joins a range listener using a fresh nonce and a distinct wire-v4 marker.
    pub fn connect_range(&self, config: GameRangeClientConfig) -> Result<Handle> {
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
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        let endpoint = self.connect_prepared(
            GameClientConfig {
                transport: config.transport,
                bind_addr: config.bind_addr,
                remote_addr: config.remote_addr,
                join_ticket: config.join_ticket,
                protocol: GameProtocol::new(
                    config.protocol.protocol_id,
                    config.protocol.max_version,
                ),
            },
            payload,
            None,
        )?;
        self.range
            .lock()
            .expect("range state poisoned")
            .client_endpoints
            .insert(
                endpoint,
                ClientPending {
                    protocol: config.protocol,
                    nonce,
                },
            );
        Ok(endpoint)
    }

    /// Hostname variant of `connect_range` with the same authenticated version selection.
    pub fn connect_host_range(&self, config: GameRangeHostClientConfig) -> Result<Handle> {
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
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        let endpoint = self.connect_host_prepared(
            GameHostClientConfig {
                transport: config.transport,
                host: config.host,
                port: config.port,
                join_ticket: config.join_ticket,
                protocol: GameProtocol::new(
                    config.protocol.protocol_id,
                    config.protocol.max_version,
                ),
            },
            payload,
            None,
        )?;
        self.range
            .lock()
            .expect("range state poisoned")
            .client_endpoints
            .insert(
                endpoint,
                ClientPending {
                    protocol: config.protocol,
                    nonce,
                },
            );
        Ok(endpoint)
    }

    /// Resumes within this runtime; the ticket pins the original selected version.
    pub fn connect_range_resume(
        &self,
        config: GameRangeClientConfig,
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
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        let endpoint = self.connect_prepared(
            GameClientConfig {
                transport: config.transport,
                bind_addr: config.bind_addr,
                remote_addr: config.remote_addr,
                join_ticket: config.join_ticket,
                protocol: GameProtocol::new(
                    config.protocol.protocol_id,
                    config.protocol.max_version,
                ),
            },
            payload,
            Some(old_session),
        )?;
        self.range
            .lock()
            .expect("range state poisoned")
            .client_endpoints
            .insert(
                endpoint,
                ClientPending {
                    protocol: config.protocol,
                    nonce,
                },
            );
        Ok(endpoint)
    }

    /// Hostname variant of single-runtime wire-v4 resume.
    pub fn connect_host_range_resume(
        &self,
        config: GameRangeHostClientConfig,
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
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        let endpoint = self.connect_host_prepared(
            GameHostClientConfig {
                transport: config.transport,
                host: config.host,
                port: config.port,
                join_ticket: config.join_ticket,
                protocol: GameProtocol::new(
                    config.protocol.protocol_id,
                    config.protocol.max_version,
                ),
            },
            payload,
            Some(old_session),
        )?;
        self.range
            .lock()
            .expect("range state poisoned")
            .client_endpoints
            .insert(
                endpoint,
                ClientPending {
                    protocol: config.protocol,
                    nonce,
                },
            );
        Ok(endpoint)
    }

    /// Returns the negotiated version for a live wire-v4 session, including pending server auth.
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
