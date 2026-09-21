//! Game facade orchestration over the transport runtime.

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
use crate::observe::{HeartbeatMetrics, ResumeMetrics};
use crate::quality::{QualityPolicy, UdpSessionQuality};
use crate::range_runtime::RangeRuntimeState;
use crate::realtime::LatestQueue;
use crate::resume_runtime::ResumeRuntimeState;
use rnet_core::{ErrorCode, Event, EventType, Handle, Result, RnetError, Transport};
use rnet_protocol::control::SecurityMode;
use rnet_transport::{
    ClientConfig, ClientSecurity, HostClientConfig, NetworkRuntime, SecurityChange, ServerConfig,
};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

/// Optional network metadata. Business message typing remains inside `payload`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GameSendOptions {
    pub sequence: Option<u32>,
    pub tick: Option<u32>,
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
    pub(crate) ready_sessions: RwLock<HashSet<Handle>>,
    pub(crate) heartbeat_metrics: HeartbeatMetrics,
    pub(crate) resume_metrics: ResumeMetrics,
    pub(crate) quality_policy: QualityPolicy,
    pub(crate) realtime: Mutex<LatestQueue>,
    pub(crate) resume: Mutex<ResumeRuntimeState>,
    pub(crate) range: Mutex<RangeRuntimeState>,
    pub(crate) realtime_flush_batch: usize,
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
            ready_sessions: RwLock::new(HashSet::new()),
            heartbeat_metrics: HeartbeatMetrics::default(),
            resume_metrics: ResumeMetrics::default(),
            quality_policy: config.quality_policy,
            realtime: Mutex::new(realtime),
            resume: Mutex::new(resume),
            range: Mutex::new(RangeRuntimeState::default()),
            realtime_flush_batch,
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

    /// Closes an endpoint and releases its game protocol policy.
    pub fn close_endpoint(&self, endpoint: Handle) -> Result<()> {
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        // Keep listener policy and transport registration locked in the same order as listen.
        // SessionOpened conversion holds the transport lock through game-ready registration, so
        // an in-flight poll cannot recreate game state after this cleanup has taken its snapshot.
        let mut protocols = self
            .server_protocols
            .lock()
            .expect("game protocol table poisoned");
        let mut transports = self
            .endpoint_transports
            .lock()
            .expect("game endpoint table poisoned");
        self.network.close_endpoint(endpoint)?;
        self.resume
            .lock()
            .expect("resume state poisoned")
            .tickets
            .revoke_endpoint(endpoint);
        // Transport closure invalidates routes immediately. Do not keep game heartbeat or
        // replaceable-send state alive until the caller happens to poll close notifications.
        let sessions: Vec<_> = self
            .session_endpoints
            .lock()
            .expect("game session table poisoned")
            .iter()
            .filter_map(|(session, owner)| (*owner == endpoint).then_some(*session))
            .collect();
        for session in sessions {
            self.revoke_resume_session(session);
            self.forget_ready_session(session);
        }
        self.resume
            .lock()
            .expect("resume state poisoned")
            .client_endpoints
            .remove(&endpoint);
        protocols.remove(&endpoint);
        transports.remove(&endpoint);
        self.range
            .lock()
            .expect("range state poisoned")
            .forget_endpoint(endpoint);
        Ok(())
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
            self.forget_resume_session(session);
            self.range
                .lock()
                .expect("range state poisoned")
                .forget_session(session);
        }
        Ok(())
    }

    /// A completed transport handshake is insufficient until the facade has published Ready.
    pub(crate) fn ensure_game_ready(&self, session: Handle) -> Result<()> {
        if self
            .ready_sessions
            .read()
            .expect("game ready table poisoned")
            .contains(&session)
        {
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
        self.ready_sessions
            .write()
            .expect("game ready table poisoned")
            .remove(&session);
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
        self.send_game_application(session, payload, options)
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
        if capacity == 0 {
            return Vec::new();
        }
        // Game controls are handled by the same event queue as lifecycle changes. Serializing
        // polls preserves their order and keeps challenge state single-writer.
        let _guard = self.poll_guard.lock().expect("game poll lock poisoned");
        self.flush_realtime(self.realtime_flush_batch);
        let mut carried = Vec::new();
        self.range
            .lock()
            .expect("range state poisoned")
            .drain_completed(&mut carried, capacity);
        if !carried.is_empty() {
            self.drive_heartbeats();
            self.drive_clock_sync();
            self.drive_range_negotiation();
            return carried;
        }
        let deadline = Instant::now().checked_add(timeout);
        let mut internal_events = 0usize;
        loop {
            // Authenticated replies already queued by I/O workers take precedence over timeout.
            // Drain even when a batch contains only internal controls, so a small public capacity
            // cannot leave a timely acknowledgement stranded behind other control events.
            let buffered = self.network.poll_events(capacity, Duration::ZERO);
            if !buffered.is_empty() {
                internal_events = internal_events.saturating_add(buffered.len());
                let output = self.convert_events(buffered, capacity);
                // Hidden controls and public events alike must not defer liveness scheduling.
                // Conversion runs first so already queued authenticated acknowledgements win.
                self.drive_heartbeats();
                self.drive_clock_sync();
                self.drive_range_negotiation();
                if !output.is_empty() {
                    return output;
                }
                // A malicious authenticated peer cannot keep one poll call trapped forever by
                // continuously filling the queue with controls that are hidden from game code.
                if internal_events >= 1024 {
                    return Vec::new();
                }
                continue;
            }
            self.drive_heartbeats();
            self.drive_clock_sync();
            self.drive_range_negotiation();
            let remaining = deadline
                .map(|deadline| deadline.saturating_duration_since(Instant::now()))
                .unwrap_or(timeout);
            let wait = if self
                .heartbeat_trackers
                .lock()
                .expect("heartbeat table poisoned")
                .is_empty()
            {
                remaining
            } else {
                remaining.min(Duration::from_millis(50))
            };
            let events = self.network.poll_events(capacity, wait);
            internal_events = internal_events.saturating_add(events.len());
            let output = self.convert_events(events, capacity);
            self.drive_heartbeats();
            self.drive_clock_sync();
            self.drive_range_negotiation();
            if !output.is_empty()
                || timeout.is_zero()
                || deadline.is_some_and(|deadline| Instant::now() >= deadline)
                || internal_events >= 1024
            {
                return output;
            }
        }
    }

    fn convert_events(&self, events: Vec<Event>, capacity: usize) -> Vec<GameEvent> {
        let mut output = Vec::with_capacity(events.len());
        for event in events {
            let endpoint = event.endpoint;
            let session = event.session;
            let is_business_message = event.event_type == EventType::Message;
            let integrity_verified = event.integrity_verified;
            match self.convert_event(event) {
                Ok(Some(event)) => {
                    self.log_game_event(&event);
                    self.range
                        .lock()
                        .expect("range state poisoned")
                        .queue_public(event, &mut output, capacity);
                }
                Ok(None) => {}
                Err(_) => {
                    // Only an actual plaintext business record may be treated as untrusted
                    // input. A later encrypted record must still fail closed after a mode switch.
                    if session != 0
                        && !(self.allow_plaintext_business_data
                            && is_business_message
                            && !integrity_verified)
                    {
                        let _ = self
                            .network
                            .close_session(session, ErrorCode::ProtocolError);
                    }
                    let violation = GameEvent::ProtocolViolation { endpoint, session };
                    self.log_game_event(&violation);
                    self.range
                        .lock()
                        .expect("range state poisoned")
                        .queue_public(violation, &mut output, capacity);
                }
            }
            self.range
                .lock()
                .expect("range state poisoned")
                .drain_completed(&mut output, capacity);
        }
        output
    }

    pub(crate) fn convert_event(&self, mut event: Event) -> Result<Option<GameEvent>> {
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
                let converted = self.convert_game_auth_request(&event);
                event.data.zeroize();
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
                if let Some(old) = old_session {
                    // A valid ticket did not authorize the player. Only this post-authorization
                    // transport event transfers ownership and invalidates the old network route.
                    let _ = self.network.close_session(old, ErrorCode::Cancelled);
                    self.forget_ready_session(old);
                }
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
                    self.ready_sessions
                        .write()
                        .expect("game ready table poisoned")
                        .insert(event.session);
                    state.inflight_server.remove(&old);
                    self.resume_metrics
                        .sessions_resumed
                        .fetch_add(1, Ordering::Relaxed);
                } else {
                    self.ready_sessions
                        .write()
                        .expect("game ready table poisoned")
                        .insert(event.session);
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
                        datagram_sequence,
                        payload,
                    } => {
                        self.observe_datagram_sequence(event.session, datagram_sequence)?;
                        return self.buffer_range_message(GameMessage {
                            endpoint: event.endpoint,
                            session: event.session,
                            sequence,
                            tick,
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
mod tests {
    use super::*;
    use crate::envelope::{encode_control, ControlKind};
    use rnet_core::Transport;
    use rnet_security::Keypair;

    #[test]
    fn heartbeat_ticks_do_not_shorten_the_requested_poll_timeout() {
        let runtime = GameRuntime::new(
            GameRuntimeConfig::production()
                .with_heartbeat(Duration::from_secs(1), Duration::from_secs(3)),
        )
        .expect("runtime");
        runtime.poll(1, Duration::ZERO); // Consume RuntimeStarted before measuring an empty poll.
        runtime.track_session(u64::MAX);
        let started = Instant::now();
        assert!(runtime.poll(1, Duration::from_millis(130)).is_empty());
        assert!(started.elapsed() >= Duration::from_millis(110));
    }

    #[test]
    fn queued_public_events_do_not_starve_expired_heartbeat_checks() {
        let runtime = GameRuntime::new(
            GameRuntimeConfig::production()
                .with_heartbeat(Duration::from_millis(10), Duration::from_millis(100)),
        )
        .expect("runtime");
        let old = Instant::now() - Duration::from_secs(1);
        let mut tracker =
            HeartbeatTracker::new(Duration::from_millis(10), Duration::from_millis(100), old)
                .expect("tracker");
        tracker
            .mark_sent(7, old + Duration::from_millis(10))
            .expect("outstanding probe");
        runtime
            .heartbeat_trackers
            .lock()
            .expect("trackers")
            .insert(42, tracker);

        assert!(matches!(
            runtime.poll(1, Duration::ZERO).as_slice(),
            [GameEvent::RuntimeStarted]
        ));
        assert_eq!(runtime.heartbeat_metrics_snapshot().timeouts, 1);
    }

    #[test]
    fn runtime_rejects_invalid_heartbeat_policy_before_starting_threads() {
        let error = GameRuntime::new(
            GameRuntimeConfig::production().with_heartbeat(Duration::ZERO, Duration::from_secs(1)),
        )
        .err()
        .expect("zero heartbeat interval must be rejected");
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn runtime_rejects_unordered_quality_thresholds() {
        let policy = crate::quality::QualityPolicy {
            good_udp_loss_per_mille: 1,
            ..Default::default()
        };
        let error = GameRuntime::new(GameRuntimeConfig::production().with_quality_policy(policy))
            .err()
            .expect("unordered quality policy must be rejected");
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn runtime_rejects_invalid_realtime_queue_limits() {
        let queue = crate::realtime::RealtimeQueueConfig {
            flush_batch: 0,
            ..Default::default()
        };
        let error = GameRuntime::new(GameRuntimeConfig::production().with_realtime_queue(queue))
            .err()
            .expect("zero flush batch must be rejected");
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn udp_sequence_extension_is_required_only_on_udp_game_sessions() {
        let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");
        runtime.track_session(41);
        runtime.track_quality_session(41, Transport::Udp);
        assert_eq!(
            runtime
                .observe_datagram_sequence(41, None)
                .expect_err("UDP must carry the v2 sequence")
                .code(),
            ErrorCode::ProtocolError
        );
        runtime
            .observe_datagram_sequence(41, Some(0))
            .expect("first UDP sequence");
        assert_eq!(
            runtime
                .udp_loss_snapshot(41)
                .expect("UDP snapshot")
                .unwrap()
                .received,
            1
        );

        runtime.track_session(42);
        assert_eq!(
            runtime
                .observe_datagram_sequence(42, Some(0))
                .expect_err("TCP/KCP must reject a UDP-only extension")
                .code(),
            ErrorCode::ProtocolError
        );
        assert_eq!(
            runtime.udp_loss_snapshot(42).expect("non-UDP snapshot"),
            None
        );
    }

    #[test]
    fn plaintext_game_controls_cannot_impersonate_authenticated_controls() {
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
    fn malformed_plaintext_business_frame_does_not_close_a_ready_session() {
        let server_key = Keypair::generate().expect("server key");
        let client_key = Keypair::generate().expect("client key");
        let client_security = ClientSecurity::pinned(client_key, server_key.public.clone());
        let runtime = GameRuntime::new_with_client_security(
            GameRuntimeConfig::production().allow_plaintext_business_data(true),
            client_security,
        )
        .expect("runtime");
        let listener = runtime
            .listen(GameServerConfig {
                transport: Transport::Tcp,
                bind_addr: "127.0.0.1:0".parse().expect("address"),
                local_key: server_key,
                initial_encryption: false,
                protocol: GameProtocol::new(93, 1),
            })
            .expect("listener");
        let client_endpoint = runtime
            .connect(GameClientConfig {
                transport: Transport::Tcp,
                bind_addr: None,
                remote_addr: runtime.endpoint_local_addr(listener).expect("address"),
                join_ticket: b"ticket".to_vec(),
                protocol: GameProtocol::new(93, 1),
            })
            .expect("connect");
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut server_session = None;
        let mut client_session = None;
        while server_session.is_none() || client_session.is_none() {
            assert!(Instant::now() < deadline, "game session did not open");
            for event in runtime.poll(16, Duration::from_millis(10)) {
                match event {
                    GameEvent::AuthRequest { session, .. } => {
                        runtime.auth_decide(session, true).expect("authorize");
                    }
                    GameEvent::SessionReady { endpoint, session } if endpoint == listener => {
                        server_session = Some(session);
                    }
                    GameEvent::SessionReady { endpoint, session }
                        if endpoint == client_endpoint =>
                    {
                        client_session = Some(session);
                    }
                    _ => {}
                }
            }
        }
        let server_session = server_session.expect("server session");
        let client_session = client_session.expect("client session");
        let forged = encode_control(ControlKind::Heartbeat, b"", 1024).expect("control");
        runtime
            .network
            .send(client_session, 0, &forged)
            .expect("send forged business frame");
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut violation_seen = false;
        while !violation_seen {
            assert!(Instant::now() < deadline, "violation not reported");
            violation_seen = runtime
                .poll(16, Duration::from_millis(10))
                .iter()
                .any(|event| matches!(event, GameEvent::ProtocolViolation { session, .. } if *session == server_session));
        }
        runtime
            .send(client_session, b"still-alive")
            .expect("client remains connected");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            assert!(
                Instant::now() < deadline,
                "valid business message did not arrive"
            );
            if runtime
                .poll(16, Duration::from_millis(10))
                .iter()
                .any(|event| matches!(event, GameEvent::Message(message) if message.session == server_session && message.payload.as_ref() == b"still-alive"))
            {
                break;
            }
        }
        let unsupported = encode_control(ControlKind::ClockSync, b"", 1024)
            .expect("unsupported authenticated control");
        runtime
            .network
            .send_game_control(client_session, &unsupported)
            .expect("send authenticated control");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            assert!(
                Instant::now() < deadline,
                "authenticated violation not reported"
            );
            if runtime
                .poll(16, Duration::from_millis(10))
                .iter()
                .any(|event| matches!(event, GameEvent::ProtocolViolation { session, .. } if *session == server_session))
            {
                break;
            }
        }
        assert_eq!(
            runtime
                .network
                .validate_payload_len(server_session, 0)
                .expect_err("authenticated control violation closes session")
                .code(),
            ErrorCode::InvalidHandle
        );
    }

    #[test]
    fn encrypted_business_violation_closes_even_when_plaintext_is_permitted() {
        for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
            assert_encrypted_business_violation_closes(transport);
        }
    }

    fn assert_encrypted_business_violation_closes(transport: Transport) {
        let server_key = Keypair::generate().expect("server key");
        let client_key = Keypair::generate().expect("client key");
        let client_security = ClientSecurity::pinned(client_key, server_key.public.clone());
        let runtime = GameRuntime::new_with_client_security(
            GameRuntimeConfig::production().allow_plaintext_business_data(true),
            client_security,
        )
        .expect("runtime");
        let listener = runtime
            .listen(GameServerConfig {
                transport,
                bind_addr: "127.0.0.1:0".parse().expect("address"),
                local_key: server_key,
                initial_encryption: false,
                protocol: GameProtocol::new(94, 1),
            })
            .expect("listener");
        let client_endpoint = runtime
            .connect(GameClientConfig {
                transport,
                bind_addr: None,
                remote_addr: runtime.endpoint_local_addr(listener).expect("address"),
                join_ticket: b"ticket".to_vec(),
                protocol: GameProtocol::new(94, 1),
            })
            .expect("connect");
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut server_session = None;
        let mut client_session = None;
        while server_session.is_none() || client_session.is_none() {
            assert!(Instant::now() < deadline, "game session did not open");
            for event in runtime.poll(16, Duration::from_millis(10)) {
                match event {
                    GameEvent::AuthRequest { session, .. } => {
                        runtime.auth_decide(session, true).expect("authorize");
                    }
                    GameEvent::SessionReady { endpoint, session } if endpoint == listener => {
                        server_session = Some(session);
                    }
                    GameEvent::SessionReady { endpoint, session }
                        if endpoint == client_endpoint =>
                    {
                        client_session = Some(session);
                    }
                    _ => {}
                }
            }
        }
        let server_session = server_session.expect("server session");
        let client_session = client_session.expect("client session");
        runtime
            .set_encryption(server_session, true)
            .expect("enable encryption");
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut changed = 0;
        while changed < 2 {
            assert!(
                Instant::now() < deadline,
                "security transition did not complete"
            );
            changed += runtime
                .poll(16, Duration::from_millis(10))
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        GameEvent::SecurityChanged {
                            encrypted: true,
                            ..
                        }
                    )
                })
                .count();
        }
        let forged = encode_control(ControlKind::Heartbeat, b"", 1024).expect("control");
        runtime
            .network
            .send(client_session, 0, &forged)
            .expect("send authenticated business frame");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            assert!(Instant::now() < deadline, "violation not reported");
            if runtime
                .poll(16, Duration::from_millis(10))
                .iter()
                .any(|event| matches!(event, GameEvent::ProtocolViolation { session, .. } if *session == server_session))
            {
                break;
            }
        }
        assert_eq!(
            runtime
                .network
                .validate_payload_len(server_session, 0)
                .expect_err("authenticated violation closes session")
                .code(),
            ErrorCode::InvalidHandle
        );
    }

    #[test]
    fn queued_authenticated_control_after_local_close_is_not_a_protocol_violation() {
        let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");
        let mut event = Event::simple(EventType::GameControl);
        event.session = 42;
        event.data = crate::heartbeat::encode_heartbeat(
            crate::heartbeat::HeartbeatPacket {
                kind: crate::heartbeat::HeartbeatKind::Probe,
                challenge: 7,
            },
            1024,
        )
        .expect("heartbeat")
        .to_vec();
        assert_eq!(
            runtime
                .convert_event(event)
                .expect("stale control is ignored"),
            None
        );
        assert_eq!(
            runtime.heartbeat_metrics_snapshot().probes_rate_limited,
            0,
            "closed sessions must not be mistaken for rate-limited live sessions"
        );
    }

    #[test]
    fn unmatched_heartbeat_ack_does_not_enter_latency_distribution() {
        let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");
        runtime.track_session(42);
        let mut event = Event::simple(EventType::GameControl);
        event.session = 42;
        event.data = crate::heartbeat::encode_heartbeat(
            crate::heartbeat::HeartbeatPacket {
                kind: crate::heartbeat::HeartbeatKind::Ack,
                challenge: 7,
            },
            1024,
        )
        .expect("heartbeat")
        .to_vec();

        assert_eq!(runtime.convert_event(event).expect("ack ignored"), None);
        let metrics = runtime.heartbeat_metrics_snapshot();
        assert_eq!(metrics.replies_rejected, 1);
        assert_eq!(metrics.replies_matched, 0);
        assert_eq!(metrics.rtt.sample_count, 0);
        assert_eq!(runtime.drain_heartbeat_rtt_window().sample_count, 0);
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
    fn runtime_rejects_body_limits_that_cannot_carry_clock_reply() {
        let mut config = GameRuntimeConfig::production();
        config.network.max_body_len = 44;

        let error = GameRuntime::new(config)
            .err()
            .expect("impossible protected clock reply must fail at construction");
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }
}
