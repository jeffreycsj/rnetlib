//! Runtime-owned routing and resource reservations shared by socket tasks and public calls.
//!
//! A session route exists before the application handshake finishes, but `established` gates
//! business sends. Pending-handshake accounting is released exactly once when the route becomes
//! established or is removed. Outbound byte reservations follow the queued message by ownership,
//! so every queue rejection, socket failure, and normal dequeue releases both budgets.

use crate::config::RuntimeConfig;
use crate::metrics::AdmissionRejectReason;
use crate::metrics::Latencies;
use crate::metrics::Metrics;
use bytes::Bytes;
use rnet_core::ErrorCode;
use rnet_core::Event;
use rnet_core::EventQueue;
use rnet_core::EventType;
use rnet_core::Handle;
use rnet_core::HandleTable;
use rnet_core::Lifecycle;
use rnet_core::Result;
use rnet_core::RnetError;
use rnet_core::RuntimeState;
use rnet_security::SecurityError;
use std::future::pending;
use std::net::SocketAddr;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug)]
pub(crate) enum SecurityCommand {
    SetMode(rnet_protocol::control::SecurityMode),
    Rekey,
}

pub(crate) async fn receive_security_command(
    commands: &mut Option<mpsc::Receiver<SecurityCommand>>,
) -> Option<SecurityCommand> {
    match commands {
        Some(receiver) => receiver.recv().await,
        None => pending().await,
    }
}

pub(crate) async fn wait_for_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => pending().await,
    }
}
use tokio::task::AbortHandle;
use tokio::time::sleep;

pub(crate) struct EndpointRecord {
    pub(crate) local_addr: SocketAddr,
    pub(crate) transport: rnet_core::Transport,
    pub(crate) abort: Option<AbortHandle>,
}

pub(crate) struct SessionRoute {
    pub(crate) endpoint: Handle,
    pub(crate) target: SessionTarget,
    /// Only a completed Noise handshake plus server authorization sets this flag.
    pub(crate) established: bool,
    pub(crate) auth_decision: Option<oneshot::Sender<bool>>,
    pub(crate) security_commands: Option<mpsc::Sender<SecurityCommand>>,
    /// Only adaptive Noise sessions may carry game controls outside the business-data mode.
    pub(crate) allows_game_controls: bool,
    pub(crate) queued_bytes: Arc<ByteBudget>,
}

pub(crate) struct Outbound {
    pub(crate) bytes: Bytes,
    pub(crate) kind: OutboundKind,
    pub(crate) queued_at: Instant,
    /// Dropping an outbound message releases its runtime and session reservations together.
    _reservations: Vec<ByteReservation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutboundKind {
    Data,
    GameControl,
}

#[derive(Debug)]
pub(crate) struct ByteBudget {
    limit: usize,
    used: AtomicUsize,
    peak: AtomicUsize,
}

#[derive(Debug)]
struct ByteReservation {
    budget: Arc<ByteBudget>,
    bytes: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DatagramCleanup {
    pub(crate) peer: SocketAddr,
    pub(crate) session: Handle,
}

impl Outbound {
    #[cfg(test)]
    pub(crate) fn new(bytes: Bytes) -> Self {
        Self {
            bytes,
            kind: OutboundKind::Data,
            queued_at: Instant::now(),
            _reservations: Vec::new(),
        }
    }

    pub(crate) fn with_budgets(
        bytes: Bytes,
        runtime: &Arc<ByteBudget>,
        session: &Arc<ByteBudget>,
    ) -> Result<Self> {
        Self::with_kind(bytes, OutboundKind::Data, runtime, session)
    }

    pub(crate) fn with_game_control(
        bytes: Bytes,
        runtime: &Arc<ByteBudget>,
        session: &Arc<ByteBudget>,
    ) -> Result<Self> {
        Self::with_kind(bytes, OutboundKind::GameControl, runtime, session)
    }

    fn with_kind(
        bytes: Bytes,
        kind: OutboundKind,
        runtime: &Arc<ByteBudget>,
        session: &Arc<ByteBudget>,
    ) -> Result<Self> {
        let size = bytes.len();
        // The first reservation is automatically rolled back if the second one fails.
        let runtime_reservation = runtime.reserve(size)?;
        let session_reservation = session.reserve(size)?;
        Ok(Self {
            bytes,
            kind,
            queued_at: Instant::now(),
            _reservations: vec![runtime_reservation, session_reservation],
        })
    }
}

impl ByteBudget {
    pub(crate) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            used: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        })
    }

    fn reserve(self: &Arc<Self>, bytes: usize) -> Result<ByteReservation> {
        let mut current = self.used.load(Ordering::Relaxed);
        loop {
            let Some(next) = current.checked_add(bytes) else {
                return Err(RnetError::new(
                    ErrorCode::WouldBlock,
                    "send byte budget overflow",
                ));
            };
            if next > self.limit {
                return Err(RnetError::new(
                    ErrorCode::WouldBlock,
                    "send byte budget exhausted",
                ));
            }
            match self.used.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.peak.fetch_max(next, Ordering::Relaxed);
                    return Ok(ByteReservation {
                        budget: Arc::clone(self),
                        bytes,
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    pub(crate) fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    pub(crate) fn peak(&self) -> usize {
        self.peak.load(Ordering::Relaxed)
    }
}

impl Drop for ByteReservation {
    fn drop(&mut self) {
        // Queue ownership is the accounting lease; no separate asynchronous release path exists.
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[derive(Clone)]
pub(crate) enum SessionTarget {
    Tcp(mpsc::Sender<Outbound>),
    Udp {
        sender: mpsc::Sender<(SocketAddr, Outbound)>,
        peer: SocketAddr,
        max_frame_len: usize,
        cleanup: Option<mpsc::UnboundedSender<DatagramCleanup>>,
    },
    Kcp {
        sender: mpsc::Sender<(SocketAddr, Outbound)>,
        peer: SocketAddr,
        cleanup: Option<mpsc::UnboundedSender<DatagramCleanup>>,
    },
}

impl SessionTarget {
    pub(crate) fn request_cleanup(&self, session: Handle) {
        let addressed = match self {
            Self::Udp { peer, cleanup, .. } | Self::Kcp { peer, cleanup, .. } => {
                cleanup.as_ref().map(|sender| (*peer, sender))
            }
            Self::Tcp(_) => None,
        };
        if let Some((peer, sender)) = addressed {
            let _ = sender.send(DatagramCleanup { peer, session });
        }
    }
}

pub(crate) struct Shared {
    pub(crate) config: RuntimeConfig,
    pub(crate) client_security: Option<crate::config::ClientSecurity>,
    pub(crate) state: RuntimeState,
    pub(crate) events: EventQueue,
    pub(crate) endpoints: Mutex<HandleTable<EndpointRecord>>,
    pub(crate) sessions: Mutex<HandleTable<SessionRoute>>,
    pub(crate) metrics: Metrics,
    pub(crate) latencies: Latencies,
    pub(crate) send_budget: Arc<ByteBudget>,
    pub(crate) stopped_event_emitted: Mutex<bool>,
}

pub(crate) fn mark_session_established(shared: &Arc<Shared>, session: Handle) -> Result<()> {
    let mut sessions = shared.sessions.lock().expect("session table poisoned");
    let route = sessions
        .get_mut(session)
        .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid session"))?;
    if !route.established {
        route.established = true;
        shared.metrics.release_pending_handshake();
    }
    route.auth_decision = None;
    Ok(())
}

pub(crate) fn insert_session_route(shared: &Arc<Shared>, route: SessionRoute) -> Result<Handle> {
    let pending = !route.established;
    if pending
        && !shared
            .metrics
            .try_reserve_pending_handshake(shared.config.max_pending_handshakes)
    {
        shared
            .metrics
            .record_admission_rejected(AdmissionRejectReason::PendingHandshakeLimit);
        return Err(RnetError::new(
            ErrorCode::WouldBlock,
            "runtime pending-handshake limit reached",
        ));
    }
    let handle = shared
        .sessions
        .lock()
        .expect("session table poisoned")
        .insert(route);
    Ok(handle)
}

pub(crate) fn release_pending_session(shared: &Arc<Shared>, route: &SessionRoute) {
    if !route.established {
        shared.metrics.release_pending_handshake();
    }
}

pub(crate) fn map_security_error(error: SecurityError) -> RnetError {
    let code = match error {
        SecurityError::PeerKeyMismatch => ErrorCode::PeerKeyMismatch,
        SecurityError::InvalidState => ErrorCode::HandshakeFailed,
        SecurityError::Crypto => ErrorCode::CryptoError,
        SecurityError::ReplayDetected => ErrorCode::ReplayDetected,
    };
    RnetError::new(code, error.to_string())
}

pub(crate) fn fail_secure_session(
    shared: &Arc<Shared>,
    endpoint: Handle,
    session: Handle,
    error: RnetError,
) {
    let reason = error.code();
    let mut event = session_event(EventType::JoinFailed, endpoint, session);
    event.status = reason;
    publish_lifecycle(shared, event);
    remove_session_with_reason(shared, endpoint, session, reason);
}

pub(crate) fn session_active(shared: &Arc<Shared>, session: Handle) -> bool {
    shared
        .sessions
        .lock()
        .expect("session table poisoned")
        .get(session)
        .is_some()
}

pub(crate) fn retarget_datagram_session(
    shared: &Arc<Shared>,
    session: Handle,
    peer: SocketAddr,
) -> Result<()> {
    let mut sessions = shared.sessions.lock().expect("session table poisoned");
    let route = sessions
        .get_mut(session)
        .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid session"))?;
    match &mut route.target {
        SessionTarget::Udp { peer: target, .. } | SessionTarget::Kcp { peer: target, .. } => {
            *target = peer;
            Ok(())
        }
        SessionTarget::Tcp(_) => Err(RnetError::new(
            ErrorCode::InvalidState,
            "TCP session cannot switch a datagram address",
        )),
    }
}

pub(crate) fn retarget_datagram_endpoint(
    shared: &Arc<Shared>,
    endpoint: Handle,
    local_addr: SocketAddr,
) -> Result<()> {
    let mut endpoints = shared.endpoints.lock().expect("endpoint table poisoned");
    let record = endpoints
        .get_mut(endpoint)
        .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid endpoint"))?;
    record.local_addr = local_addr;
    Ok(())
}

pub(crate) fn remove_session(shared: &Arc<Shared>, endpoint: Handle, session: Handle) {
    remove_session_with_reason(shared, endpoint, session, ErrorCode::Ok);
}

pub(crate) fn remove_session_with_reason(
    shared: &Arc<Shared>,
    _endpoint: Handle,
    session: Handle,
    reason: ErrorCode,
) {
    if let Some(route) = shared
        .sessions
        .lock()
        .expect("session table poisoned")
        .remove(session)
    {
        release_pending_session(shared, &route);
        shared.metrics.record_session_closed(reason);
        route.target.request_cleanup(session);
        let mut event = session_event(EventType::SessionClosed, route.endpoint, session);
        event.status = reason;
        publish_lifecycle(shared, event);
    }
}

/// Publishes a bounded lifecycle notification, preferring it over stale data notifications.
pub(crate) fn publish_lifecycle(shared: &Arc<Shared>, event: Event) {
    match shared.events.push_priority(event) {
        Ok(true) => {
            shared
                .metrics
                .events_dropped
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(false) => {}
        Err(_) => {
            shared
                .metrics
                .lifecycle_events_rejected
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Publishes a per-session event without ever waiting on a shared datagram driver.
///
/// Queue saturation terminates only the offending session; a priority close notification makes
/// the overload explicit to the consumer and prevents one slow consumer from freezing peers.
pub(crate) fn try_push_session_event(shared: &Arc<Shared>, event: Event) -> Result<()> {
    if shared.events.try_push(event.clone()).is_ok() {
        return Ok(());
    }
    shared
        .metrics
        .events_dropped
        .fetch_add(1, Ordering::Relaxed);
    if event.session != 0 {
        remove_session(shared, event.endpoint, event.session);
    }
    Err(RnetError::new(
        ErrorCode::WouldBlock,
        "event queue is full; overloaded session was closed",
    ))
}

pub(crate) async fn push_tcp_event(shared: &Arc<Shared>, event: Event) {
    loop {
        if matches!(
            event.event_type,
            EventType::Message | EventType::Writable | EventType::GameControl
        ) && !session_active(shared, event.session)
        {
            return;
        }
        if shared.events.try_push(event.clone()).is_ok() {
            return;
        }
        if shared.state.load() != Lifecycle::Running {
            shared
                .metrics
                .events_dropped
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        sleep(Duration::from_millis(1)).await;
    }
}

pub(crate) fn session_event(event_type: EventType, endpoint: Handle, session: Handle) -> Event {
    let mut event = Event::simple(event_type);
    event.endpoint = endpoint;
    event.session = session;
    event
}

pub(crate) fn message_event(
    endpoint: Handle,
    session: Handle,
    frame: rnet_protocol::Frame,
) -> Event {
    Event {
        event_type: EventType::Message,
        endpoint,
        session,
        msg_type: frame.header.msg_type,
        stream_id: frame.header.stream_id,
        request_id: frame.header.request_id,
        status: ErrorCode::Ok,
        data: frame.body.to_vec(),
        queued_at: Instant::now(),
    }
}

pub(crate) fn game_control_event(endpoint: Handle, session: Handle, data: Vec<u8>) -> Event {
    let mut event = session_event(EventType::GameControl, endpoint, session);
    event.data = data;
    event
}

pub(crate) fn push_endpoint_error(
    shared: &Arc<Shared>,
    endpoint: Handle,
    status: ErrorCode,
    text: String,
) {
    let mut event = Event::simple(EventType::EndpointError);
    event.endpoint = endpoint;
    event.status = status;
    event.data = text.into_bytes();
    if shared.events.try_push(event).is_err() {
        shared
            .metrics
            .events_dropped
            .fetch_add(1, Ordering::Relaxed);
    }
}
