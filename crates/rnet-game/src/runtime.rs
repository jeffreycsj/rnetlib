//! Game facade orchestration over the transport runtime.

use crate::admission::SendAdmission;
use crate::clock_sync::ClockSyncTracker;
use crate::clock_sync_runtime::ClockSyncMetrics;
use crate::config::{GameClientConfig, GameHostClientConfig, GameRuntimeConfig};
use crate::diagnostics::GameDiagnostics;
use crate::event::GameEvent;
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
    ClientConfig, ClientSecurity, HostClientConfig, NetworkRuntime, SecurityChange,
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
    /// Optional application-supplied sequence, copied unchanged to the receiver.
    /// `None` omits it; `Some(0)` is valid. It does not enable ordering or deduplication.
    pub sequence: Option<u32>,
    /// Optional simulation tick, copied unchanged; unrelated to the library's clock samples.
    pub tick: Option<u32>,
    /// Optional request/trace correlation metadata; zero means absent.
    pub correlation_id: u64,
    /// Local scheduling priority. Normal is the default and participates in bounded fair queuing.
    pub priority: GamePriority,
    /// Local staging lifetime measured from admission. `None` never expires; zero expires
    /// before forwarding. It cannot recall socket/KCP-owned data or set a remote deadline.
    pub expires_after: Option<Duration>,
}

/// High-level runtime whose normal send/receive path uses only session handles and payload bytes.
///
/// The same runtime can listen and, with client security configured, initiate connections.
/// It owns I/O workers but has no autonomous game loop: keep calling [`Self::poll`] to drive
/// authorization events, negotiation, heartbeats, clock samples, staged sends and diagnostics.
///
/// The type is `Send + Sync`. Calls may be made from multiple threads, but poll, explicit
/// flush and close/stop operations serialize on one lock; use one regular poll consumer.
/// A handle belongs to its originating runtime and is not a durable player identity.
pub struct GameRuntime {
    pub(crate) network: NetworkRuntime,
    pub(crate) maximum_envelope_len: usize,
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
    pub(crate) diagnostics: GameDiagnostics,
    pub(crate) poll_guard: Mutex<()>,
    // Registration and its first event conversion must be atomic, but registration must not
    // wait for an entire blocking poll. When both are needed, acquire poll_guard first.
    pub(crate) endpoint_registration: Mutex<()>,
}

impl GameRuntime {
    /// Creates a runtime capable of listening, without outbound client trust configuration.
    /// `config` is consumed and validated; listeners are created separately with [`Self::listen`].
    /// Returns `InvalidArgument` for invalid budgets/policies, or an initialization error.
    /// Use [`Self::new_with_client_security`] when this runtime must also connect as a client.
    pub fn new(config: GameRuntimeConfig) -> Result<Self> {
        Self::build(config, None)
    }

    /// Creates a runtime capable of making client connections with explicit server trust.
    /// `client_security` supplies the client identity and server verification policy used by
    /// outbound connections. It does not prevent this runtime from also hosting listeners.
    /// Configuration and security initialization failures are returned before any join starts.
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
        let resume = ResumeRuntimeState::new(config.resume_ticket_ttl, config.max_resume_tickets)?;
        let network = NetworkRuntime::new_with_client_security(config.network, client_security)?;
        let clock_origin = Instant::now();
        Ok(Self {
            network,
            maximum_envelope_len,
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
            diagnostics: GameDiagnostics::new(),
            poll_guard: Mutex::new(()),
            endpoint_registration: Mutex::new(()),
        })
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

    /// Returns the actual local address of `endpoint`, including an OS-assigned port.
    /// `endpoint` must be a live listen/connect handle, not a session; stale handles fail.
    pub fn endpoint_local_addr(&self, endpoint: Handle) -> Result<SocketAddr> {
        self.network.endpoint_local_addr(endpoint)
    }

    /// Answers one [`GameEvent::AuthRequest`] or [`GameEvent::ResumeRequest`] on the server.
    /// `session` is the pending handle from that event; `accept` is the application's decision.
    /// Acceptance continues negotiation: wait for Ready/Resumed before sending game data.
    /// Rejection closes the pending connection. Unknown handles return `InvalidHandle`;
    /// repeated, expired or non-pending decisions return `InvalidState`.
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
        // Cleanup is repeatable: I/O failure, local kick and negotiation expiry may all reach
        // it. Remove readiness before clearing staging so an admitted send finishes under the
        // admission lock before its queue is emptied; later sends cannot repopulate that queue.
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
    ///
    /// `session` must be game-ready. `payload` is copied into bounded local staging and may
    /// be empty; the caller may reuse its buffer on return. Success is admission, not delivery.
    /// Keep polling or call [`Self::flush_scheduled`] to forward queued messages.
    ///
    /// Returns `HandshakeRequired` before readiness, `InvalidHandle` for stale sessions,
    /// `InvalidState` while stopping, `MessageTooLarge` for payload plus envelope exceeding
    /// transport limits, or `WouldBlock` for full game staging. Transport errors after admission
    /// appear in queue counters/logs; disconnection can discard already accepted messages.
    pub fn send(&self, session: Handle, payload: &[u8]) -> Result<()> {
        self.send_with_options(session, payload, GameSendOptions::default())
    }

    /// Stages `payload` for `session` using optional metadata, priority and local expiry.
    /// Ownership, readiness and admission errors match [`Self::send`]. `options` is copied;
    /// an expiry that cannot form a monotonic deadline returns `InvalidArgument`.
    /// Priority may reorder messages across classes; it is not a transport priority or delivery SLA.
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
    /// `session` is a local server-side handle; `enabled = false` requires explicit runtime
    /// plaintext permission or returns `NotSupported`. A client-side handle returns `InvalidState`.
    /// Success queues a request, not a completed transition: observe [`GameEvent::SecurityChanged`].
    /// Full command queues return `WouldBlock`; stale handles return `InvalidHandle`.
    /// A request for the already active mode may be a no-op, without a new change event.
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
    /// `session` must be server-controlled. Success queues a rotation; completion is reported
    /// by [`GameEvent::SecurityChanged`]. Invalid handles/roles and full command queues fail
    /// as in [`Self::set_encryption`]. This cannot recover a compromised application identity.
    pub fn rekey(&self, session: Handle) -> Result<()> {
        self.network.rekey_session(session)
    }

    pub(crate) fn convert_event(&self, mut event: Event) -> Result<Option<GameEvent>> {
        let _registration = matches!(
            event.event_type,
            EventType::AuthRequest | EventType::SessionOpened
        )
        .then(|| {
            self.endpoint_registration
                .lock()
                .expect("endpoint registration poisoned")
        });
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
                // Move the claim into negotiation ownership, without invalidating the old
                // route. Transport-open is only the start of SELECT/ACK/READY, not takeover.
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
                self.start_range_session(
                    event.endpoint,
                    event.session,
                    old_session,
                    server_resume,
                )?;
                return Ok(None);
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
            EventType::Message => return self.convert_business_event(&event),
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
