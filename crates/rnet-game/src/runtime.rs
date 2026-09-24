//! Game facade orchestration over the transport runtime.

use crate::admission::SendAdmission;
use crate::clock_sync::ClockSyncTracker;
use crate::clock_sync_runtime::ClockSyncMetrics;
use crate::config::{
    GameClientConfig, GameHostClientConfig, GameProtocol, GameRuntimeConfig, GameServerConfig,
};
use crate::diagnostics::GameDiagnostics;
use crate::envelope::{decode, DecodedEnvelope};
use crate::event::{GameEvent, GameMessage};
use crate::heartbeat::HeartbeatTracker;
use crate::join;
use crate::observe::{HeartbeatMetrics, ProtocolMetrics, ResumeMetrics};
use crate::quality::{QualityPolicy, UdpSessionQuality};
use crate::range_state::RangeRuntimeState;
use crate::realtime::LatestQueue;
use crate::resume_runtime::ResumeRuntimeState;
use crate::scheduler::{GamePriority, ScheduledQueue};
use rnet_core::{ErrorCode, Event, EventType, Handle, Result, RnetError, Transport};
use rnet_protocol::control::SecurityMode;
use rnet_transport::{
    ClientConfig, ClientSecurity, HostClientConfig, NetworkRuntime, SecurityChange, ServerConfig,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

/// Optional network metadata. Business message typing remains inside `payload`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GameSendOptions {
    pub sequence: Option<u32>,
    pub tick: Option<u32>,
    /// Optional request/trace correlation metadata; zero means absent.
    pub correlation_id: u64,
    /// Local scheduling priority. Normal is the default and participates in bounded fair queuing.
    pub priority: GamePriority,
    /// Optional local staging lifetime. It cannot recall socket/KCP-owned data.
    pub expires_after: Option<Duration>,
}

/// High-level runtime whose normal send/receive path uses only session handles and payload bytes.
pub struct GameRuntime {
    pub(crate) network: NetworkRuntime,
    pub(crate) maximum_envelope_len: usize,
    pub(crate) server_protocols: Mutex<HashMap<Handle, GameProtocol>>,
    pub(crate) endpoint_transports: Mutex<HashMap<Handle, Transport>>,
    pub(crate) session_endpoints: Mutex<HashMap<Handle, Handle>>,
    pub(crate) udp_sessions: Mutex<HashMap<Handle, Arc<Mutex<UdpSessionQuality>>>>,
    pub(crate) heartbeat_interval: Duration,
    pub(crate) heartbeat_timeout: Duration,
    pub(crate) heartbeat_trackers: Mutex<HashMap<Handle, HeartbeatTracker>>,
    pub(crate) clock_origin: Instant,
    pub(crate) clock_trackers: Mutex<HashMap<Handle, ClockSyncTracker>>,
    pub(crate) clock_next_scan: Mutex<Instant>,
    pub(crate) clock_metrics: ClockSyncMetrics,
    pub(crate) admission: SendAdmission,
    pub(crate) heartbeat_metrics: HeartbeatMetrics,
    pub(crate) resume_metrics: ResumeMetrics,
    pub(crate) protocol_metrics: ProtocolMetrics,
    pub(crate) quality_policy: QualityPolicy,
    pub(crate) realtime: Mutex<LatestQueue>,
    pub(crate) scheduled: Mutex<ScheduledQueue>,
    pub(crate) resume: Mutex<ResumeRuntimeState>,
    pub(crate) range: Mutex<RangeRuntimeState>,
    pub(crate) realtime_flush_batch: usize,
    pub(crate) scheduled_flush_batch: usize,
    pub(crate) allow_plaintext_business_data: bool,
    pub(crate) diagnostics: GameDiagnostics,
    pub(crate) poll_guard: Mutex<()>,
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
        if config.network.max_body_len < join::HEADER_LEN.max(12 + crate::clock_sync::REPLY_LEN) {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "maximum body length cannot contain the game join header or clock reply",
            ));
        }
        HeartbeatTracker::new(
            config.heartbeat_interval,
            config.heartbeat_timeout,
            Instant::now(),
        )?;
        if !config.quality_policy.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "quality thresholds must be ordered and sample minima must be positive",
            ));
        }
        let maximum_envelope_len = config.network.max_body_len;
        let realtime = LatestQueue::from_config(config.realtime_queue)?;
        let realtime_flush_batch = config.realtime_queue.flush_batch;
        let scheduled = ScheduledQueue::from_config(config.scheduled_queue)?;
        let scheduled_flush_batch = config.scheduled_queue.flush_batch;
        let range_buffer_messages = config.network.event_queue_capacity;
        let range_buffer_bytes = config.network.max_event_bytes;
        let allow_plaintext_business_data =
            config.network.security_policy.allow_plaintext_business_data;
        let resume = ResumeRuntimeState::new(config.resume_ticket_ttl, config.max_resume_tickets)?;
        let network = NetworkRuntime::new_with_client_security(config.network, client_security)?;
        let clock_origin = Instant::now();
        Ok(Self {
            network,
            maximum_envelope_len,
            server_protocols: Mutex::new(HashMap::new()),
            endpoint_transports: Mutex::new(HashMap::new()),
            session_endpoints: Mutex::new(HashMap::new()),
            udp_sessions: Mutex::new(HashMap::new()),
            heartbeat_interval: config.heartbeat_interval,
            heartbeat_timeout: config.heartbeat_timeout,
            heartbeat_trackers: Mutex::new(HashMap::new()),
            clock_origin,
            clock_trackers: Mutex::new(HashMap::new()),
            clock_next_scan: Mutex::new(clock_origin),
            clock_metrics: ClockSyncMetrics::default(),
            admission: SendAdmission::new(),
            heartbeat_metrics: HeartbeatMetrics::default(),
            resume_metrics: ResumeMetrics::default(),
            protocol_metrics: ProtocolMetrics::default(),
            quality_policy: config.quality_policy,
            realtime: Mutex::new(realtime),
            scheduled: Mutex::new(scheduled),
            resume: Mutex::new(resume),
            range: Mutex::new(RangeRuntimeState::with_buffer_limits(
                range_buffer_messages,
                range_buffer_bytes,
            )),
            realtime_flush_batch,
            scheduled_flush_batch,
            allow_plaintext_business_data,
            diagnostics: GameDiagnostics::new(),
            poll_guard: Mutex::new(()),
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
        // Hold both maps until the endpoint is registered: a concurrent poll must not observe
        // an authentication or ready event before its game-level policy is installed.
        let mut protocols = self
            .server_protocols
            .lock()
            .expect("game protocol table poisoned");
        let mut transports = self
            .endpoint_transports
            .lock()
            .expect("game endpoint table poisoned");
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
        protocols.insert(endpoint, config.protocol);
        transports.insert(endpoint, config.transport);
        Ok(endpoint)
    }

    /// Starts a numeric-address client connection without exposing an encryption choice.
    pub fn connect(&self, config: GameClientConfig) -> Result<Handle> {
        let join_payload = join::encode(
            config.protocol,
            &config.join_ticket,
            self.maximum_envelope_len.min(60 * 1024),
        )?;
        self.connect_prepared(config, join_payload, None)
    }

    pub(crate) fn connect_prepared(
        &self,
        config: GameClientConfig,
        join_payload: Vec<u8>,
        old_session: Option<Handle>,
    ) -> Result<Handle> {
        let _join_ticket = Zeroizing::new(config.join_ticket);
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
        let mut transports = self
            .endpoint_transports
            .lock()
            .expect("game endpoint table poisoned");
        let endpoint = self.network.connect(ClientConfig {
            transport: config.transport,
            bind_addr,
            remote_addr: config.remote_addr,
            join_payload,
        })?;
        transports.insert(endpoint, config.transport);
        if let Some(old_session) = old_session {
            self.resume
                .lock()
                .expect("resume state poisoned")
                .client_endpoints
                .insert(endpoint, old_session);
        }
        Ok(endpoint)
    }

    /// Resolves a hostname and joins without exposing address-candidate or encryption plumbing.
    pub fn connect_host(&self, config: GameHostClientConfig) -> Result<Handle> {
        let join_payload = join::encode(
            config.protocol,
            &config.join_ticket,
            self.maximum_envelope_len.min(60 * 1024),
        )?;
        self.connect_host_prepared(config, join_payload, None)
    }

    pub(crate) fn connect_host_prepared(
        &self,
        config: GameHostClientConfig,
        join_payload: Vec<u8>,
        old_session: Option<Handle>,
    ) -> Result<Handle> {
        let _join_ticket = Zeroizing::new(config.join_ticket);
        let mut transports = self
            .endpoint_transports
            .lock()
            .expect("game endpoint table poisoned");
        let endpoint = self.network.connect_host(HostClientConfig {
            transport: config.transport,
            host: config.host,
            port: config.port,
            join_payload,
        })?;
        transports.insert(endpoint, config.transport);
        if let Some(old_session) = old_session {
            self.resume
                .lock()
                .expect("resume state poisoned")
                .client_endpoints
                .insert(endpoint, old_session);
        }
        Ok(endpoint)
    }

    pub fn endpoint_local_addr(&self, endpoint: Handle) -> Result<SocketAddr> {
        self.network.endpoint_local_addr(endpoint)
    }

    pub fn auth_decide(&self, session: Handle, accept: bool) -> Result<()> {
        self.network.auth_decide(session, accept)?;
        if !accept {
            if self
                .resume
                .lock()
                .expect("resume state poisoned")
                .pending_server
                .contains_key(&session)
            {
                self.resume_metrics
                    .authorization_denied
                    .fetch_add(1, Ordering::Relaxed);
            }
            self.forget_ready_session(session);
        }
        Ok(())
    }

    /// A completed transport handshake is insufficient until the facade has published Ready.
    pub(crate) fn ensure_game_ready(&self, session: Handle) -> Result<()> {
        if self.admission.contains(session) {
            Ok(())
        } else {
            // Distinguish a genuinely pending game session from a stale/closed handle. Callers
            // can then discard stale ownership without treating it as a recoverable handshake.
            self.network.validate_payload_len(session, 0)?;
            Err(RnetError::new(
                ErrorCode::HandshakeRequired,
                "game session is not ready",
            ))
        }
    }

    pub(crate) fn forget_game_ready(&self, session: Handle) {
        self.admission.remove(session);
    }

    pub(crate) fn forget_ready_session(&self, session: Handle) {
        self.session_endpoints
            .lock()
            .expect("game session table poisoned")
            .remove(&session);
        self.forget_session(session);
        self.forget_clock_session(session);
        self.forget_quality_session(session);
        self.forget_game_ready(session);
        self.realtime
            .lock()
            .expect("realtime queue poisoned")
            .forget_session(session);
        self.scheduled
            .lock()
            .expect("scheduled queue poisoned")
            .forget_session(session);
        self.forget_resume_session(session);
        self.range
            .lock()
            .expect("range state poisoned")
            .forget_session(session);
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
        self.send_scheduled(session, payload, options)
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

    pub(crate) fn convert_event(&self, mut event: Event) -> Result<Option<GameEvent>> {
        let converted = match event.event_type {
            EventType::RuntimeStarted => GameEvent::RuntimeStarted,
            EventType::EndpointOpened => GameEvent::EndpointOpened {
                endpoint: event.endpoint,
            },
            EventType::EndpointError => {
                self.reap_closed_endpoints();
                GameEvent::EndpointError {
                    endpoint: event.endpoint,
                    status: event.status,
                }
            }
            EventType::AuthRequest => {
                let converted = self.convert_game_auth_request(&event);
                event.data.zeroize();
                // Pending business authorization owns resume metadata before SessionOpened.
                // Track that ownership too, so losing a close event cannot leak pending state.
                if matches!(
                    &converted,
                    Ok(GameEvent::AuthRequest { .. } | GameEvent::ResumeRequest { .. })
                ) {
                    self.session_endpoints
                        .lock()
                        .expect("game session table poisoned")
                        .insert(event.session, event.endpoint);
                }
                converted?
            }
            EventType::SessionOpened => {
                let server_session = self
                    .server_protocols
                    .lock()
                    .expect("game protocol table poisoned")
                    .contains_key(&event.endpoint)
                    || self
                        .range
                        .lock()
                        .expect("range state poisoned")
                        .server_endpoints
                        .contains_key(&event.endpoint);
                let transports = self
                    .endpoint_transports
                    .lock()
                    .expect("game endpoint table poisoned");
                let transport = transports.get(&event.endpoint).copied();
                let Some(transport) = transport else {
                    let _ = self
                        .network
                        .close_session(event.session, ErrorCode::Cancelled);
                    return Ok(None);
                };
                // A queued SessionOpened can outlive a local kick or endpoint close. Never
                // publish it as game-ready after its transport route has been invalidated.
                if self.network.validate_payload_len(event.session, 0).is_err() {
                    return Ok(None);
                }
                let (old_session, server_resume) = {
                    let mut state = self.resume.lock().expect("resume state poisoned");
                    if let Some(claim) = state.pending_server.remove(&event.session) {
                        state
                            .inflight_server
                            .insert(claim.old_session, event.session);
                        (Some(claim.old_session), true)
                    } else {
                        (state.client_endpoints.remove(&event.endpoint), false)
                    }
                };
                self.track_session(event.session);
                self.track_clock_session(event.session, !server_session);
                self.track_quality_session(event.session, transport);
                self.session_endpoints
                    .lock()
                    .expect("game session table poisoned")
                    .insert(event.session, event.endpoint);
                if self.start_range_session(
                    event.endpoint,
                    event.session,
                    old_session,
                    server_resume,
                )? {
                    return Ok(None);
                }
                if let Some(old) = old_session {
                    // Exact-version wire v3 is ready at SessionOpened. Wire v4 returned above and
                    // defers this takeover until SELECT/ACK/READY publication.
                    let _ = self.network.close_session(old, ErrorCode::Cancelled);
                    self.forget_ready_session(old);
                }
                if server_resume {
                    let old = old_session.expect("server resume has an old session");
                    let mut state = self.resume.lock().expect("resume state poisoned");
                    let revoked = state.revoked_inflight.remove(&old);
                    if revoked || self.network.validate_payload_len(event.session, 0).is_err() {
                        state.inflight_server.remove(&old);
                        drop(state);
                        let _ = self
                            .network
                            .close_session(event.session, ErrorCode::Cancelled);
                        self.forget_ready_session(event.session);
                        return Ok(None);
                    }
                    // Ready publication and the final revocation check share this lock.
                    // A concurrent kick cannot observe a half-published takeover.
                    self.admission.publish(event.session);
                    state.inflight_server.remove(&old);
                    self.resume_metrics
                        .sessions_resumed
                        .fetch_add(1, Ordering::Relaxed);
                } else {
                    self.admission.publish(event.session);
                }
                drop(transports);
                if let Some(old_session) = old_session {
                    GameEvent::SessionResumed {
                        endpoint: event.endpoint,
                        old_session,
                        new_session: event.session,
                    }
                } else {
                    GameEvent::SessionReady {
                        endpoint: event.endpoint,
                        session: event.session,
                    }
                }
            }
            EventType::SessionClosed => {
                self.forget_ready_session(event.session);
                GameEvent::SessionClosed {
                    endpoint: event.endpoint,
                    session: event.session,
                    reason: event.status,
                }
            }
            EventType::GameControl => {
                let converted = self.handle_game_control_event(&event);
                event.data.zeroize();
                return converted;
            }
            EventType::Message => {
                // Values other than zero indicate a legacy or non-game peer. Accepting them would
                // reintroduce application routing fields that the game contract intentionally owns
                // inside its opaque payload.
                if event.msg_type != 0 || event.stream_id != 0 || event.status != ErrorCode::Ok {
                    return Err(RnetError::new(
                        ErrorCode::ProtocolError,
                        "game message contains legacy routing metadata",
                    ));
                }
                match decode(&event.data, self.maximum_envelope_len)? {
                    DecodedEnvelope::Application {
                        sequence,
                        tick,
                        datagram_sequence,
                        payload,
                    } => {
                        self.observe_datagram_sequence(event.session, datagram_sequence)?;
                        return self.buffer_range_message(GameMessage {
                            endpoint: event.endpoint,
                            session: event.session,
                            sequence,
                            tick,
                            correlation_id: event.request_id,
                            payload,
                        });
                    }
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
            EventType::JoinFailed => {
                self.resume
                    .lock()
                    .expect("resume state poisoned")
                    .client_endpoints
                    .remove(&event.endpoint);
                self.range
                    .lock()
                    .expect("range state poisoned")
                    .forget_failed_client_endpoint(event.endpoint);
                GameEvent::JoinFailed {
                    endpoint: event.endpoint,
                    session: event.session,
                    reason: event.status,
                }
            }
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
#[path = "runtime_tests.rs"]
mod tests;
