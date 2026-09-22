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
  RNET_GAME_PROFILE_REALTIME = 1,
  RNET_GAME_PROFILE_RELIABLE_REALTIME = 2,
  RNET_GAME_PROFILE_SESSION = 3
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
  RNET_EVENT_SECURITY_CHANGED = 11,
  /* Reserved for authenticated game-library controls; not application data. */
  RNET_EVENT_GAME_CONTROL = 12
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

/* Additive game facade. Game runtime handles are not interchangeable with
 * transport runtime handles. The application never supplies msg_type or stream_id. */
enum {
  RNET_GAME_RUNTIME_STARTED = 1,
  RNET_GAME_ENDPOINT_OPENED = 2,
  RNET_GAME_ENDPOINT_ERROR = 3,
  RNET_GAME_AUTH_REQUEST = 4,
  RNET_GAME_RESUME_REQUEST = 5,
  RNET_GAME_PROTOCOL_REJECTED = 6,
  RNET_GAME_SESSION_READY = 7,
  RNET_GAME_SESSION_RESUMED = 8,
  RNET_GAME_RESUME_TICKET = 9,
  RNET_GAME_SESSION_CLOSED = 10,
  RNET_GAME_MESSAGE = 11,
  RNET_GAME_WRITABLE = 12,
  RNET_GAME_JOIN_FAILED = 13,
  RNET_GAME_SECURITY_CHANGED = 14,
  RNET_GAME_QUALITY_CHANGED = 15,
  RNET_GAME_PROTOCOL_VIOLATION = 16,
  RNET_GAME_RUNTIME_STOPPED = 17
};

typedef struct rnet_game_config {
  uint32_t struct_size;
  uint32_t abi_version;
  /* Zero selects the production default. */
  uint32_t heartbeat_interval_ms;
  uint32_t heartbeat_timeout_ms;
  uint32_t allow_plaintext_business_data;
  uint32_t reserved;
  /* Optional borrowed V5 capacity/socket tuning; logger pointers must be NULL.
   * The game-level plaintext flag remains authoritative. */
  const rnet_config_v5_t *network_config;
} rnet_game_config_t;

typedef struct rnet_game_server_config {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t transport;
  uint32_t initial_encryption;
  rnet_slice_t bind_host; /* Numeric IPv4 or IPv6. */
  uint16_t bind_port;
  uint16_t reserved;
  rnet_slice_t local_private_key;
  uint64_t protocol_id;
  uint32_t protocol_version;
} rnet_game_server_config_t;

typedef struct rnet_game_client_config {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t transport;
  uint32_t reserved;
  rnet_slice_t remote_host; /* Hostname or numeric address. */
  uint16_t remote_port;
  uint16_t reserved2;
  rnet_slice_t join_ticket;
  uint64_t protocol_id;
  uint32_t protocol_version;
  uint32_t reserved3;
  uint64_t build_id;
  uint64_t capabilities;
} rnet_game_client_config_t;

/* Explicit wire-v4 range configs. The existing exact-version structs stay wire v3. */
typedef struct rnet_game_range_server_config {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t transport;
  uint32_t initial_encryption;
  rnet_slice_t bind_host;
  uint16_t bind_port;
  uint16_t reserved;
  rnet_slice_t local_private_key;
  uint64_t protocol_id;
  uint32_t min_version;
  uint32_t max_version;
} rnet_game_range_server_config_t;

typedef struct rnet_game_range_client_config {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t transport;
  uint32_t reserved;
  rnet_slice_t remote_host;
  uint16_t remote_port;
  uint16_t reserved2;
  rnet_slice_t join_ticket;
  uint64_t protocol_id;
  uint32_t min_version;
  uint32_t max_version;
  uint32_t reserved3;
  uint64_t build_id;
  uint64_t capabilities;
} rnet_game_range_client_config_t;

typedef struct rnet_game_event {
  uint32_t struct_size;
  uint32_t event_type;
  rnet_endpoint_t endpoint;
  rnet_session_t session;
  rnet_session_t related_session; /* Old session for resume events. */
  int32_t status;
  /* Join ticket, identity, resume ticket, or opaque business payload. */
  const uint8_t *data;
  size_t data_len;
  uint64_t buffer_token;
  /* The resume request's separate join ticket. */
  const uint8_t *aux_data;
  size_t aux_data_len;
  uint64_t aux_buffer_token;
  uint8_t client_public_key[32];
  uint64_t build_id;
  uint64_t capabilities;
  uint32_t encrypted;
  uint64_t security_epoch;
  uint32_t security_operation;
  /* Grades: 0 unknown, 1 excellent, 2 good, 3 fair, 4 poor.
   * Basis: 1 latency, 2 UDP sequence gap, 3 KCP retransmission. */
  uint32_t quality_grade;
  uint32_t quality_basis;
  uint64_t last_rtt_us;
  uint64_t jitter_us;
  uint64_t quality_samples;
  uint32_t has_sequence;
  uint32_t sequence;
  uint32_t has_tick;
  uint32_t tick;
} rnet_game_event_t;

/* Quality loss is a UDP sequence-gap estimate; KCP reports retransmission,
 * not raw IP loss. TCP has neither loss nor retransmission availability. */
typedef struct rnet_game_quality {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t available;
  uint32_t grade;
  uint32_t basis;
  uint32_t has_udp_loss;
  uint32_t has_kcp_retransmissions;
  uint32_t udp_recent_loss_per_mille;
  uint32_t kcp_recent_retransmission_per_mille;
  uint64_t last_rtt_us;
  uint64_t smoothed_rtt_us;
  uint64_t jitter_us;
  uint64_t samples;
  uint64_t udp_expected;
  uint64_t udp_missing;
  uint64_t kcp_segments_sent;
  uint64_t kcp_retransmitted;
} rnet_game_quality_t;

/* Four-timestamp sample. Offset maps the client runtime-local monotonic clock
 * to the server runtime-local monotonic clock; never use it as UTC or trust. */
typedef struct rnet_game_clock_sync {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t available; /* has_sample: 0 for server sessions or before the first sample */
  uint32_t reserved;
  int64_t server_minus_client_us;
  uint64_t rtt_us;
  uint64_t samples;
} rnet_game_clock_sync_t;

/* Cumulative, low-cardinality game telemetry. RTT values are cumulative
 * histogram estimates. logger_available distinguishes disabled logging from
 * a logger with zero drops; callback timing is recorded on the dispatch thread. */
typedef struct rnet_game_metrics {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t logger_available;
  uint32_t reserved;
  uint64_t heartbeat_probes_sent;
  uint64_t heartbeat_probe_send_failures;
  uint64_t heartbeat_replies_sent;
  uint64_t heartbeat_reply_send_failures;
  uint64_t heartbeat_replies_matched;
  uint64_t heartbeat_replies_rejected;
  uint64_t heartbeat_probes_rate_limited;
  uint64_t heartbeat_timeouts;
  uint64_t heartbeat_rtt_samples;
  uint64_t heartbeat_rtt_p50_us;
  uint64_t heartbeat_rtt_p90_us;
  uint64_t heartbeat_rtt_p95_us;
  uint64_t heartbeat_rtt_p99_us;
  uint64_t heartbeat_rtt_p999_us;
  uint64_t heartbeat_rtt_max_us;
  uint64_t resume_tickets_issued;
  uint64_t resume_requests_received;
  uint64_t resume_tickets_rejected;
  uint64_t resume_authorization_denied;
  uint64_t resume_pending_revoked;
  uint64_t resume_sessions_resumed;
  uint64_t resume_outstanding_tickets;
  uint64_t clock_probes_sent;
  uint64_t clock_replies_sent;
  uint64_t clock_samples;
  uint64_t clock_rejected;
  uint64_t clock_send_failures;
  uint64_t logger_dropped;
  uint64_t logger_sink_panics;
  uint64_t logger_callback_samples;
  uint64_t logger_callback_p99_us;
  uint64_t logger_callback_max_us;
} rnet_game_metrics_t;

/* Runtime-wide LatestOnly staging metrics. Gauges describe the current
 * pre-transport queue; forwarded counts transport admission, not delivery. */
typedef struct rnet_game_realtime_queue {
  uint32_t struct_size;
  uint32_t abi_version;
  uint64_t queued_messages;
  uint64_t queued_bytes;
  uint64_t admission_rejected;
  uint64_t replaced;
  uint64_t closed_dropped;
  uint64_t backpressure_dropped;
  uint64_t send_failed;
  uint64_t forwarded;
} rnet_game_realtime_queue_t;

/* Runtime-wide wire-v4 business data retained until readiness is observable.
 * Rejection fields are cumulative; all other usage fields are gauges. */
typedef struct rnet_game_range_buffer {
  uint32_t struct_size;
  uint32_t abi_version;
  uint64_t buffered_messages;
  uint64_t buffered_bytes;
  uint64_t peak_buffered_messages;
  uint64_t peak_buffered_bytes;
  uint64_t max_buffered_messages;
  uint64_t max_buffered_bytes;
  uint64_t session_admission_rejected;
  uint64_t runtime_admission_rejected;
} rnet_game_range_buffer_t;

/* Cumulative LatestOnly telemetry after the game staging queue. A worker
 * pickup is the boundary after which TCP/KCP data can no longer be recalled. */
typedef struct rnet_game_transport_latest {
  uint32_t struct_size;
  uint32_t abi_version;
  uint64_t pending_replaced;
  uint64_t worker_pickups;
  uint64_t admission_would_block;
  uint64_t admission_invalid_handle;
  uint64_t admission_invalid_state;
  uint64_t admission_handshake_required;
  uint64_t admission_not_supported;
  uint64_t admission_message_too_large;
  uint64_t admission_other_failures;
} rnet_game_transport_latest_t;

typedef struct rnet_game_buffer {
  uint32_t struct_size;
  uint32_t abi_version;
  const uint8_t *data;
  size_t len;
  uint64_t token;
} rnet_game_buffer_t;

int32_t rnet_game_config_init(rnet_game_config_t *out);
/* Expands a scenario profile into existing config fields without changing wire semantics. */
int32_t rnet_game_profile_defaults(uint32_t profile, uint32_t *out_transport,
                                   uint32_t *out_initial_encryption);
int32_t rnet_game_runtime_create(const rnet_game_config_t *config,
                                 const rnet_client_security_t *client_security,
                                 rnet_runtime_t *out);
/* Game records are delivered asynchronously without payloads, credentials or
 * tickets. The callback runtime field is the public game ABI runtime handle.
 * Logger user_data must stay valid until destroy returns. The callback may
 * query metrics but must not call runtime_stop/runtime_destroy. */
int32_t rnet_game_runtime_create_logged(
    const rnet_game_config_t *config,
    const rnet_client_security_t *client_security,
    const rnet_logger_v2_t *logger, rnet_runtime_t *out);
int32_t rnet_game_server_listen(rnet_runtime_t runtime,
                                const rnet_game_server_config_t *config,
                                rnet_endpoint_t *out);
int32_t rnet_game_server_listen_range(rnet_runtime_t runtime,
                                      const rnet_game_range_server_config_t *config,
                                      rnet_endpoint_t *out);
int32_t rnet_game_client_connect(rnet_runtime_t runtime,
                                 const rnet_game_client_config_t *config,
                                 rnet_endpoint_t *out);
int32_t rnet_game_client_connect_range(rnet_runtime_t runtime,
                                       const rnet_game_range_client_config_t *config,
                                       rnet_endpoint_t *out);
/* Resume always yields new network handles, then requires another AUTH decision.
 * The server emits RESUME_REQUEST and both sides receive SESSION_RESUMED. */
int32_t rnet_game_client_resume_connect(
    rnet_runtime_t runtime, const rnet_game_client_config_t *config,
    rnet_session_t old_session, rnet_slice_t resume_ticket,
    rnet_endpoint_t *out);
int32_t rnet_game_client_resume_connect_range(
    rnet_runtime_t runtime, const rnet_game_range_client_config_t *config,
    rnet_session_t old_session, rnet_slice_t resume_ticket,
    rnet_endpoint_t *out);
/* Query succeeds for a selected wire-v4 session, including pending server auth. */
int32_t rnet_game_selected_protocol_version(rnet_runtime_t runtime,
                                             rnet_session_t session, uint32_t *out);
int32_t rnet_game_issue_resume_ticket(rnet_runtime_t runtime,
                                      rnet_session_t session,
                                      rnet_slice_t identity);
int32_t rnet_game_endpoint_local_port(rnet_runtime_t runtime,
                                      rnet_endpoint_t endpoint, uint16_t *out_port);
int32_t rnet_game_auth_decide(rnet_runtime_t runtime,
                              rnet_session_t session, uint32_t accept);
int32_t rnet_game_send(rnet_runtime_t runtime, rnet_session_t session,
                       rnet_slice_t payload);
/* Best-effort coalescing before transport admission; already admitted reliable
 * messages cannot be withdrawn. The key is scoped to one session. */
int32_t rnet_game_send_latest(rnet_runtime_t runtime, rnet_session_t session,
                              uint64_t key, rnet_slice_t payload);
int32_t rnet_game_session_close(rnet_runtime_t runtime, rnet_session_t session);
int32_t rnet_game_endpoint_close(rnet_runtime_t runtime,
                                 rnet_endpoint_t endpoint);
int32_t rnet_game_rekey(rnet_runtime_t runtime, rnet_session_t session);
int32_t rnet_game_security_set(rnet_runtime_t runtime,
                               rnet_session_t session, uint32_t encrypted);
int32_t rnet_game_network_quality(rnet_runtime_t runtime,
                                  rnet_session_t session,
                                  rnet_game_quality_t *out);
int32_t rnet_game_clock_micros(rnet_runtime_t runtime, uint64_t *out);
int32_t rnet_game_clock_sync_snapshot(rnet_runtime_t runtime,
                                      rnet_session_t session,
                                      rnet_game_clock_sync_t *out);
int32_t rnet_game_metrics_snapshot(rnet_runtime_t runtime,
                                   rnet_game_metrics_t *out);
int32_t rnet_game_realtime_queue_snapshot(
    rnet_runtime_t runtime, rnet_game_realtime_queue_t *out);
int32_t rnet_game_range_buffer_snapshot(rnet_runtime_t runtime,
                                        rnet_game_range_buffer_t *out);
/* TCP/KCP snapshots replaced after game staging but before I/O worker pickup. */
int32_t rnet_game_transport_latest_replacements(rnet_runtime_t runtime,
                                                 uint64_t *out);
int32_t rnet_game_transport_latest_snapshot(
    rnet_runtime_t runtime, rnet_game_transport_latest_t *out);
/* Prometheus text has no per-player labels; release its nonzero token. */
int32_t rnet_game_prometheus_snapshot(rnet_runtime_t runtime,
                                      rnet_game_buffer_t *out);
/* Both nonzero event buffer tokens must be released separately. Sensitive
 * event bytes are erased by the library when their tokens are released. */
int32_t rnet_game_poll_events(rnet_runtime_t runtime, rnet_game_event_t *events,
                              size_t capacity, uint32_t timeout_ms,
                              size_t *out_count);
int32_t rnet_game_buffer_release(rnet_runtime_t runtime, uint64_t token);
int32_t rnet_game_runtime_stop(rnet_runtime_t runtime,
                               uint32_t drain_timeout_ms);
int32_t rnet_game_runtime_destroy(rnet_runtime_t runtime);

#ifdef __cplusplus
}
#endif

#endif
