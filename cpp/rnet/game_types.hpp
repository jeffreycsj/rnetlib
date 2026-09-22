#ifndef RNET_GAME_TYPES_HPP
#define RNET_GAME_TYPES_HPP

#include "rnet/runtime.hpp"

#include <array>

namespace rnet {

enum class GameProfile : uint32_t {
  Realtime = RNET_GAME_PROFILE_REALTIME,
  ReliableRealtime = RNET_GAME_PROFILE_RELIABLE_REALTIME,
  Session = RNET_GAME_PROFILE_SESSION,
};

enum class GamePriority : uint32_t {
  Low = RNET_GAME_PRIORITY_LOW,
  Normal = RNET_GAME_PRIORITY_NORMAL,
  High = RNET_GAME_PRIORITY_HIGH,
  Critical = RNET_GAME_PRIORITY_CRITICAL,
};

struct GameSendOptions {
  bool has_sequence = false;
  uint32_t sequence = 0;
  bool has_tick = false;
  uint32_t tick = 0;
  uint64_t correlation_id = 0;
  GamePriority priority = GamePriority::Normal;
  uint64_t expiry_ms = 0;
};

struct GameProtocol {
  uint64_t id = 0;
  uint32_t version = 0;
  uint64_t build_id = 0;
  uint64_t capabilities = 0;
};

// Wire-v4 negotiation is opt-in; ordinary GameProtocol remains exact-version wire v3.
struct GameProtocolRange {
  uint64_t id = 0;
  uint32_t min_version = 0;
  uint32_t max_version = 0;
  uint64_t build_id = 0;
  uint64_t capabilities = 0;
};

struct GameServerOptions {
  Transport transport = Transport::Kcp;
  std::string host = "127.0.0.1";
  uint16_t port = 0;
  Keypair keypair;
  bool initial_encryption = true;
  GameProtocol protocol;
};

struct GameClientOptions {
  Transport transport = Transport::Kcp;
  std::string host = "127.0.0.1";
  uint16_t port = 0;
  std::vector<uint8_t> join_ticket;
  GameProtocol protocol;
};

struct GameRangeServerOptions {
  Transport transport = Transport::Kcp;
  std::string host = "127.0.0.1";
  uint16_t port = 0;
  Keypair keypair;
  bool initial_encryption = true;
  GameProtocolRange protocol;
};

struct GameRangeClientOptions {
  Transport transport = Transport::Kcp;
  std::string host = "127.0.0.1";
  uint16_t port = 0;
  std::vector<uint8_t> join_ticket;
  GameProtocolRange protocol;
};

struct GameEvent {
  uint32_t type = 0;
  rnet_endpoint_t endpoint = 0;
  rnet_session_t session = 0;
  rnet_session_t related_session = 0;
  int32_t status = RNET_OK;
  std::vector<uint8_t> data;
  std::vector<uint8_t> aux_data;
  std::array<uint8_t, 32> client_public_key{};
  uint64_t build_id = 0;
  uint64_t capabilities = 0;
  bool encrypted = false;
  uint64_t security_epoch = 0;
  uint32_t security_operation = 0;
  uint32_t quality_grade = 0;
  uint32_t quality_basis = 0;
  uint64_t last_rtt_us = 0;
  uint64_t jitter_us = 0;
  uint64_t quality_samples = 0;
  bool has_sequence = false;
  uint32_t sequence = 0;
  bool has_tick = false;
  uint32_t tick = 0;
  uint64_t correlation_id = 0;
};

struct GameQuality {
  bool available = false;
  uint32_t grade = 0;
  uint32_t basis = 0;
  bool has_udp_loss = false;
  bool has_kcp_retransmissions = false;
  uint32_t udp_recent_loss_per_mille = 0;
  uint32_t kcp_recent_retransmission_per_mille = 0;
  uint64_t last_rtt_us = 0;
  uint64_t smoothed_rtt_us = 0;
  uint64_t jitter_us = 0;
  uint64_t samples = 0;
  uint64_t udp_expected = 0;
  uint64_t udp_missing = 0;
  uint64_t kcp_segments_sent = 0;
  uint64_t kcp_retransmitted = 0;
};

// Runtime-local offset, not UTC or an authenticated time authority.
struct GameClockSync {
  bool available = false;
  int64_t server_minus_client_us = 0;
  uint64_t rtt_us = 0;
  uint64_t samples = 0;
};

// Cumulative game telemetry. `logger_available` distinguishes a disabled
// logger from one that has emitted no drops or callback errors.
using GameMetrics = rnet_game_metrics_t;

// Runtime-wide pre-transport snapshot queue; forwarded is not delivery.
struct GameRealtimeQueue {
  uint64_t queued_messages = 0;
  uint64_t queued_bytes = 0;
  uint64_t admission_rejected = 0;
  uint64_t replaced = 0;
  uint64_t closed_dropped = 0;
  uint64_t backpressure_dropped = 0;
  uint64_t send_failed = 0;
  uint64_t forwarded = 0;
};

struct GameScheduledQueue {
  uint64_t queued_messages = 0;
  uint64_t queued_bytes = 0;
  uint64_t admission_rejected = 0;
  uint64_t expired_dropped = 0;
  uint64_t closed_dropped = 0;
  uint64_t send_failed = 0;
  uint64_t forwarded = 0;
  uint64_t backpressure_requeued = 0;
  std::array<uint64_t, 4> admitted_by_priority{{0, 0, 0, 0}};
  std::array<uint64_t, 4> forwarded_by_priority{{0, 0, 0, 0}};
  uint64_t queue_delay_samples = 0;
  uint64_t queue_delay_p90_us = 0;
  uint64_t queue_delay_p95_us = 0;
  uint64_t queue_delay_p99_us = 0;
  uint64_t queue_delay_max_us = 0;
  uint64_t tick_queue_delay_samples = 0;
  uint64_t tick_queue_delay_p90_us = 0;
  uint64_t tick_queue_delay_p95_us = 0;
  uint64_t tick_queue_delay_p99_us = 0;
  uint64_t tick_queue_delay_max_us = 0;
};

struct GameRangeBuffer {
  uint64_t buffered_messages = 0;
  uint64_t buffered_bytes = 0;
  uint64_t peak_buffered_messages = 0;
  uint64_t peak_buffered_bytes = 0;
  uint64_t max_buffered_messages = 0;
  uint64_t max_buffered_bytes = 0;
  uint64_t session_admission_rejected = 0;
  uint64_t runtime_admission_rejected = 0;
};

struct GameTransportLatest {
  uint64_t pending_replaced = 0;
  uint64_t worker_pickups = 0;
  uint64_t admission_would_block = 0;
  uint64_t admission_invalid_handle = 0;
  uint64_t admission_invalid_state = 0;
  uint64_t admission_handshake_required = 0;
  uint64_t admission_not_supported = 0;
  uint64_t admission_message_too_large = 0;
  uint64_t admission_other_failures = 0;
};

}  // namespace rnet

#endif
