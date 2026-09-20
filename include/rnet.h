#ifndef RNET_H
#define RNET_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define RNET_ABI_VERSION 1u

typedef uint64_t rnet_runtime_t;
typedef uint64_t rnet_endpoint_t;
typedef uint64_t rnet_session_t;

enum {
  RNET_OK = 0,
  RNET_E_INVALID_ARGUMENT = -1,
  RNET_E_INVALID_HANDLE = -2,
  RNET_E_INVALID_STATE = -3,
  RNET_E_WOULD_BLOCK = -4,
  RNET_E_TIMEOUT = -5,
  RNET_E_NOT_SUPPORTED = -6,
  RNET_E_IO_ERROR = -7,
  RNET_E_PROTOCOL_ERROR = -8,
  RNET_E_MESSAGE_TOO_LARGE = -9,
  RNET_E_INTERNAL_PANIC = -10,
  RNET_E_HANDSHAKE_REQUIRED = -11,
  RNET_E_HANDSHAKE_FAILED = -12,
  RNET_E_AUTH_REJECTED = -13,
  RNET_E_PEER_KEY_MISMATCH = -14,
  RNET_E_REPLAY_DETECTED = -15,
  RNET_E_CRYPTO_ERROR = -16,
  RNET_E_RATE_LIMITED = -17,
  RNET_E_CANCELLED = -18
};

enum {
  RNET_TRANSPORT_TCP = 1,
  RNET_TRANSPORT_UDP = 2,
  RNET_TRANSPORT_KCP = 3
};

enum {
  RNET_SECURITY_PLAINTEXT = 1,
  RNET_SECURITY_ENCRYPTED = 2
};

enum {
  RNET_SECURITY_OPERATION_MODE_SWITCH = 1,
  RNET_SECURITY_OPERATION_REKEY = 2
};

enum {
  RNET_ENDPOINT_LISTENER = 1,
  RNET_ENDPOINT_CLIENT = 2,
  RNET_ENDPOINT_DATAGRAM = 3
};

enum {
  RNET_EVENT_RUNTIME_STARTED = 1,
  RNET_EVENT_ENDPOINT_OPENED = 2,
  RNET_EVENT_ENDPOINT_ERROR = 3,
  RNET_EVENT_SESSION_OPENED = 4,
  RNET_EVENT_SESSION_CLOSED = 5,
  RNET_EVENT_MESSAGE = 6,
  RNET_EVENT_WRITABLE = 7,
  RNET_EVENT_RUNTIME_STOPPED = 8,
  RNET_EVENT_AUTH_REQUEST = 9,
  RNET_EVENT_JOIN_FAILED = 10,
  RNET_EVENT_SECURITY_CHANGED = 11
};

enum {
  RNET_LOG_TRACE = 0,
  RNET_LOG_DEBUG = 1,
  RNET_LOG_INFO = 2,
  RNET_LOG_WARN = 3,
  RNET_LOG_ERROR = 4
};

enum {
  RNET_LATENCY_CONNECT = 1,
  RNET_LATENCY_CRYPTO_HANDSHAKE = 2,
  RNET_LATENCY_AUTH_WAIT = 3,
  RNET_LATENCY_SEND_QUEUE = 4,
  RNET_LATENCY_EVENT_QUEUE = 5,
  RNET_LATENCY_KCP_RTT = 6,
  RNET_LATENCY_KCP_UPDATE_DELAY = 7,
  RNET_LATENCY_LOGGER_CALLBACK = 8
};

typedef struct rnet_slice {
  const uint8_t *ptr;
  size_t len;
} rnet_slice_t;

typedef void (*rnet_log_fn)(void *user_data, uint32_t level,
                            const uint8_t *target, size_t target_len,
                            const uint8_t *message, size_t message_len);

typedef struct rnet_logger {
  uint32_t struct_size;
  uint32_t abi_version;
  rnet_log_fn log;
  void *user_data;
  uint32_t min_level;
} rnet_logger_t;

typedef void (*rnet_log_v2_fn)(
    void *user_data, uint64_t timestamp_unix_ms, uint32_t level,
    const uint8_t *event_name, size_t event_name_len, rnet_runtime_t runtime,
    rnet_endpoint_t endpoint, rnet_session_t session, uint32_t transport,
    int32_t error_code, uint64_t correlation_id, const uint8_t *message,
    size_t message_len);

typedef struct rnet_logger_v2 {
  uint32_t struct_size;
  uint32_t abi_version;
  rnet_log_v2_fn log;
  void *user_data;
  uint32_t min_level;
} rnet_logger_v2_t;

typedef struct rnet_config {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t worker_threads;
  uint32_t event_queue_capacity;
  uint32_t write_queue_capacity;
  uint32_t max_body_len;
  uint32_t max_datagram_size;
  const rnet_logger_t *logger;
} rnet_config_t;

/* Secure-by-default production configuration. A zero rekey threshold disables that trigger. */
typedef struct rnet_config_v3 {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t worker_threads;
  uint32_t event_queue_capacity;
  uint32_t write_queue_capacity;
  uint32_t max_body_len;
  uint32_t max_datagram_size;
  uint64_t max_event_bytes;
  uint64_t max_runtime_queued_bytes;
  uint64_t max_session_queued_bytes;
  uint32_t max_sessions_per_endpoint;
  uint32_t max_sessions_per_ip;
  uint32_t handshake_rate_per_ip;
  uint32_t handshake_burst_per_ip;
  uint32_t ipv6_admission_prefix_bits;
  uint64_t handshake_timeout_ms;
  uint64_t connect_timeout_ms;
  uint64_t dns_timeout_ms;
  uint64_t datagram_idle_timeout_ms;
  uint32_t allow_plaintext_business_data;
  uint32_t allow_legacy_unauthenticated_endpoints;
  uint64_t rekey_after_ms;
  uint64_t rekey_after_bytes;
  const rnet_logger_t *logger;
  const rnet_logger_v2_t *logger_v2;
} rnet_config_v3_t;

/* V3 remains frozen. V4 appends TCP socket tuning without moving any V3 field. */
typedef struct rnet_config_v4 {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t worker_threads;
  uint32_t event_queue_capacity;
  uint32_t write_queue_capacity;
  uint32_t max_body_len;
  uint32_t max_datagram_size;
  uint64_t max_event_bytes;
  uint64_t max_runtime_queued_bytes;
  uint64_t max_session_queued_bytes;
  uint32_t max_sessions_per_endpoint;
  uint32_t max_sessions_per_ip;
  uint32_t handshake_rate_per_ip;
  uint32_t handshake_burst_per_ip;
  uint32_t ipv6_admission_prefix_bits;
  uint64_t handshake_timeout_ms;
  uint64_t connect_timeout_ms;
  uint64_t dns_timeout_ms;
  uint64_t datagram_idle_timeout_ms;
  uint32_t allow_plaintext_business_data;
  uint32_t allow_legacy_unauthenticated_endpoints;
  uint64_t rekey_after_ms;
  uint64_t rekey_after_bytes;
  const rnet_logger_t *logger;
  const rnet_logger_v2_t *logger_v2;
  uint32_t tcp_nodelay;
  /* Zero preserves the operating-system default. */
  uint64_t tcp_send_buffer_bytes;
  uint64_t tcp_recv_buffer_bytes;
} rnet_config_v4_t;

/* V5 appends runtime-wide endpoint and pending-handshake limits. */
typedef struct rnet_config_v5 {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t worker_threads;
  uint32_t event_queue_capacity;
  uint32_t write_queue_capacity;
  uint32_t max_body_len;
  uint32_t max_datagram_size;
  uint64_t max_event_bytes;
  uint64_t max_runtime_queued_bytes;
  uint64_t max_session_queued_bytes;
  uint32_t max_sessions_per_endpoint;
  uint32_t max_sessions_per_ip;
  uint32_t handshake_rate_per_ip;
  uint32_t handshake_burst_per_ip;
  uint32_t ipv6_admission_prefix_bits;
  uint64_t handshake_timeout_ms;
  uint64_t connect_timeout_ms;
  uint64_t dns_timeout_ms;
  uint64_t datagram_idle_timeout_ms;
  uint32_t allow_plaintext_business_data;
  uint32_t allow_legacy_unauthenticated_endpoints;
  uint64_t rekey_after_ms;
  uint64_t rekey_after_bytes;
  const rnet_logger_t *logger;
  const rnet_logger_v2_t *logger_v2;
  uint32_t tcp_nodelay;
  uint64_t tcp_send_buffer_bytes;
  uint64_t tcp_recv_buffer_bytes;
  uint32_t max_endpoints;
  uint32_t max_pending_handshakes;
} rnet_config_v5_t;

/* Called on an I/O worker thread. The callback and user_data must remain valid
 * until rnet_runtime_destroy returns. Return nonzero to trust the key. */
typedef uint32_t (*rnet_peer_verify_fn)(void *user_data,
                                        const uint8_t *public_key,
                                        size_t public_key_len);

typedef struct rnet_client_security {
  uint32_t struct_size;
  uint32_t abi_version;
  rnet_slice_t local_private_key;
  /* Used as an exact pin when verify_server is NULL. */
  rnet_slice_t expected_server_public_key;
  rnet_peer_verify_fn verify_server;
  void *user_data;
} rnet_client_security_t;

/* Generic server API: transport selects TCP, UDP, or KCP. */
typedef struct rnet_server_config_v2 {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t transport;
  uint32_t initial_security;
  rnet_slice_t bind_host;
  uint16_t bind_port;
  uint16_t reserved;
  rnet_slice_t local_private_key;
} rnet_server_config_v2_t;

/* Generic client API: security is inherited from runtime and server policy. */
typedef struct rnet_client_config_v2 {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t transport;
  uint32_t reserved0;
  rnet_slice_t remote_host;
  uint16_t remote_port;
  uint16_t reserved1;
  rnet_slice_t join_payload;
} rnet_client_config_v2_t;

/* Server bind hosts are numeric IPv4/IPv6 addresses. Client remote hosts may also be DNS names.
 * Host slices need not be NUL terminated. */
typedef struct rnet_endpoint_config {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t transport;
  uint32_t mode;
  rnet_slice_t bind_host;
  uint16_t bind_port;
  uint16_t reserved0;
  rnet_slice_t remote_host;
  uint16_t remote_port;
  uint16_t reserved1;
} rnet_endpoint_config_t;

typedef struct rnet_keypair {
  uint32_t struct_size;
  uint32_t abi_version;
  uint8_t private_key[32];
  uint8_t public_key[32];
} rnet_keypair_t;

typedef struct rnet_listener_config {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t transport;
  uint32_t reserved0;
  rnet_slice_t bind_host;
  uint16_t bind_port;
  uint16_t reserved1;
  rnet_slice_t local_private_key;
} rnet_listener_config_t;

typedef struct rnet_join_config {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t transport;
  uint32_t reserved0;
  rnet_slice_t remote_host;
  uint16_t remote_port;
  uint16_t reserved1;
  rnet_slice_t local_private_key;
  rnet_slice_t expected_server_public_key;
  rnet_slice_t join_payload;
} rnet_join_config_t;

typedef struct rnet_send_options {
  uint32_t struct_size;
  uint32_t abi_version;
  uint64_t correlation_id;
  uint32_t flags;
  uint32_t reserved;
} rnet_send_options_t;

typedef struct rnet_event {
  uint32_t struct_size;
  uint32_t event_type;
  rnet_endpoint_t endpoint;
  rnet_session_t session;
  uint32_t msg_type;
  uint32_t stream_id;
  uint64_t request_id;
  const uint8_t *data;
  size_t data_len;
  uint64_t buffer_token;
  int32_t status;
} rnet_event_t;

/* SECURITY_CHANGED data is mode:u8 followed by epoch:u64 in network byte order.
 * The event is emitted on both endpoints after their local barrier commits. */

typedef struct rnet_metrics {
  uint32_t struct_size;
  uint32_t abi_version;
  uint64_t frames_received;
  uint64_t frames_sent;
  uint64_t bytes_received;
  uint64_t bytes_sent;
  uint64_t events_dropped;
  uint64_t send_would_block;
  uint64_t protocol_errors;
  uint64_t logs_dropped;
  uint64_t logger_panics;
} rnet_metrics_t;

typedef struct rnet_latency_metric {
  uint32_t struct_size;
  uint32_t kind;
  uint64_t sample_count;
  uint64_t p50_us;
  uint64_t p90_us;
  uint64_t p95_us;
  uint64_t p99_us;
  uint64_t max_us;
} rnet_latency_metric_t;

typedef struct rnet_metrics_v2 {
  uint32_t struct_size;
  uint32_t abi_version;
  uint64_t frames_received;
  uint64_t frames_sent;
  uint64_t bytes_received;
  uint64_t bytes_sent;
  uint64_t events_dropped;
  uint64_t send_would_block;
  uint64_t protocol_errors;
  uint64_t lifecycle_events_rejected;
  uint64_t admission_rejected;
  uint64_t queued_send_bytes;
  uint64_t peak_queued_send_bytes;
  uint64_t queued_event_bytes;
  uint64_t session_closed_by_reason[19];
  uint64_t logs_dropped;
  uint64_t logger_panics;
} rnet_metrics_v2_t;

typedef struct rnet_metrics_v3 {
  uint32_t struct_size;
  uint32_t abi_version;
  uint64_t frames_received;
  uint64_t frames_sent;
  uint64_t bytes_received;
  uint64_t bytes_sent;
  uint64_t events_dropped;
  uint64_t send_would_block;
  uint64_t protocol_errors;
  uint64_t lifecycle_events_rejected;
  uint64_t admission_rejected;
  uint64_t queued_send_bytes;
  uint64_t peak_queued_send_bytes;
  uint64_t queued_event_bytes;
  uint64_t session_closed_by_reason[19];
  uint64_t logs_dropped;
  uint64_t logger_panics;
  uint64_t current_endpoints;
  uint64_t current_sessions;
  uint64_t established_sessions;
  uint64_t pending_handshakes;
  uint64_t peak_pending_handshakes;
  uint64_t admission_rejected_by_reason[8];
} rnet_metrics_v3_t;

typedef struct rnet_latency_metric_v2 {
  uint32_t struct_size;
  uint32_t kind;
  uint64_t sample_count;
  uint64_t p50_us;
  uint64_t p90_us;
  uint64_t p95_us;
  uint64_t p99_us;
  uint64_t p999_us;
  uint64_t max_us;
} rnet_latency_metric_v2_t;

uint32_t rnet_abi_version(void);
int32_t rnet_config_init(rnet_config_t *config);
int32_t rnet_config_v3_init(rnet_config_v3_t *config);
int32_t rnet_config_v4_init(rnet_config_v4_t *config);
int32_t rnet_config_v5_init(rnet_config_v5_t *config);
int32_t rnet_runtime_create(const rnet_config_t *config,
                            rnet_runtime_t *out);
int32_t rnet_runtime_create_v2(const rnet_config_t *config,
                               const rnet_client_security_t *client_security,
                               rnet_runtime_t *out);
int32_t rnet_runtime_create_v3(const rnet_config_v3_t *config,
                               const rnet_client_security_t *client_security,
                               rnet_runtime_t *out);
int32_t rnet_runtime_create_v4(const rnet_config_v4_t *config,
                               const rnet_client_security_t *client_security,
                               rnet_runtime_t *out);
int32_t rnet_runtime_create_v5(const rnet_config_v5_t *config,
                               const rnet_client_security_t *client_security,
                               rnet_runtime_t *out);
int32_t rnet_metrics_snapshot_v2(rnet_runtime_t runtime,
                                 rnet_metrics_v2_t *out);
int32_t rnet_metrics_snapshot_v3(rnet_runtime_t runtime,
                                 rnet_metrics_v3_t *out);
int32_t rnet_latency_snapshot_v2(rnet_runtime_t runtime,
                                 rnet_latency_metric_v2_t *metrics,
                                 size_t capacity, size_t *out_count,
                                 uint32_t drain_window);
int32_t rnet_server_open_v2(rnet_runtime_t runtime,
                            const rnet_server_config_v2_t *config,
                            rnet_endpoint_t *out);
int32_t rnet_client_connect_v2(rnet_runtime_t runtime,
                               const rnet_client_config_v2_t *config,
                               rnet_endpoint_t *out);
int32_t rnet_endpoint_open(rnet_runtime_t runtime,
                           const rnet_endpoint_config_t *config,
                           rnet_endpoint_t *out);
int32_t rnet_keypair_generate(rnet_keypair_t *out);
int32_t rnet_keypair_from_private(rnet_slice_t private_key,
                                  rnet_keypair_t *out);
int32_t rnet_listener_open(rnet_runtime_t runtime,
                           const rnet_listener_config_t *config,
                           rnet_endpoint_t *out);
int32_t rnet_client_join(rnet_runtime_t runtime,
                         const rnet_join_config_t *config,
                         rnet_endpoint_t *out);
int32_t rnet_session_auth_decide(rnet_runtime_t runtime,
                                 rnet_session_t session,
                                 uint32_t accept);
int32_t rnet_session_security_set(rnet_runtime_t runtime,
                                  rnet_session_t session, uint32_t mode);
int32_t rnet_session_rekey(rnet_runtime_t runtime,
                           rnet_session_t session);
int32_t rnet_endpoint_local_port(rnet_runtime_t runtime,
                                 rnet_endpoint_t endpoint,
                                 uint16_t *out_port);
int32_t rnet_send(rnet_runtime_t runtime, rnet_session_t session,
                  uint32_t msg_type, uint32_t stream_id,
                  rnet_slice_t payload, uint64_t request_id);
int32_t rnet_session_send(rnet_runtime_t runtime, rnet_session_t session,
                          uint32_t msg_type, rnet_slice_t payload);
int32_t rnet_session_send_ex(rnet_runtime_t runtime, rnet_session_t session,
                             uint32_t msg_type, rnet_slice_t payload,
                             const rnet_send_options_t *options);
size_t rnet_poll_events(rnet_runtime_t runtime, rnet_event_t *events,
                        size_t capacity, uint32_t timeout_ms);
int32_t rnet_poll_events_ex(rnet_runtime_t runtime, rnet_event_t *events,
                            size_t capacity, uint32_t timeout_ms,
                            size_t *out_count);
int32_t rnet_buffer_release(rnet_runtime_t runtime, uint64_t buffer_token);
int32_t rnet_session_close(rnet_runtime_t runtime, rnet_session_t session,
                           int32_t reason);
int32_t rnet_endpoint_close(rnet_runtime_t runtime,
                            rnet_endpoint_t endpoint);
/* On WOULD_BLOCK, poll pending events and retry to publish RUNTIME_STOPPED. */
int32_t rnet_runtime_stop(rnet_runtime_t runtime,
                          uint32_t drain_timeout_ms);
int32_t rnet_runtime_destroy(rnet_runtime_t runtime);
/* Thread-local UTF-8 diagnostic for the most recent failed call. The pointer is
 * valid until the next RNet call on the same thread and must not be freed. */
const char *rnet_last_error_message(void);
int32_t rnet_metrics_snapshot(rnet_runtime_t runtime,
                              rnet_metrics_t *out);
int32_t rnet_latency_snapshot(rnet_runtime_t runtime,
                              rnet_latency_metric_t *metrics,
                              size_t capacity, size_t *out_count);
/* A zero interval disables periodic summaries. Emission is driven by polling. */
int32_t rnet_metrics_log_interval_set(rnet_runtime_t runtime,
                                      uint64_t interval_ms);

#ifdef __cplusplus
}
#endif

#endif
