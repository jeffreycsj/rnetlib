#ifndef RNET_GAME_HPP
#define RNET_GAME_HPP

#include "rnet/runtime.hpp"

namespace rnet {

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

class GameRuntime {
 public:
  GameRuntime() {
    rnet_game_config_t config{};
    check(rnet_game_config_init(&config));
    check(rnet_game_runtime_create(&config, nullptr, &handle_));
  }

  explicit GameRuntime(const rnet_game_config_t &config) {
    check(rnet_game_runtime_create(&config, nullptr, &handle_));
  }

  GameRuntime(const rnet_game_config_t &config,
              const rnet_logger_v2_t &logger) {
    check(rnet_game_runtime_create_logged(&config, nullptr, &logger, &handle_));
  }

  GameRuntime(const rnet_game_config_t &config, const Keypair &client_key,
              const std::array<uint8_t, 32> &expected_server_key) {
    rnet_client_security_t security{};
    security.struct_size = sizeof(security);
    security.abi_version = RNET_ABI_VERSION;
    security.local_private_key = {client_key.private_key(), 32};
    security.expected_server_public_key = {expected_server_key.data(), 32};
    check(rnet_game_runtime_create(&config, &security, &handle_));
  }

  GameRuntime(const rnet_game_config_t &config, const Keypair &client_key,
              const std::array<uint8_t, 32> &expected_server_key,
              const rnet_logger_v2_t &logger) {
    rnet_client_security_t security{};
    security.struct_size = sizeof(security);
    security.abi_version = RNET_ABI_VERSION;
    security.local_private_key = {client_key.private_key(), 32};
    security.expected_server_public_key = {expected_server_key.data(), 32};
    check(rnet_game_runtime_create_logged(&config, &security, &logger, &handle_));
  }

  GameRuntime(const Keypair &client_key,
              const std::array<uint8_t, 32> &expected_server_key,
              bool allow_plaintext_business_data = false) {
    rnet_game_config_t config{};
    check(rnet_game_config_init(&config));
    config.allow_plaintext_business_data = allow_plaintext_business_data ? 1U : 0U;
    rnet_client_security_t security{};
    security.struct_size = sizeof(security);
    security.abi_version = RNET_ABI_VERSION;
    security.local_private_key = {client_key.private_key(), 32};
    security.expected_server_public_key = {expected_server_key.data(), 32};
    check(rnet_game_runtime_create(&config, &security, &handle_));
  }

  GameRuntime(const GameRuntime &) = delete;
  GameRuntime &operator=(const GameRuntime &) = delete;

  GameRuntime(GameRuntime &&other) noexcept : handle_(other.handle_) {
    other.handle_ = 0;
  }

  GameRuntime &operator=(GameRuntime &&other) noexcept {
    if (this != &other) {
      reset();
      handle_ = other.handle_;
      other.handle_ = 0;
    }
    return *this;
  }

  ~GameRuntime() { reset(); }

  rnet_endpoint_t listen(const GameServerOptions &options) const {
    rnet_game_server_config_t config{};
    config.struct_size = sizeof(config);
    config.abi_version = RNET_ABI_VERSION;
    config.transport = static_cast<uint32_t>(options.transport);
    config.initial_encryption = options.initial_encryption ? 1U : 0U;
    config.bind_host = bytes(options.host);
    config.bind_port = options.port;
    config.local_private_key = {options.keypair.private_key(), 32};
    config.protocol_id = options.protocol.id;
    config.protocol_version = options.protocol.version;
    rnet_endpoint_t endpoint = 0;
    check(rnet_game_server_listen(handle_, &config, &endpoint));
    return endpoint;
  }

  rnet_endpoint_t connect(const GameClientOptions &options) const {
    const rnet_game_client_config_t config = client_config(options);
    rnet_endpoint_t endpoint = 0;
    check(rnet_game_client_connect(handle_, &config, &endpoint));
    return endpoint;
  }

  rnet_endpoint_t listen_range(const GameRangeServerOptions &options) const {
    rnet_game_range_server_config_t config{};
    config.struct_size = sizeof(config);
    config.abi_version = RNET_ABI_VERSION;
    config.transport = static_cast<uint32_t>(options.transport);
    config.initial_encryption = options.initial_encryption ? 1U : 0U;
    config.bind_host = bytes(options.host);
    config.bind_port = options.port;
    config.local_private_key = {options.keypair.private_key(), 32};
    config.protocol_id = options.protocol.id;
    config.min_version = options.protocol.min_version;
    config.max_version = options.protocol.max_version;
    rnet_endpoint_t endpoint = 0;
    check(rnet_game_server_listen_range(handle_, &config, &endpoint));
    return endpoint;
  }

  rnet_endpoint_t connect_range(const GameRangeClientOptions &options) const {
    const rnet_game_range_client_config_t config = range_client_config(options);
    rnet_endpoint_t endpoint = 0;
    check(rnet_game_client_connect_range(handle_, &config, &endpoint));
    return endpoint;
  }

  rnet_endpoint_t connect_range_resume(const GameRangeClientOptions &options,
                                       rnet_session_t old_session,
                                       const std::vector<uint8_t> &ticket) const {
    const rnet_game_range_client_config_t config = range_client_config(options);
    rnet_endpoint_t endpoint = 0;
    check(rnet_game_client_resume_connect_range(handle_, &config, old_session,
                                                bytes(ticket), &endpoint));
    return endpoint;
  }

  uint32_t selected_protocol_version(rnet_session_t session) const {
    uint32_t version = 0;
    check(rnet_game_selected_protocol_version(handle_, session, &version));
    return version;
  }

  uint64_t transport_latest_replacements() const {
    uint64_t replaced = 0;
    check(rnet_game_transport_latest_replacements(handle_, &replaced));
    return replaced;
  }

  GameTransportLatest transport_latest_snapshot() const {
    rnet_game_transport_latest_t raw{};
    check(rnet_game_transport_latest_snapshot(handle_, &raw));
    GameTransportLatest value;
    value.pending_replaced = raw.pending_replaced;
    value.worker_pickups = raw.worker_pickups;
    value.admission_would_block = raw.admission_would_block;
    value.admission_invalid_handle = raw.admission_invalid_handle;
    value.admission_invalid_state = raw.admission_invalid_state;
    value.admission_handshake_required = raw.admission_handshake_required;
    value.admission_not_supported = raw.admission_not_supported;
    value.admission_message_too_large = raw.admission_message_too_large;
    value.admission_other_failures = raw.admission_other_failures;
    return value;
  }

  rnet_endpoint_t connect_resume(const GameClientOptions &options,
                                 rnet_session_t old_session,
                                 const std::vector<uint8_t> &ticket) const {
    const rnet_game_client_config_t config = client_config(options);
    rnet_endpoint_t endpoint = 0;
    check(rnet_game_client_resume_connect(handle_, &config, old_session,
                                          bytes(ticket), &endpoint));
    return endpoint;
  }

  void issue_resume_ticket(rnet_session_t session,
                           const std::vector<uint8_t> &identity) const {
    check(rnet_game_issue_resume_ticket(handle_, session, bytes(identity)));
  }

  uint16_t local_port(rnet_endpoint_t endpoint) const {
    uint16_t port = 0;
    check(rnet_game_endpoint_local_port(handle_, endpoint, &port));
    return port;
  }

  void auth_decide(rnet_session_t session, bool accept) const {
    check(rnet_game_auth_decide(handle_, session, accept ? 1U : 0U));
  }

  void send(rnet_session_t session, const std::vector<uint8_t> &payload) const {
    check(rnet_game_send(handle_, session, bytes(payload)));
  }

  void send(rnet_session_t session, const std::string &payload) const {
    check(rnet_game_send(handle_, session, bytes(payload)));
  }

  void send_latest(rnet_session_t session, uint64_t key,
                   const std::vector<uint8_t> &payload) const {
    check(rnet_game_send_latest(handle_, session, key, bytes(payload)));
  }

  void send_latest(rnet_session_t session, uint64_t key,
                   const std::string &payload) const {
    check(rnet_game_send_latest(handle_, session, key, bytes(payload)));
  }

  void close_session(rnet_session_t session) const {
    check(rnet_game_session_close(handle_, session));
  }

  void close_endpoint(rnet_endpoint_t endpoint) const {
    check(rnet_game_endpoint_close(handle_, endpoint));
  }

  void rekey(rnet_session_t session) const {
    check(rnet_game_rekey(handle_, session));
  }

  void set_encryption(rnet_session_t session, bool enabled) const {
    check(rnet_game_security_set(handle_, session, enabled ? 1U : 0U));
  }

  GameQuality network_quality(rnet_session_t session) const {
    rnet_game_quality_t raw{};
    check(rnet_game_network_quality(handle_, session, &raw));
    GameQuality value;
    value.available = raw.available != 0;
    value.grade = raw.grade;
    value.basis = raw.basis;
    value.has_udp_loss = raw.has_udp_loss != 0;
    value.has_kcp_retransmissions = raw.has_kcp_retransmissions != 0;
    value.udp_recent_loss_per_mille = raw.udp_recent_loss_per_mille;
    value.kcp_recent_retransmission_per_mille =
        raw.kcp_recent_retransmission_per_mille;
    value.last_rtt_us = raw.last_rtt_us;
    value.smoothed_rtt_us = raw.smoothed_rtt_us;
    value.jitter_us = raw.jitter_us;
    value.samples = raw.samples;
    value.udp_expected = raw.udp_expected;
    value.udp_missing = raw.udp_missing;
    value.kcp_segments_sent = raw.kcp_segments_sent;
    value.kcp_retransmitted = raw.kcp_retransmitted;
    return value;
  }

  uint64_t clock_micros() const {
    uint64_t value = 0;
    check(rnet_game_clock_micros(handle_, &value));
    return value;
  }

  GameClockSync clock_sync_snapshot(rnet_session_t session) const {
    rnet_game_clock_sync_t raw{};
    check(rnet_game_clock_sync_snapshot(handle_, session, &raw));
    GameClockSync value;
    value.available = raw.available != 0;
    value.server_minus_client_us = raw.server_minus_client_us;
    value.rtt_us = raw.rtt_us;
    value.samples = raw.samples;
    return value;
  }

  GameMetrics metrics_snapshot() const {
    GameMetrics value{};
    check(rnet_game_metrics_snapshot(handle_, &value));
    return value;
  }

  GameRealtimeQueue realtime_queue_snapshot() const {
    rnet_game_realtime_queue_t raw{};
    check(rnet_game_realtime_queue_snapshot(handle_, &raw));
    GameRealtimeQueue value;
    value.queued_messages = raw.queued_messages;
    value.queued_bytes = raw.queued_bytes;
    value.admission_rejected = raw.admission_rejected;
    value.replaced = raw.replaced;
    value.closed_dropped = raw.closed_dropped;
    value.backpressure_dropped = raw.backpressure_dropped;
    value.send_failed = raw.send_failed;
    value.forwarded = raw.forwarded;
    return value;
  }

  GameRangeBuffer range_buffer_snapshot() const {
    rnet_game_range_buffer_t raw{};
    check(rnet_game_range_buffer_snapshot(handle_, &raw));
    GameRangeBuffer value;
    value.buffered_messages = raw.buffered_messages;
    value.buffered_bytes = raw.buffered_bytes;
    value.peak_buffered_messages = raw.peak_buffered_messages;
    value.peak_buffered_bytes = raw.peak_buffered_bytes;
    value.max_buffered_messages = raw.max_buffered_messages;
    value.max_buffered_bytes = raw.max_buffered_bytes;
    value.session_admission_rejected = raw.session_admission_rejected;
    value.runtime_admission_rejected = raw.runtime_admission_rejected;
    return value;
  }

  std::string prometheus_snapshot() const {
    rnet_game_buffer_t raw{};
    check(rnet_game_prometheus_snapshot(handle_, &raw));
    struct Release {
      rnet_runtime_t runtime;
      uint64_t token;
      ~Release() {
        if (token != 0)
          (void)rnet_game_buffer_release(runtime, token);
      }
    } release{handle_, raw.token};
    return raw.len == 0 ? std::string()
                        : std::string(reinterpret_cast<const char *>(raw.data),
                                      raw.len);
  }

  std::vector<GameEvent> poll(size_t capacity, uint32_t timeout_ms) const {
    std::vector<rnet_game_event_t> raw(capacity);
    size_t count = 0;
    check(rnet_game_poll_events(handle_, raw.empty() ? nullptr : raw.data(),
                                raw.size(), timeout_ms, &count));
    // Own all returned tokens before any C++ allocation can throw.
    struct ReleaseBatch {
      rnet_runtime_t runtime;
      const std::vector<rnet_game_event_t> &events;
      size_t count;
      ~ReleaseBatch() {
        for (size_t i = 0; i < count; ++i) {
          if (events[i].buffer_token != 0)
            (void)rnet_game_buffer_release(runtime, events[i].buffer_token);
          if (events[i].aux_buffer_token != 0)
            (void)rnet_game_buffer_release(runtime, events[i].aux_buffer_token);
        }
      }
    } release{handle_, raw, count};
    std::vector<GameEvent> result;
    result.reserve(count);
    for (size_t i = 0; i < count; ++i) {
      const rnet_game_event_t &source = raw[i];
      GameEvent event;
      event.type = source.event_type;
      event.endpoint = source.endpoint;
      event.session = source.session;
      event.related_session = source.related_session;
      event.status = source.status;
      if (source.data_len != 0)
        event.data.assign(source.data, source.data + source.data_len);
      if (source.aux_data_len != 0)
        event.aux_data.assign(source.aux_data, source.aux_data + source.aux_data_len);
      std::copy(source.client_public_key, source.client_public_key + 32,
                event.client_public_key.begin());
      event.build_id = source.build_id;
      event.capabilities = source.capabilities;
      event.encrypted = source.encrypted != 0;
      event.security_epoch = source.security_epoch;
      event.security_operation = source.security_operation;
      event.quality_grade = source.quality_grade;
      event.quality_basis = source.quality_basis;
      event.last_rtt_us = source.last_rtt_us;
      event.jitter_us = source.jitter_us;
      event.quality_samples = source.quality_samples;
      event.has_sequence = source.has_sequence != 0;
      event.sequence = source.sequence;
      event.has_tick = source.has_tick != 0;
      event.tick = source.tick;
      result.push_back(std::move(event));
    }
    return result;
  }

  void stop(uint32_t drain_timeout_ms = 0) {
    if (handle_ != 0)
      check(rnet_game_runtime_stop(handle_, drain_timeout_ms));
  }

  void close() {
    if (handle_ == 0)
      return;
    stop();
    check(rnet_game_runtime_destroy(handle_));
    handle_ = 0;
  }

 private:
  static rnet_game_range_client_config_t range_client_config(
      const GameRangeClientOptions &options) {
    rnet_game_range_client_config_t config{};
    config.struct_size = sizeof(config);
    config.abi_version = RNET_ABI_VERSION;
    config.transport = static_cast<uint32_t>(options.transport);
    config.remote_host = bytes(options.host);
    config.remote_port = options.port;
    config.join_ticket = bytes(options.join_ticket);
    config.protocol_id = options.protocol.id;
    config.min_version = options.protocol.min_version;
    config.max_version = options.protocol.max_version;
    config.build_id = options.protocol.build_id;
    config.capabilities = options.protocol.capabilities;
    return config;
  }

  static rnet_game_client_config_t client_config(
      const GameClientOptions &options) {
    rnet_game_client_config_t config{};
    config.struct_size = sizeof(config);
    config.abi_version = RNET_ABI_VERSION;
    config.transport = static_cast<uint32_t>(options.transport);
    config.remote_host = bytes(options.host);
    config.remote_port = options.port;
    config.join_ticket = bytes(options.join_ticket);
    config.protocol_id = options.protocol.id;
    config.protocol_version = options.protocol.version;
    config.build_id = options.protocol.build_id;
    config.capabilities = options.protocol.capabilities;
    return config;
  }

  static rnet_slice_t bytes(const std::string &value) {
    return {reinterpret_cast<const uint8_t *>(value.data()), value.size()};
  }

  static rnet_slice_t bytes(const std::vector<uint8_t> &value) {
    return {value.empty() ? nullptr : value.data(), value.size()};
  }

  void reset() noexcept {
    if (handle_ == 0)
      return;
    (void)rnet_game_runtime_stop(handle_, 0);
    (void)rnet_game_runtime_destroy(handle_);
    handle_ = 0;
  }

  rnet_runtime_t handle_ = 0;
};

}  // namespace rnet

#endif
