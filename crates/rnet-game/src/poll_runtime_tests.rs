use super::*;
use crate::envelope::{encode_application, encode_control, ControlKind};
use crate::range_state::{Phase, RangeRuntimeState, RangeSession, MAX_BUFFERED_MESSAGES};
use crate::{GameProtocol, GameRuntimeConfig};
use std::collections::VecDeque;

fn negotiating_runtime(max_messages: usize, max_bytes: usize) -> GameRuntime {
    let runtime = GameRuntime::new(GameRuntimeConfig::default()).unwrap();
    let mut range = RangeRuntimeState::with_buffer_limits(max_messages, max_bytes);
    let now = Instant::now();
    range.sessions.insert(
        2,
        RangeSession {
            endpoint: 1,
            protocol: GameProtocol::new(1, 1, 2),
            nonce: [7; 16],
            selected: Some(2),
            server_handle: 3,
            old_session: None,
            server_resume: false,
            phase: Phase::ClientAwaitReady,
            deadline: now + Duration::from_secs(3),
            next_retry: now,
            buffered: VecDeque::new(),
            buffered_bytes: 0,
        },
    );
    *runtime.range.lock().unwrap() = range;
    runtime
}

fn message(payload: &[u8], verified: bool) -> Event {
    let mut event = Event::simple(EventType::Message);
    event.endpoint = 1;
    event.session = 2;
    event.integrity_verified = verified;
    event.data = encode_application(payload, None, None, 1024)
        .unwrap()
        .to_vec();
    event
}

#[test]
fn valid_early_data_budget_failure_is_visible_for_both_security_modes() {
    // Empty messages must consume message slots; byte and per-session limits are independent.
    for verified in [false, true] {
        for (messages, bytes, payload, accepted, per_session) in [
            (1, 1024, &b""[..], 1, false),
            (10, 4, &b"1234"[..], 1, false),
            (64, 1024, &b""[..], MAX_BUFFERED_MESSAGES, true),
        ] {
            let runtime = negotiating_runtime(messages, bytes);
            let mut output = Vec::new();
            for _ in 0..accepted {
                runtime.convert_events_into(vec![message(payload, verified)], &mut output, 1);
            }
            assert!(
                output.is_empty(),
                "early data must remain private until READY"
            );
            runtime.convert_events_into(vec![message(payload, verified)], &mut output, 1);
            assert!(
                matches!(
                    output.as_slice(),
                    [GameEvent::ProtocolViolation { session: 2, .. }]
                ),
                "local budget exhaustion must not be silently discarded: verified={verified}"
            );
            let metrics = runtime.range_buffer_snapshot();
            assert_eq!(metrics.session_admission_rejected, u64::from(per_session));
            assert_eq!(metrics.runtime_admission_rejected, u64::from(!per_session));
            assert_eq!(
                runtime
                    .protocol_metrics_snapshot()
                    .plaintext_invalid_envelopes_dropped,
                0
            );
        }
    }
}

#[test]
fn malformed_business_data_does_not_consume_early_data_budget() {
    for verified in [false, true] {
        for shape in 0..4 {
            let runtime = negotiating_runtime(1, 4);
            let mut event = message(b"okay", verified);
            match shape {
                0 => event.data.clear(),
                1 => {
                    event.data = encode_control(ControlKind::Heartbeat, b"", 1024)
                        .unwrap()
                        .to_vec()
                }
                2 => event.msg_type = 1,
                _ => {
                    event.data =
                        crate::envelope::encode_udp_application(b"okay", None, None, 0, 1024)
                            .unwrap()
                            .to_vec()
                }
            }
            let mut output = Vec::new();
            runtime.convert_events_into(vec![event], &mut output, 1);
            assert_eq!(output.len(), usize::from(verified));
            assert_eq!(
                runtime
                    .protocol_metrics_snapshot()
                    .plaintext_invalid_envelopes_dropped,
                u64::from(!verified)
            );
            let metrics = runtime.range_buffer_snapshot();
            assert_eq!(metrics.buffered_messages, 0);
            assert_eq!(metrics.buffered_bytes, 0);
            assert_eq!(metrics.runtime_admission_rejected, 0);
            assert_eq!(metrics.session_admission_rejected, 0);
        }
    }
}
