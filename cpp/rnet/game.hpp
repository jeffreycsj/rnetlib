#ifndef RNET_GAME_HPP
#define RNET_GAME_HPP

#include "rnet/game_types.hpp"

namespace rnet {

/// Move-only game runtime; client identity/trust is copied at construction.
/// Methods throw Error (with native status/detail); allocations may throw bad_alloc.
/// Handles belong to this object. Use one poll consumer; synchronize move/close/
/// destruction against every call. Logger callbacks must not stop/destroy it.
/// Both peers must poll regularly for negotiation, heartbeats, clocks and queues.
/// Explicit close reports failures; destructor cleanup is best-effort and noexcept.
class GameRuntime {
 public:
  /// Creates a server-capable runtime with secure resource defaults; does not listen.
  GameRuntime() {
    check_abi();
    rnet_game_config_t config{};
    check(rnet_game_config_init(&config));
    check(rnet_game_runtime_create(&config, nullptr, &handle_));
  }

  /// Server-capable runtime; config must come from rnet_game_config_init, then customization.
  explicit GameRuntime(const rnet_game_config_t &config) {
    check_abi();
    check(rnet_game_runtime_create(&config, nullptr, &handle_));
  }

  /// Overrides config.logger; callback/context must survive destruction and must not throw.
  GameRuntime(const rnet_game_config_t &config,
              const rnet_logger_t &logger) {
    check_abi();
    auto configured = config;
    configured.logger = &logger;
    check(rnet_game_runtime_create(&configured, nullptr, &handle_));
  }

  /// Copies client identity and exact server pin; supports listen as well as outbound connect.
  GameRuntime(const rnet_game_config_t &config, const Keypair &client_key,
              const std::array<uint8_t, 32> &expected_server_key) {
    check_abi();
    rnet_client_security_t security{};
    security.struct_size = sizeof(security);
    security.abi_version = RNET_ABI_VERSION;
    security.local_private_key = {client_key.private_key(), 32};
    security.expected_server_public_key = {expected_server_key.data(), 32};
    check(rnet_game_runtime_create(&config, &security, &handle_));
  }

  /// Client identity/pin plus logger override; callback/context must outlive this runtime.
  GameRuntime(const rnet_game_config_t &config, const Keypair &client_key,
              const std::array<uint8_t, 32> &expected_server_key,
              const rnet_logger_t &logger) {
    check_abi();
    rnet_client_security_t security{};
    security.struct_size = sizeof(security);
    security.abi_version = RNET_ABI_VERSION;
    security.local_private_key = {client_key.private_key(), 32};
    security.expected_server_public_key = {expected_server_key.data(), 32};
    auto configured = config;
    configured.logger = &logger;
    check(rnet_game_runtime_create(&configured, &security, &handle_));
  }

  /// Default config plus copied client identity/pin. Plaintext flag permits LOCAL server
  /// policy only; outbound clients follow the server's negotiated mode automatically.
  GameRuntime(const Keypair &client_key,
              const std::array<uint8_t, 32> &expected_server_key,
              bool allow_plaintext_business_data = false) {
    check_abi();
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

  /// Unique ownership: runtime resources cannot be copied.
  GameRuntime(const GameRuntime &) = delete;
  /// Unique ownership: use move to transfer the handle.
  GameRuntime &operator=(const GameRuntime &) = delete;

  /// Transfers ownership; other becomes empty. No concurrent calls may use either object.
  GameRuntime(GameRuntime &&other) noexcept : handle_(other.handle_) {
    other.handle_ = 0;
  }

  /// Best-effort closes this runtime before transfer; call close first to observe cleanup errors.
  GameRuntime &operator=(GameRuntime &&other) noexcept {
    if (this != &other) {
      reset();
      handle_ = other.handle_;
      other.handle_ = 0;
    }
    return *this;
  }

  /// Best-effort stop/destroy without exceptions; explicit close is the checked shutdown path.
  ~GameRuntime() { reset(); }

  /// Binds fixed transport/numeric IP and returns a listener; fill protocol ID/version bounds.
  /// Copying options/key does not transfer their ownership. Poll to authorize incoming peers.
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
    config.min_version = options.protocol.min_version;
    config.max_version = options.protocol.max_version;
    rnet_endpoint_t endpoint = 0;
    check(rnet_game_server_listen(handle_, &config, &endpoint));
    return endpoint;
  }

  /// Starts join and returns endpoint, not session. DNS may wait; poll until SESSION_READY.
  rnet_endpoint_t connect(const GameClientOptions &options) const {
    const rnet_game_client_config_t config = client_config(options);
    rnet_endpoint_t endpoint = 0;
    check(rnet_game_client_connect(handle_, &config, &endpoint));
    return endpoint;
  }

  /// Uses nonzero old_session and copied ticket for a new endpoint; server reauthorization required.
  /// Ticket is single-use on its issuing server runtime only, not valid across restart.
  /// SESSION_RESUMED maps old to new handle; a live old session survives failed negotiation.
  rnet_endpoint_t connect_resume(const GameClientOptions &options,
                                 rnet_session_t old_session,
                                 const std::vector<uint8_t> &ticket) const {
    const rnet_game_client_config_t config = client_config(options);
    rnet_endpoint_t endpoint = 0;
    check(rnet_game_client_resume_connect(handle_, &config, old_session,
                                         bytes(ticket), &endpoint));
    return endpoint;
  }

  /// Returns selected business version or throws if unavailable; selection is not readiness.
  uint32_t selected_protocol_version(rnet_session_t session) const {
    uint32_t version = 0;
    check(rnet_game_selected_protocol_version(handle_, session, &version));
    return version;
  }

  /// Cumulative TCP/KCP replacements after game staging but before worker pickup.
  uint64_t transport_latest_replacements() const {
    uint64_t replaced = 0;
    check(rnet_game_transport_latest_replacements(handle_, &replaced));
    return replaced;
  }

  /// Copies TCP/KCP pending-slot admission/replacement/pickup counters; not delivery counts.
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

  /// Server queues ticket for ready session and copied 1..64-byte identity; client gets RESUME_TICKET.
  void issue_resume_ticket(rnet_session_t session,
                           const std::vector<uint8_t> &identity) const {
    check(rnet_game_issue_resume_ticket(handle_, session, bytes(identity)));
  }

  /// Bound port of a live endpoint, including OS choice after listen(port=0); stale handles throw.
  uint16_t local_port(rnet_endpoint_t endpoint) const {
    uint16_t port = 0;
    check(rnet_game_endpoint_local_port(handle_, endpoint, &port));
    return port;
  }

  /// Answers AUTH_REQUEST/RESUME_REQUEST; accept=true is not readiness. Wait for READY/RESUMED.
  void auth_decide(rnet_session_t session, bool accept) const {
    check(rnet_game_auth_decide(handle_, session, accept ? 1U : 0U));
  }

  /// Copies opaque payload to Normal queue of a ready session; empty is valid. Poll forwards it.
  /// Success is admission only; oversize, not-ready and backpressure throw Error.
  void send(rnet_session_t session, const std::vector<uint8_t> &payload) const {
    check(rnet_game_send(handle_, session, bytes(payload)));
  }

  /// Binary-safe string overload of send; embedded NUL bytes are preserved, no terminator is sent.
  void send(rnet_session_t session, const std::string &payload) const {
    check(rnet_game_send(handle_, session, bytes(payload)));
  }

  /// Queues a copied ready-session payload with optional metadata, priority and local staging expiry.
  void send(rnet_session_t session, const std::vector<uint8_t> &payload,
            const GameSendOptions &options) const {
    send_with_options(session, bytes(payload), options);
  }

  /// Binary-safe send with scheduling options; expiry cannot recall transport-admitted bytes.
  void send(rnet_session_t session, const std::string &payload,
            const GameSendOptions &options) const {
    send_with_options(session, bytes(payload), options);
  }

  /// Copies a best-effort snapshot for ready session/key (zero key valid); poll forwards it.
  /// Replaces game staging and TCP/KCP pending slots until worker pickup; never socket/KCP cache.
  void send_latest(rnet_session_t session, uint64_t key,
                   const std::vector<uint8_t> &payload) const {
    check(rnet_game_send_latest(handle_, session, key, bytes(payload)));
  }

  /// Binary-safe snapshot overload; same-session/key replacement, not a remote delivery guarantee.
  void send_latest(rnet_session_t session, uint64_t key,
                   const std::string &payload) const {
    check(rnet_game_send_latest(handle_, session, key, bytes(payload)));
  }

  /// Closes peer and revokes its resume/staged state; listener remains open.
  void close_session(rnet_session_t session) const {
    check(rnet_game_session_close(handle_, session));
  }

  /// Closes endpoint and all its sessions, discarding staged/resume state; runtime stays alive.
  void close_endpoint(rnet_endpoint_t endpoint) const {
    check(rnet_game_endpoint_close(handle_, endpoint));
  }

  /// Server requests rotation; SECURITY_CHANGED/REKEY reports local commit, not this return.
  void rekey(rnet_session_t session) const {
    check(rnet_game_rekey(handle_, session));
  }

  /// Server requests business mode; enabled=false requires runtime opt-in and removes integrity.
  /// Controls stay protected, client follows automatically; SECURITY_CHANGED reports local commit.
  /// Requesting the already active mode can succeed without another event.
  void set_encryption(rnet_session_t session, bool enabled) const {
    check(rnet_game_security_set(handle_, session, enabled ? 1U : 0U));
  }

  /// Copies quality for session; available=false before sampling, distinct from measured zero loss.
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

  /// Runtime-local monotonic microseconds, not Unix/UTC time.
  uint64_t clock_micros() const {
    uint64_t value = 0;
    check(rnet_game_clock_micros(handle_, &value));
    return value;
  }

  /// Approximate client offset sample; available=false on server/before sampling. Not anti-cheat time.
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

  /// Copies cumulative counters/current gauges; fields are not one globally atomic observation.
  GameMetrics metrics_snapshot() const {
    GameMetrics value{};
    check(rnet_game_metrics_snapshot(handle_, &value));
    return value;
  }

  /// Poll-driven cumulative log interval in milliseconds; default 30000, zero disables.
  void set_metrics_log_interval(uint64_t interval_ms) const {
    check(rnet_game_metrics_log_interval_set(handle_, interval_ms));
  }

  /// Current LatestOnly staging gauges and cumulative outcomes; forwarded is not delivered.
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

  /// Priority queue gauges/outcomes and cumulative delays (all messages versus tick-bearing); no reset.
  GameScheduledQueue scheduled_queue_snapshot() const {
    rnet_game_scheduled_queue_t raw{};
    check(rnet_game_scheduled_queue_snapshot(handle_, &raw));
    GameScheduledQueue value;
    value.queued_messages = raw.queued_messages;
    value.queued_bytes = raw.queued_bytes;
    value.admission_rejected = raw.admission_rejected;
    value.expired_dropped = raw.expired_dropped;
    value.closed_dropped = raw.closed_dropped;
    value.send_failed = raw.send_failed;
    value.forwarded = raw.forwarded;
    value.backpressure_requeued = raw.backpressure_requeued;
    for (std::size_t index = 0; index < 4; ++index) {
      value.admitted_by_priority[index] = raw.admitted_by_priority[index];
      value.forwarded_by_priority[index] = raw.forwarded_by_priority[index];
    }
    value.queue_delay_samples = raw.queue_delay_samples;
    value.queue_delay_p90_us = raw.queue_delay_p90_us;
    value.queue_delay_p95_us = raw.queue_delay_p95_us;
    value.queue_delay_p99_us = raw.queue_delay_p99_us;
    value.queue_delay_max_us = raw.queue_delay_max_us;
    value.tick_queue_delay_samples = raw.tick_queue_delay_samples;
    value.tick_queue_delay_p90_us = raw.tick_queue_delay_p90_us;
    value.tick_queue_delay_p95_us = raw.tick_queue_delay_p95_us;
    value.tick_queue_delay_p99_us = raw.tick_queue_delay_p99_us;
    value.tick_queue_delay_max_us = raw.tick_queue_delay_max_us;
    return value;
  }

  /// Negotiation early-data gauges and admission failures, aggregated over this runtime.
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

  /// Returns an owned low-cardinality Prometheus text copy; native buffer is released even on exception.
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

  /// Returns up to capacity owned events, releasing both native tokens even when copying throws.
  /// Zero capacity still drives maintenance; timeout_ms limits idle wait, not lock/work time.
  /// Staged count/byte limits pause transport draining, not timers. Use positive capacity
  /// regularly; zero-only polling can block queued ACKs and cause heartbeat timeouts.
  /// Poll regularly on both peers. Allocation failure consumes the exported events, without replay.
  std::vector<GameEvent> poll(size_t capacity, uint32_t timeout_ms) const {
    std::vector<rnet_game_event_t> raw(capacity);
    size_t count = 0;
    const int32_t status = rnet_game_poll_events(
        handle_, raw.empty() ? nullptr : raw.data(), raw.size(), timeout_ms, &count);
    // An error can follow a partial export. Own its tokens before check() throws.
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
    check(status);
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
      event.correlation_id = raw[i].correlation_id;
      result.push_back(std::move(event));
    }
    return result;
  }

  /// Stops I/O with send-drain budget in milliseconds (zero=no drain); keeps handle for poll/close.
  /// On Error(WOULD_BLOCK), poll pending events then retry; never call from logger callback.
  void stop(uint32_t drain_timeout_ms = 0) {
    if (handle_ != 0)
      check(rnet_game_runtime_stop(handle_, drain_timeout_ms));
  }

  /// Checked stop(0)/destroy; on failure retain handle and resolve/retry. Empty object is a no-op.
  void close() {
    if (handle_ == 0)
      return;
    stop();
    check(rnet_game_runtime_destroy(handle_));
    handle_ = 0;
  }

private:
  void send_with_options(rnet_session_t session, rnet_slice_t payload,
                         const GameSendOptions &options) const {
    rnet_game_send_options_t raw{};
    raw.struct_size = sizeof(raw);
    raw.abi_version = RNET_ABI_VERSION;
    raw.has_sequence = options.has_sequence ? 1U : 0U;
    raw.sequence = options.sequence;
    raw.has_tick = options.has_tick ? 1U : 0U;
    raw.tick = options.tick;
    raw.correlation_id = options.correlation_id;
    raw.priority = static_cast<uint32_t>(options.priority);
    raw.expiry_ms = options.expiry_ms;
    check(rnet_game_send_ex(handle_, session, payload, &raw));
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
    config.min_version = options.protocol.min_version;
    config.max_version = options.protocol.max_version;
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
