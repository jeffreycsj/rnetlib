#ifndef RNET_GAME_TYPES_HPP
#define RNET_GAME_TYPES_HPP

#include "rnet/runtime.hpp"

#include <array>

namespace rnet {

/// Local scenario preset; independent from business protocol/SDK compatibility.
enum class GameProfile : uint32_t {
  Realtime = RNET_GAME_PROFILE_REALTIME, ///< UDP with encryption enabled; application tolerates losses.
  ReliableRealtime = RNET_GAME_PROFILE_RELIABLE_REALTIME, ///< KCP with encryption enabled; reliable real-time traffic.
  Session = RNET_GAME_PROFILE_SESSION, ///< TCP with encryption enabled; ordered session traffic.
};

/// Weighted local game scheduling class, not an on-wire dispatch promise.
enum class GamePriority : uint32_t {
  Low = RNET_GAME_PRIORITY_LOW, ///< Lowest local scheduler priority; no effect on remote dispatch.
  Normal = RNET_GAME_PRIORITY_NORMAL, ///< Default priority used by ordinary game send.
  High = RNET_GAME_PRIORITY_HIGH, ///< Higher local scheduler share, not a guaranteed deadline.
  Critical = RNET_GAME_PRIORITY_CRITICAL, ///< Highest local scheduler share; lower queues still receive service.
};

/// Optional game envelope metadata and local scheduling; no deduplication/ordering is implied.
struct GameSendOptions {
  bool has_sequence = false; ///< False omits sequence, true includes it; no ordering or deduplication is enabled.
  uint32_t sequence = 0; ///< Caller-defined sequence copied unchanged when has_sequence is true; zero is valid.
  bool has_tick = false; ///< False omits tick, true includes it.
  uint32_t tick = 0; ///< Caller-defined simulation tick copied unchanged when has_tick is true.
  uint64_t correlation_id = 0; ///< Optional request/trace identifier copied to the peer; zero absent.
  GamePriority priority = GamePriority::Normal; ///< Local queue priority, Normal by default; does not change network delivery guarantees.
  uint64_t expiry_ms = 0; ///< Relative local staging lifetime in milliseconds; zero means no expiry, not immediate expiry.
};

/// Application-version range, independent from SDK/wire compatibility; fill ID and bounds before use.
struct GameProtocol {
  uint64_t id = 0; ///< Application protocol identifier; must be nonzero and match the server.
  uint32_t min_version = 0; ///< Positive inclusive minimum business version; equal bounds request an exact version.
  uint32_t max_version = 0; ///< Inclusive maximum business version, at least min_version; highest overlap is selected.
  uint64_t build_id = 0; ///< Optional client build metadata for authorization; not used by listen.
  uint64_t capabilities = 0; ///< Optional client capability bits for authorization; not negotiated or used by listen.
};

/// Listener policy; construction generates a key but does not open a socket.
struct GameServerOptions {
  Transport transport = Transport::Kcp; ///< Fixed listener transport; default KCP.
  std::string host = "127.0.0.1"; ///< Numeric bind IP without brackets, URL scheme or port.
  uint16_t port = 0; ///< Local bind port; zero requests an OS-assigned port.
  Keypair keypair; ///< Auto-generated server identity; restore a stable key for pinned production trust.
  bool initial_encryption = true; ///< Initial business encryption; false requires runtime plaintext opt-in.
  GameProtocol protocol; ///< Required application ID/version range; default zero fields must be filled.
};

/// Game join options; construction does not connect and default protocol/port are incomplete.
struct GameClientOptions {
  Transport transport = Transport::Kcp; ///< Fixed transport matching the listener; default KCP.
  std::string host = "127.0.0.1"; ///< DNS name or numeric IP without URL scheme/path or embedded port.
  uint16_t port = 0; ///< Remote service port; default zero is invalid and must be filled.
  std::vector<uint8_t> join_ticket; ///< Opaque login/authorization bytes, copied by connect; never log credentials.
  GameProtocol protocol; ///< Required ID/version range plus optional client build/capability metadata.
};

/// Owned game event; use fields by type. Caller owns erasure of credential/identity vector copies.
struct GameEvent {
  uint32_t type = 0; ///< RNET_GAME_* event discriminant; only the documented fields for that event are meaningful.
  rnet_endpoint_t endpoint = 0; ///< Local endpoint; zero for runtime-wide events.
  rnet_session_t session = 0; ///< Current/pending local session, including NEW handle on SESSION_RESUMED; zero when not applicable.
  rnet_session_t related_session = 0; ///< Old local handle on RESUME_REQUEST/SESSION_RESUMED; zero otherwise.
  int32_t status = RNET_OK; ///< RNET_E_* on error/rejection/close events; other kinds (including PROTOCOL_VIOLATION) use RNET_OK.
  std::vector<uint8_t> data; ///< AUTH_REQUEST login ticket; RESUME_REQUEST identity; RESUME_TICKET credential; MESSAGE business bytes. Owned copy; caller erases secrets.
  std::vector<uint8_t> aux_data; ///< RESUME_REQUEST's fresh login ticket, separate from the identity in data; otherwise empty. Owned copy.
  std::array<uint8_t, 32> client_public_key{}; ///< Authenticated 32-byte client key on AUTH_REQUEST/RESUME_REQUEST.
  uint64_t build_id = 0; ///< Client build metadata on AUTH_REQUEST/RESUME_REQUEST.
  uint64_t capabilities = 0; ///< Client capability metadata on AUTH_REQUEST/RESUME_REQUEST; not authorization.
  bool encrypted = false; ///< SECURITY_CHANGED resulting business mode: false plaintext, true encrypted; controls stay protected.
  uint64_t security_epoch = 0; ///< SECURITY_CHANGED session-local key/mode generation, not a wall-clock time.
  uint32_t security_operation = 0; ///< SECURITY_CHANGED RNET_SECURITY_OPERATION_* value.
  uint32_t quality_grade = 0; ///< QUALITY_CHANGED grade: 0 unknown, 1 excellent, 2 good, 3 fair, 4 poor.
  uint32_t quality_basis = 0; ///< QUALITY_CHANGED basis: 1 latency, 2 latency+UDP gaps, 3 latency+KCP retransmission.
  uint64_t last_rtt_us = 0; ///< QUALITY_CHANGED latest authenticated heartbeat RTT in microseconds.
  uint64_t jitter_us = 0; ///< QUALITY_CHANGED smoothed absolute RTT variation in microseconds.
  uint64_t quality_samples = 0; ///< QUALITY_CHANGED accepted heartbeat sample count.
  bool has_sequence = false; ///< MESSAGE: false when absent, true when sender supplied optional sequence metadata.
  uint32_t sequence = 0; ///< MESSAGE sequence if has_sequence is true; not internal UDP loss numbering.
  bool has_tick = false; ///< MESSAGE: false when absent, true when sender supplied a simulation tick.
  uint32_t tick = 0; ///< MESSAGE simulation tick if has_tick is true; not a trusted clock.
  uint64_t correlation_id = 0; ///< MESSAGE optional request/trace metadata; zero means absent.
};

/// Authenticated heartbeat latency plus transport-specific evidence, not a universal loss estimator.
struct GameQuality {
  bool available = false; ///< False before the first heartbeat sample; true permits interpreting the remaining measurements.
  uint32_t grade = 0; ///< 0 unknown, 1 excellent, 2 good, 3 fair, 4 poor; sample minima may keep grade unknown.
  uint32_t basis = 0; ///< 1 latency only, 2 latency+UDP gaps, 3 latency+KCP retransmissions.
  bool has_udp_loss = false; ///< False when unavailable; true when UDP loss fields present; not the same as measured zero loss.
  bool has_kcp_retransmissions = false; ///< False when unavailable; true when KCP fields present; TCP has neither transport signal.
  uint32_t udp_recent_loss_per_mille = 0; ///< Recent UDP sequence-gap estimate per 1,000 positions (10 means 1%); check availability.
  uint32_t kcp_recent_retransmission_per_mille = 0; ///< Recent KCP retransmission ratio per 1,000 PUSH transmissions; not raw network loss.
  uint64_t last_rtt_us = 0; ///< Latest authenticated heartbeat RTT in microseconds, including queueing effects.
  uint64_t smoothed_rtt_us = 0; ///< Smoothed heartbeat RTT in microseconds; not a percentile.
  uint64_t jitter_us = 0; ///< Smoothed absolute RTT variation in microseconds.
  uint64_t samples = 0; ///< Accepted heartbeat samples on this session.
  uint64_t udp_expected = 0; ///< Cumulative expected UDP data positions, excluding implausible forward jumps.
  uint64_t udp_missing = 0; ///< Expected minus received UDP positions; late arrivals may reduce this value.
  uint64_t kcp_segments_sent = 0; ///< Cumulative KCP PUSH transmissions, including retransmissions, excluding ACKs.
  uint64_t kcp_retransmitted = 0; ///< Cumulative repeated KCP PUSH transmissions.
};

/// Approximate client monotonic-clock sample; not UTC or a trusted anti-cheat clock.
struct GameClockSync {
  bool available = false; ///< False on server sessions/before first sample; true when a client sample is available.
  int64_t server_minus_client_us = 0; ///< Signed microseconds added to client runtime clock to approximate server runtime clock.
  uint64_t rtt_us = 0; ///< Round-trip estimate in microseconds after subtracting reported server turnaround.
  uint64_t samples = 0; ///< Accepted clock samples on this client session, not heartbeat samples.
};

/// Cumulative game telemetry; logger_available distinguishes no logger from zero drops.
/// Fields carry their units/availability on rnet_game_metrics_t; this is the same documented type.
using GameMetrics = rnet_game_metrics_t;

/// LatestOnly game-staging gauges and cumulative outcomes; forwarding is not delivery.
struct GameRealtimeQueue {
  uint64_t queued_messages = 0; ///< Current distinct pending (session,key) entries in game staging.
  uint64_t queued_bytes = 0; ///< Current payload plus game-envelope reservations, not allocator RSS.
  uint64_t admission_rejected = 0; ///< Cumulative staging budget/key-cap rejections, including failed requeues.
  uint64_t replaced = 0; ///< Cumulative old pending snapshots superseded by newer same-key values.
  uint64_t closed_dropped = 0; ///< Cumulative staged values discarded during session cleanup/shutdown.
  uint64_t backpressure_dropped = 0; ///< Cumulative values unable to reenter staging after transport backpressure.
  uint64_t send_failed = 0; ///< Cumulative values discarded after non-backpressure forwarding failures.
  uint64_t forwarded = 0; ///< Cumulative snapshots accepted by transport, not delivered to the peer.
};

/// Local priority/expiry queue telemetry; tick_* samples cover only messages carrying a tick.
struct GameScheduledQueue {
  uint64_t queued_messages = 0; ///< Current admitted messages awaiting forwarding or drop.
  uint64_t queued_bytes = 0; ///< Current payload plus worst-case game-envelope reservations.
  uint64_t admission_rejected = 0; ///< Cumulative staging byte/message budget rejections.
  uint64_t expired_dropped = 0; ///< Cumulative messages expiring locally before forwarding.
  uint64_t closed_dropped = 0; ///< Cumulative messages discarded on session cleanup/shutdown.
  uint64_t send_failed = 0; ///< Cumulative non-backpressure transport forwarding failures.
  uint64_t forwarded = 0; ///< Cumulative transport admissions, not deliveries.
  uint64_t backpressure_requeued = 0; ///< Cumulative retries returned to staging; one message can be counted repeatedly.
  std::array<uint64_t, 4> admitted_by_priority{{0, 0, 0, 0}}; ///< Cumulative admissions indexed low, normal, high, critical.
  std::array<uint64_t, 4> forwarded_by_priority{{0, 0, 0, 0}}; ///< Cumulative transport forwards in low, normal, high, critical order.
  uint64_t queue_delay_samples = 0; ///< Cumulative successful-forward delay samples; admission to transport, not delivery.
  uint64_t queue_delay_p90_us = 0; ///< Estimated cumulative local queue-delay P90 in microseconds.
  uint64_t queue_delay_p95_us = 0; ///< Estimated cumulative local queue-delay P95 in microseconds.
  uint64_t queue_delay_p99_us = 0; ///< Estimated cumulative local queue-delay P99 in microseconds.
  uint64_t queue_delay_max_us = 0; ///< Largest sampled local queue delay in microseconds.
  uint64_t tick_queue_delay_samples = 0; ///< Same delay sample count, restricted to messages carrying a tick.
  uint64_t tick_queue_delay_p90_us = 0; ///< Tick-bearing messages' estimated cumulative queue-delay P90, microseconds.
  uint64_t tick_queue_delay_p95_us = 0; ///< Tick-bearing messages' estimated cumulative queue-delay P95, microseconds.
  uint64_t tick_queue_delay_p99_us = 0; ///< Tick-bearing messages' estimated cumulative queue-delay P99, microseconds.
  uint64_t tick_queue_delay_max_us = 0; ///< Largest sampled tick-bearing message queue delay, microseconds.
};

/// Runtime-wide early business-data buffering before negotiation is ready.
struct GameRangeBuffer {
  uint64_t buffered_messages = 0; ///< Current early messages retained, including ready-but-not-yet-polled messages.
  uint64_t buffered_bytes = 0; ///< Current early payload bytes; excludes framing/allocator overhead.
  uint64_t peak_buffered_messages = 0; ///< Highest retained early-message count observed.
  uint64_t peak_buffered_bytes = 0; ///< Highest retained early payload bytes observed.
  uint64_t max_buffered_messages = 0; ///< Runtime-wide cap inherited from configured event queue capacity.
  uint64_t max_buffered_bytes = 0; ///< Runtime-wide cap inherited from configured event-byte budget.
  uint64_t session_admission_rejected = 0; ///< Cumulative per-session early-data budget rejections.
  uint64_t runtime_admission_rejected = 0; ///< Cumulative runtime-wide early-data budget rejections.
};

/// TCP/KCP replaceable pending-slot counters; worker pickup ends replaceability.
struct GameTransportLatest {
  uint64_t pending_replaced = 0; ///< Cumulative TCP/KCP pending values replaced before I/O worker pickup.
  uint64_t worker_pickups = 0; ///< Cumulative slots consumed by I/O workers; no recall is possible after pickup.
  uint64_t admission_would_block = 0; ///< Cumulative bounded transport-capacity rejections, including retries.
  uint64_t admission_invalid_handle = 0; ///< Cumulative admissions referencing retired/unknown sessions.
  uint64_t admission_invalid_state = 0; ///< Cumulative admissions when runtime/session was not accepting work.
  uint64_t admission_handshake_required = 0; ///< Cumulative admissions before transport readiness.
  uint64_t admission_not_supported = 0; ///< Cumulative unsupported-path rejections; UDP uses ordinary forwarding.
  uint64_t admission_message_too_large = 0; ///< Cumulative size-limit rejections.
  uint64_t admission_other_failures = 0; ///< Cumulative admission failures outside the explicit categories.
};

}  // namespace rnet

#endif
