//! Optional, bounded game-session diagnostics. Records never contain credential or payload bytes.

use crate::event::GameEvent;
use crate::runtime::GameRuntime;
use rnet_core::{ErrorCode, Handle};
use rnet_observe::{BoundedLogger, LatencySnapshot, LogLevel, LogRecord};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GameLoggerSnapshot {
    pub dropped: u64,
    pub sink_panics: u64,
}

pub(crate) struct GameDiagnostics {
    runtime_id: Handle,
    logger: Option<BoundedLogger>,
}

impl GameDiagnostics {
    pub(crate) fn new() -> Self {
        Self {
            runtime_id: NEXT_RUNTIME_ID.fetch_add(1, Ordering::Relaxed),
            logger: None,
        }
    }

    fn record(&self, event: &GameEvent, transport: u32, protocol: Option<(u64, u32)>) {
        let Some(logger) = self.logger.as_ref() else {
            return;
        };
        let Some((name, level, endpoint, session, status, message)) = describe(event, protocol)
        else {
            return;
        };
        let _ = logger.log(LogRecord {
            timestamp_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            level,
            event_name: name.into(),
            target: "rnet.game".into(),
            message,
            runtime: self.runtime_id,
            endpoint,
            session,
            transport,
            error_code: status,
            correlation_id: 0,
        });
    }

    fn record_send_failure(
        &self,
        endpoint: Handle,
        session: Handle,
        transport: u32,
        status: ErrorCode,
        correlation_id: u64,
    ) {
        let Some(logger) = self.logger.as_ref() else {
            return;
        };
        let _ = logger.log(LogRecord {
            timestamp_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            level: LogLevel::Warn,
            event_name: "game_scheduled_send_failed".into(),
            target: "rnet.game".into(),
            message: String::new(),
            runtime: self.runtime_id,
            endpoint,
            session,
            transport,
            error_code: status,
            correlation_id,
        });
    }
}

impl GameRuntime {
    /// Installs an optional asynchronous logger before the runtime is used. Log callbacks run
    /// outside network and game-poll threads; normal sends and receives remain unchanged.
    pub fn with_logger(mut self, logger: BoundedLogger) -> Self {
        self.diagnostics.logger = Some(logger);
        self
    }

    /// Logger overload and callback-panic counters, or `None` when logging was not configured.
    pub fn game_logger_snapshot(&self) -> Option<GameLoggerSnapshot> {
        self.diagnostics
            .logger
            .as_ref()
            .map(|logger| GameLoggerSnapshot {
                dropped: logger.dropped(),
                sink_panics: logger.sink_panics(),
            })
    }

    /// Asynchronous sink callback time. This separate query preserves the original public
    /// `GameLoggerSnapshot` shape for Rust callers constructing it directly.
    pub fn game_logger_callback_latency(&self) -> Option<LatencySnapshot> {
        self.diagnostics
            .logger
            .as_ref()
            .map(BoundedLogger::callback_latency)
    }

    pub(crate) fn log_game_event(&self, event: &GameEvent) {
        if self.diagnostics.logger.is_none()
            || matches!(event, GameEvent::Message(_) | GameEvent::Writable { .. })
        {
            return;
        }
        let endpoint = event_endpoint(event);
        let transport = self
            .endpoint_transports
            .lock()
            .expect("game endpoint table poisoned")
            .get(&endpoint)
            .map_or(0, |transport| *transport as u32);
        let protocol = self
            .server_protocols
            .lock()
            .expect("game protocol table poisoned")
            .get(&endpoint)
            .map(|protocol| (protocol.protocol_id, protocol.version));
        self.diagnostics.record(event, transport, protocol);
    }

    pub(crate) fn log_scheduled_send_failure(
        &self,
        session: Handle,
        status: ErrorCode,
        correlation_id: u64,
    ) {
        let endpoint = self
            .session_endpoints
            .lock()
            .expect("game session table poisoned")
            .get(&session)
            .copied()
            .unwrap_or_default();
        let transport = self
            .endpoint_transports
            .lock()
            .expect("game endpoint table poisoned")
            .get(&endpoint)
            .map_or(0, |transport| *transport as u32);
        self.diagnostics
            .record_send_failure(endpoint, session, transport, status, correlation_id);
    }
}

fn event_endpoint(event: &GameEvent) -> Handle {
    match event {
        GameEvent::RuntimeStarted | GameEvent::RuntimeStopped => 0,
        GameEvent::EndpointOpened { endpoint }
        | GameEvent::EndpointError { endpoint, .. }
        | GameEvent::AuthRequest { endpoint, .. }
        | GameEvent::ResumeRequest { endpoint, .. }
        | GameEvent::ProtocolRejected { endpoint, .. }
        | GameEvent::SessionReady { endpoint, .. }
        | GameEvent::SessionResumed { endpoint, .. }
        | GameEvent::ResumeTicket { endpoint, .. }
        | GameEvent::SessionClosed { endpoint, .. }
        | GameEvent::Writable { endpoint, .. }
        | GameEvent::JoinFailed { endpoint, .. }
        | GameEvent::SecurityChanged { endpoint, .. }
        | GameEvent::QualityChanged { endpoint, .. }
        | GameEvent::ProtocolViolation { endpoint, .. } => *endpoint,
        GameEvent::Message(message) => message.endpoint,
    }
}

type Description = (&'static str, LogLevel, Handle, Handle, ErrorCode, String);

fn describe(event: &GameEvent, protocol: Option<(u64, u32)>) -> Option<Description> {
    let endpoint = event_endpoint(event);
    let (name, level, session, status, message) = match event {
        GameEvent::RuntimeStarted => (
            "game_runtime_started",
            LogLevel::Info,
            0,
            ErrorCode::Ok,
            String::new(),
        ),
        GameEvent::RuntimeStopped => (
            "game_runtime_stopped",
            LogLevel::Info,
            0,
            ErrorCode::Ok,
            String::new(),
        ),
        GameEvent::EndpointOpened { .. } => (
            "game_endpoint_opened",
            LogLevel::Info,
            0,
            ErrorCode::Ok,
            String::new(),
        ),
        GameEvent::EndpointError { status, .. } => (
            "game_endpoint_error",
            LogLevel::Error,
            0,
            *status,
            String::new(),
        ),
        GameEvent::AuthRequest {
            session, build_id, ..
        } => {
            let (id, version) = protocol.unwrap_or_default();
            (
                "game_auth_requested",
                LogLevel::Info,
                *session,
                ErrorCode::Ok,
                format!("protocol_id={id} protocol_version={version} build_id={build_id}"),
            )
        }
        GameEvent::ResumeRequest {
            session,
            old_session,
            build_id,
            ..
        } => {
            let (id, version) = protocol.unwrap_or_default();
            ("game_resume_requested", LogLevel::Info, *session, ErrorCode::Ok,
                format!("old_session={old_session} protocol_id={id} protocol_version={version} build_id={build_id}"))
        }
        GameEvent::ProtocolRejected {
            session, reason, ..
        } => (
            "game_protocol_rejected",
            LogLevel::Warn,
            *session,
            *reason,
            String::new(),
        ),
        GameEvent::SessionReady { session, .. } => (
            "game_session_ready",
            LogLevel::Info,
            *session,
            ErrorCode::Ok,
            String::new(),
        ),
        GameEvent::SessionResumed {
            new_session,
            old_session,
            ..
        } => (
            "game_session_resumed",
            LogLevel::Info,
            *new_session,
            ErrorCode::Ok,
            format!("old_session={old_session}"),
        ),
        GameEvent::ResumeTicket { session, .. } => (
            "game_resume_ticket_received",
            LogLevel::Info,
            *session,
            ErrorCode::Ok,
            String::new(),
        ),
        GameEvent::SessionClosed {
            session, reason, ..
        } => (
            "game_session_closed",
            if *reason == ErrorCode::Ok {
                LogLevel::Info
            } else {
                LogLevel::Warn
            },
            *session,
            *reason,
            String::new(),
        ),
        GameEvent::JoinFailed {
            session, reason, ..
        } => (
            "game_join_failed",
            LogLevel::Warn,
            *session,
            *reason,
            String::new(),
        ),
        GameEvent::SecurityChanged {
            session,
            encrypted,
            epoch,
            operation,
            ..
        } => (
            "game_security_changed",
            LogLevel::Info,
            *session,
            ErrorCode::Ok,
            format!("encrypted={encrypted} epoch={epoch} operation={operation:?}"),
        ),
        GameEvent::QualityChanged {
            session, quality, ..
        } => (
            "game_quality_changed",
            LogLevel::Info,
            *session,
            ErrorCode::Ok,
            format!(
                "grade={:?} basis={:?} rtt_us={}",
                quality.grade,
                quality.basis,
                quality.last_rtt.as_micros()
            ),
        ),
        GameEvent::ProtocolViolation { session, .. } => (
            "game_protocol_violation",
            LogLevel::Warn,
            *session,
            ErrorCode::ProtocolError,
            String::new(),
        ),
        GameEvent::Message(_) | GameEvent::Writable { .. } => return None,
    };
    Some((name, level, endpoint, session, status, message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::GameMessage;
    use bytes::Bytes;

    #[test]
    fn optional_logger_preserves_runtime_send_sync_contract() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GameRuntime>();
    }

    #[test]
    fn successful_close_is_informational_and_failure_keeps_its_reason() {
        let clean = describe(
            &GameEvent::SessionClosed {
                endpoint: 1,
                session: 2,
                reason: ErrorCode::Ok,
            },
            None,
        )
        .unwrap();
        assert_eq!(clean.1, LogLevel::Info);
        assert_eq!(clean.4, ErrorCode::Ok);
        let failed = describe(
            &GameEvent::SessionClosed {
                endpoint: 1,
                session: 2,
                reason: ErrorCode::Timeout,
            },
            None,
        )
        .unwrap();
        assert_eq!(failed.1, LogLevel::Warn);
        assert_eq!(failed.4, ErrorCode::Timeout);
    }

    #[test]
    fn resume_diagnostics_exclude_identity_and_ticket_bytes() {
        let event = GameEvent::ResumeRequest {
            endpoint: 4,
            session: 8,
            old_session: 6,
            identity: b"private-player-id".to_vec().into(),
            client_public_key: [0; 32],
            join_ticket: b"private-login-proof".to_vec().into(),
            build_id: 9,
            capabilities: 0,
        };
        let record = describe(&event, Some((42, 3))).unwrap();
        assert_eq!(record.0, "game_resume_requested");
        assert_eq!(record.3, 8);
        assert!(record.5.contains("old_session=6"));
        assert!(!record.5.contains("private-player-id"));
        assert!(!record.5.contains("private-login-proof"));
        assert!(!record.5.contains(&format!("{:?}", b"private-login-proof")));
        assert!(describe(
            &GameEvent::Message(GameMessage {
                endpoint: 4,
                session: 8,
                sequence: None,
                tick: None,
                correlation_id: 0,
                payload: Bytes::from_static(b"private-business-data"),
            }),
            None
        )
        .is_none());
    }
}
