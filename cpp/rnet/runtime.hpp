#ifndef RNET_RUNTIME_HPP
#define RNET_RUNTIME_HPP

#include "rnet/keypair.hpp"

#include <utility>

namespace rnet {

enum class Transport : uint32_t {
  Tcp = RNET_TRANSPORT_TCP,
  Udp = RNET_TRANSPORT_UDP,
  Kcp = RNET_TRANSPORT_KCP
};

struct ServerOptions {
  Transport transport = Transport::Tcp;
  std::string host = "127.0.0.1";
  uint16_t port = 0;
  Keypair keypair;
  SecurityMode initial_security = SecurityMode::Encrypted;
};

struct ClientOptions {
  Transport transport = Transport::Tcp;
  std::string host = "127.0.0.1";
  uint16_t port = 0;
  std::string join_payload;
};

class Runtime {
 public:
  Runtime() {
    rnet_config_v5_t config{};
    check(rnet_config_v5_init(&config));
    check(rnet_runtime_create_v5(&config, nullptr, &handle_));
  }

  explicit Runtime(const rnet_config_v5_t &config) {
    check(rnet_runtime_create_v5(&config, nullptr, &handle_));
  }

  explicit Runtime(const rnet_config_v4_t &config) {
    check(rnet_runtime_create_v4(&config, nullptr, &handle_));
  }

  explicit Runtime(const rnet_config_v3_t &config) {
    check(rnet_runtime_create_v3(&config, nullptr, &handle_));
  }

  explicit Runtime(const rnet_config_t &config) {
    check(rnet_runtime_create(&config, &handle_));
  }

  /* Client identity and trust are runtime-scoped; connect() never selects a
   * plaintext/encrypted branch. */
  Runtime(const rnet_config_t &config, const Keypair &client_key,
          const std::array<uint8_t, 32> &expected_server_key) {
    rnet_client_security_t security = {};
    security.struct_size = sizeof(security);
    security.abi_version = RNET_ABI_VERSION;
    security.local_private_key.ptr = client_key.private_key();
    security.local_private_key.len = 32;
    security.expected_server_public_key.ptr = expected_server_key.data();
    security.expected_server_public_key.len = expected_server_key.size();
    check(rnet_runtime_create_v2(&config, &security, &handle_));
  }

  Runtime(const rnet_config_v3_t &config, const Keypair &client_key,
          const std::array<uint8_t, 32> &expected_server_key) {
    rnet_client_security_t security = {};
    security.struct_size = sizeof(security);
    security.abi_version = RNET_ABI_VERSION;
    security.local_private_key.ptr = client_key.private_key();
    security.local_private_key.len = 32;
    security.expected_server_public_key.ptr = expected_server_key.data();
    security.expected_server_public_key.len = expected_server_key.size();
    check(rnet_runtime_create_v3(&config, &security, &handle_));
  }

  Runtime(const rnet_config_v4_t &config, const Keypair &client_key,
          const std::array<uint8_t, 32> &expected_server_key) {
    rnet_client_security_t security = {};
    security.struct_size = sizeof(security);
    security.abi_version = RNET_ABI_VERSION;
    security.local_private_key.ptr = client_key.private_key();
    security.local_private_key.len = 32;
    security.expected_server_public_key.ptr = expected_server_key.data();
    security.expected_server_public_key.len = expected_server_key.size();
    check(rnet_runtime_create_v4(&config, &security, &handle_));
  }

  Runtime(const rnet_config_v5_t &config, const Keypair &client_key,
          const std::array<uint8_t, 32> &expected_server_key) {
    rnet_client_security_t security = {};
    security.struct_size = sizeof(security);
    security.abi_version = RNET_ABI_VERSION;
    security.local_private_key.ptr = client_key.private_key();
    security.local_private_key.len = 32;
    security.expected_server_public_key.ptr = expected_server_key.data();
    security.expected_server_public_key.len = expected_server_key.size();
    check(rnet_runtime_create_v5(&config, &security, &handle_));
  }

  Runtime(const Runtime &) = delete;
  Runtime &operator=(const Runtime &) = delete;

  Runtime(Runtime &&other) noexcept : handle_(other.handle_) {
    other.handle_ = 0;
  }

  Runtime &operator=(Runtime &&other) noexcept {
    if (this != &other) {
      reset();
      handle_ = other.handle_;
      other.handle_ = 0;
    }
    return *this;
  }

  ~Runtime() { reset(); }

  rnet_endpoint_t listen(const ServerOptions &options) {
    rnet_server_config_v2_t config = {};
    config.struct_size = sizeof(config);
    config.abi_version = RNET_ABI_VERSION;
    config.transport = static_cast<uint32_t>(options.transport);
    config.initial_security = static_cast<uint32_t>(options.initial_security);
    config.bind_host.ptr = reinterpret_cast<const uint8_t *>(options.host.data());
    config.bind_host.len = options.host.size();
    config.bind_port = options.port;
    config.local_private_key.ptr = options.keypair.private_key();
    config.local_private_key.len = 32;
    rnet_endpoint_t endpoint = 0;
    check(rnet_server_open_v2(handle_, &config, &endpoint));
    return endpoint;
  }

  rnet_endpoint_t connect(const ClientOptions &options) {
    rnet_client_config_v2_t config = {};
    config.struct_size = sizeof(config);
    config.abi_version = RNET_ABI_VERSION;
    config.transport = static_cast<uint32_t>(options.transport);
    config.remote_host.ptr = reinterpret_cast<const uint8_t *>(options.host.data());
    config.remote_host.len = options.host.size();
    config.remote_port = options.port;
    config.join_payload.ptr =
        reinterpret_cast<const uint8_t *>(options.join_payload.data());
    config.join_payload.len = options.join_payload.size();
    rnet_endpoint_t endpoint = 0;
    check(rnet_client_connect_v2(handle_, &config, &endpoint));
    return endpoint;
  }

  rnet_endpoint_t open_tcp_listener(const std::string &host, uint16_t port) {
    return open_endpoint(RNET_TRANSPORT_TCP, RNET_ENDPOINT_LISTENER, host,
                         port, std::string(), 0);
  }

  rnet_endpoint_t open_tcp_client(const std::string &host, uint16_t port) {
    return open_endpoint(RNET_TRANSPORT_TCP, RNET_ENDPOINT_CLIENT,
                         std::string(), 0, host, port);
  }

  rnet_endpoint_t open_udp(const std::string &bind_host, uint16_t bind_port,
                           const std::string &remote_host = std::string(),
                           uint16_t remote_port = 0) {
    return open_endpoint(RNET_TRANSPORT_UDP, RNET_ENDPOINT_DATAGRAM, bind_host,
                         bind_port, remote_host, remote_port);
  }

  rnet_endpoint_t open_secure_tcp_listener(const std::string &host,
                                           uint16_t port,
                                           const Keypair &keypair) {
    return open_secure_listener(RNET_TRANSPORT_TCP, host, port, keypair);
  }

  rnet_endpoint_t open_secure_udp_listener(const std::string &host,
                                           uint16_t port,
                                           const Keypair &keypair) {
    return open_secure_listener(RNET_TRANSPORT_UDP, host, port, keypair);
  }

  rnet_endpoint_t open_secure_kcp_listener(const std::string &host,
                                           uint16_t port,
                                           const Keypair &keypair) {
    return open_secure_listener(RNET_TRANSPORT_KCP, host, port, keypair);
  }

  rnet_endpoint_t open_secure_listener(uint32_t transport,
                                       const std::string &host, uint16_t port,
                                       const Keypair &keypair) {
    rnet_listener_config_t config = {};
    config.struct_size = sizeof(config);
    config.abi_version = RNET_ABI_VERSION;
    config.transport = transport;
    config.bind_host.ptr = reinterpret_cast<const uint8_t *>(host.data());
    config.bind_host.len = host.size();
    config.bind_port = port;
    config.local_private_key.ptr = keypair.private_key();
    config.local_private_key.len = 32;
    rnet_endpoint_t endpoint = 0;
    check(rnet_listener_open(handle_, &config, &endpoint));
    return endpoint;
  }

  rnet_endpoint_t join_secure_tcp(const std::string &host, uint16_t port,
                                  const Keypair &local_key,
                                  const uint8_t *server_public_key,
                                  const std::string &join_payload) {
    return join_secure(RNET_TRANSPORT_TCP, host, port, local_key,
                       server_public_key, join_payload);
  }

  rnet_endpoint_t join_secure_udp(const std::string &host, uint16_t port,
                                  const Keypair &local_key,
                                  const uint8_t *server_public_key,
                                  const std::string &join_payload) {
    return join_secure(RNET_TRANSPORT_UDP, host, port, local_key,
                       server_public_key, join_payload);
  }

  rnet_endpoint_t join_secure_kcp(const std::string &host, uint16_t port,
                                  const Keypair &local_key,
                                  const uint8_t *server_public_key,
                                  const std::string &join_payload) {
    return join_secure(RNET_TRANSPORT_KCP, host, port, local_key,
                       server_public_key, join_payload);
  }

  rnet_endpoint_t join_secure(uint32_t transport, const std::string &host,
                              uint16_t port, const Keypair &local_key,
                              const uint8_t *server_public_key,
                              const std::string &join_payload) {
    if (server_public_key == NULL) {
      throw std::invalid_argument("server public key is null");
    }
    rnet_join_config_t config = {};
    config.struct_size = sizeof(config);
    config.abi_version = RNET_ABI_VERSION;
    config.transport = transport;
    config.remote_host.ptr = reinterpret_cast<const uint8_t *>(host.data());
    config.remote_host.len = host.size();
    config.remote_port = port;
    config.local_private_key.ptr = local_key.private_key();
    config.local_private_key.len = 32;
    config.expected_server_public_key.ptr = server_public_key;
    config.expected_server_public_key.len = 32;
    config.join_payload.ptr =
        reinterpret_cast<const uint8_t *>(join_payload.data());
    config.join_payload.len = join_payload.size();
    rnet_endpoint_t endpoint = 0;
    check(rnet_client_join(handle_, &config, &endpoint));
    return endpoint;
  }

  void auth_decide(rnet_session_t session, bool accept) const {
    check(rnet_session_auth_decide(handle_, session, accept ? 1u : 0u));
  }

  void set_security(rnet_session_t session, SecurityMode mode) const {
    check(rnet_session_security_set(handle_, session,
                                    static_cast<uint32_t>(mode)));
  }

  void rekey(rnet_session_t session) const {
    check(rnet_session_rekey(handle_, session));
  }

  uint16_t local_port(rnet_endpoint_t endpoint) const {
    uint16_t port = 0;
    check(rnet_endpoint_local_port(handle_, endpoint, &port));
    return port;
  }

  void send(rnet_session_t session, uint32_t msg_type,
            const std::string &payload) const {
    send(session, msg_type, payload.data(), payload.size());
  }

  void send(rnet_session_t session, uint32_t msg_type, const void *payload,
            size_t payload_size) const {
    rnet_slice_t bytes;
    bytes.ptr = static_cast<const uint8_t *>(payload);
    bytes.len = payload_size;
    check(rnet_session_send(handle_, session, msg_type, bytes));
  }

  void send_with_options(rnet_session_t session, uint32_t msg_type,
                         const std::string &payload,
                         const SendOptions &options) const {
    rnet_send_options_t raw = {};
    raw.struct_size = sizeof(raw);
    raw.abi_version = RNET_ABI_VERSION;
    raw.correlation_id = options.correlation_id;
    rnet_slice_t bytes = {
        reinterpret_cast<const uint8_t *>(payload.data()), payload.size()};
    check(rnet_session_send_ex(handle_, session, msg_type, bytes, &raw));
  }

  void send_legacy(rnet_session_t session, uint32_t msg_type,
                   uint32_t stream_id, uint64_t request_id,
                   const std::string &payload) const {
    rnet_slice_t bytes = {
        reinterpret_cast<const uint8_t *>(payload.data()), payload.size()};
    check(rnet_send(handle_, session, msg_type, stream_id, bytes, request_id));
  }

  std::vector<Event> poll(size_t capacity, uint32_t timeout_ms) const {
    std::vector<rnet_event_t> raw(capacity);
    size_t count = 0;
    check(rnet_poll_events_ex(handle_, raw.empty() ? NULL : raw.data(),
                              raw.size(), timeout_ms, &count));
    std::vector<Event> result;
    result.reserve(count);
    for (size_t index = 0; index < count; ++index) {
      const auto &source = raw[index];
      Event event;
      event.type = source.event_type;
      event.endpoint = source.endpoint;
      event.session = source.session;
      event.msg_type = source.msg_type;
      event.stream_id = source.stream_id;
      event.request_id = source.request_id;
      event.status = source.status;
      if (source.data_len != 0) {
        event.data.assign(source.data, source.data + source.data_len);
      }
      if (source.buffer_token != 0) {
        check(rnet_buffer_release(handle_, source.buffer_token));
      }
      result.push_back(std::move(event));
    }
    return result;
  }

  void close_session(rnet_session_t session,
                     int32_t reason = RNET_E_CANCELLED) const {
    check(rnet_session_close(handle_, session, reason));
  }

  void close_endpoint(rnet_endpoint_t endpoint) const {
    check(rnet_endpoint_close(handle_, endpoint));
  }

  Metrics metrics() const {
    rnet_metrics_v3_t raw = {};
    check(rnet_metrics_snapshot_v3(handle_, &raw));
    Metrics result;
    result.frames_received = raw.frames_received;
    result.frames_sent = raw.frames_sent;
    result.bytes_received = raw.bytes_received;
    result.bytes_sent = raw.bytes_sent;
    result.events_dropped = raw.events_dropped;
    result.send_would_block = raw.send_would_block;
    result.protocol_errors = raw.protocol_errors;
    result.lifecycle_events_rejected = raw.lifecycle_events_rejected;
    result.admission_rejected = raw.admission_rejected;
    result.queued_send_bytes = raw.queued_send_bytes;
    result.peak_queued_send_bytes = raw.peak_queued_send_bytes;
    result.queued_event_bytes = raw.queued_event_bytes;
    result.current_endpoints = raw.current_endpoints;
    result.current_sessions = raw.current_sessions;
    result.established_sessions = raw.established_sessions;
    result.pending_handshakes = raw.pending_handshakes;
    result.peak_pending_handshakes = raw.peak_pending_handshakes;
    std::copy(raw.admission_rejected_by_reason,
              raw.admission_rejected_by_reason + 8,
              result.admission_rejected_by_reason.begin());
    std::copy(raw.session_closed_by_reason,
              raw.session_closed_by_reason + 19,
              result.session_closed_by_reason.begin());
    result.logs_dropped = raw.logs_dropped;
    result.logger_panics = raw.logger_panics;
    return result;
  }

  std::vector<LatencyMetric> latency_metrics(bool drain_window = false) const {
    size_t count = 0;
    check(rnet_latency_snapshot_v2(handle_, NULL, 0, &count,
                                   drain_window ? 1U : 0U));
    std::vector<rnet_latency_metric_v2_t> raw(count);
    check(rnet_latency_snapshot_v2(handle_, raw.empty() ? NULL : raw.data(),
                                   raw.size(), &count,
                                   drain_window ? 1U : 0U));
    std::vector<LatencyMetric> result;
    result.reserve(count);
    for (size_t index = 0; index < count; ++index) {
      LatencyMetric metric;
      metric.kind = raw[index].kind;
      metric.sample_count = raw[index].sample_count;
      metric.p50_us = raw[index].p50_us;
      metric.p90_us = raw[index].p90_us;
      metric.p95_us = raw[index].p95_us;
      metric.p99_us = raw[index].p99_us;
      metric.p999_us = raw[index].p999_us;
      metric.max_us = raw[index].max_us;
      result.push_back(metric);
    }
    return result;
  }

  void set_metrics_log_interval(uint64_t interval_ms) const {
    check(rnet_metrics_log_interval_set(handle_, interval_ms));
  }

  void stop(uint32_t drain_timeout_ms = 0) {
    if (handle_ != 0) {
      check(rnet_runtime_stop(handle_, drain_timeout_ms));
    }
  }

  void close() {
    if (handle_ == 0) {
      return;
    }
    stop();
    check(rnet_runtime_destroy(handle_));
    handle_ = 0;
  }

 private:
  rnet_endpoint_t open_endpoint(uint32_t transport, uint32_t mode,
                                const std::string &bind_host,
                                uint16_t bind_port,
                                const std::string &remote_host,
                                uint16_t remote_port) {
    rnet_endpoint_config_t config{};
    config.struct_size = sizeof(config);
    config.abi_version = RNET_ABI_VERSION;
    config.transport = transport;
    config.mode = mode;
    config.bind_host.ptr =
        reinterpret_cast<const uint8_t *>(bind_host.data());
    config.bind_host.len = bind_host.size();
    config.bind_port = bind_port;
    config.remote_host.ptr =
        reinterpret_cast<const uint8_t *>(remote_host.data());
    config.remote_host.len = remote_host.size();
    config.remote_port = remote_port;
    rnet_endpoint_t endpoint = 0;
    check(rnet_endpoint_open(handle_, &config, &endpoint));
    return endpoint;
  }

  void reset() noexcept {
    if (handle_ == 0) {
      return;
    }
    (void)rnet_runtime_stop(handle_, 0);
    (void)rnet_runtime_destroy(handle_);
    handle_ = 0;
  }

  rnet_runtime_t handle_ = 0;
};

}  // namespace rnet

#endif
