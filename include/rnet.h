#ifndef RNET_H
#define RNET_H

/**
 * @file
 * Current C ABI for authenticated transport and game runtimes.
 *
 * Call rnet_abi_version before any initializer or output-struct call. Deploy
 * SDK, headers and native library together; no older layouts are supported.
 * Input layout headers must contain exact sizeof(struct) and RNET_ABI_VERSION;
 * reserved fields are zero. Required pointers must address aligned, live storage
 * for the entire declared object/length; NULL checks cannot validate bad pointers.
 *
 * Unless specified otherwise, int32_t functions return RNET_OK on success or
 * RNET_E_* on failure. Read rnet_last_error_message immediately on the same thread
 * for details. Outputs are only usable on success, except poll out_count describes
 * any reported exported prefix whose nonzero tokens must be released even on failure.
 * Input slices/configurations are borrowed for the call and copied as needed;
 * callback code and user_data instead remain valid through runtime destruction.
 *
 * Handles are local to their owning runtime; endpoint identifies a listener or
 * outbound connection, session identifies one peer. Game and transport runtime
 * families cannot be mixed. Session/endpoint/token numbers can coincide across
 * runtimes: the caller must retain their owner, not rely on cross-runtime rejection.
 * Use a single event consumer for deterministic ordering.
 * Synchronize destruction against all calls, callbacks and borrowed event data.
 *
 * Sending successfully admits local work, not remote delivery. Applications own
 * authentication decisions, backpressure policy and continued polling. Games
 * should use rnet_game_*: transport APIs alone do not supply game negotiation,
 * heartbeats, resume or game envelopes. Never mix their wire payload formats.
 */

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Exact SDK/native ABI layout identifier, not the application protocol version. */
#define RNET_ABI_VERSION 4

/** Opaque owning-runtime handle; zero is invalid. */
typedef uint64_t rnet_runtime_t;
/** Runtime-local listener/outbound-connection handle, not a send destination. */
typedef uint64_t rnet_endpoint_t;
/** Runtime-local peer handle used for send/auth/security operations. */
typedef uint64_t rnet_session_t;

enum {
  RNET_OK = 0, /**< Success; no local library error. */
  RNET_E_INVALID_ARGUMENT = -1, /**< Invalid value, layout, pointer presence or unsupported field combination. */
  RNET_E_INVALID_HANDLE = -2, /**< Unknown, stale, foreign or already released handle/token. */
  RNET_E_INVALID_STATE = -3, /**< Operation is not allowed in the current lifecycle/security state. */
  RNET_E_WOULD_BLOCK = -4, /**< Bounded admission or lifecycle progress blocked; poll/drain and retry appropriately. */
  RNET_E_TIMEOUT = -5, /**< Local operation/peer deadline expired. */
  RNET_E_NOT_SUPPORTED = -6, /**< Operation not supported by the selected transport/role. */
  RNET_E_IO_ERROR = -7, /**< Local socket/runtime I/O failure; inspect diagnostic and affected handle. */
  RNET_E_PROTOCOL_ERROR = -8, /**< Invalid network protocol/frame or incompatible peer behavior. */
  RNET_E_MESSAGE_TOO_LARGE = -9, /**< Payload exceeds the applicable frame/datagram/budget limit. */
  RNET_E_INTERNAL_PANIC = -10, /**< Caught internal Rust panic; not a peer's business error. */
  RNET_E_HANDSHAKE_REQUIRED = -11, /**< Session is not yet ready for the requested operation. */
  RNET_E_HANDSHAKE_FAILED = -12, /**< Cryptographic or protected join exchange failed. */
  RNET_E_AUTH_REJECTED = -13, /**< Server application authorization denied. */
  RNET_E_PEER_KEY_MISMATCH = -14, /**< Presented server key was not trusted by the client. */
  RNET_E_REPLAY_DETECTED = -15, /**< Protected sequence/replay validation failed. */
  RNET_E_CRYPTO_ERROR = -16, /**< Encryption/decryption/authentication operation failed. */
  RNET_E_RATE_LIMITED = -17, /**< Peer admission or request rate limit exceeded. */
  RNET_E_CANCELLED = -18 /**< Operation/session explicitly cancelled. */
};

enum {
  RNET_TRANSPORT_TCP = 1, /**< Reliable ordered stream transport with library message framing. */
  RNET_TRANSPORT_UDP = 2, /**< Unreliable message transport; no retransmission or delivery guarantee. */
  RNET_TRANSPORT_KCP = 3 /**< Reliable ordered KCP delivery over UDP. */
};

enum {
  RNET_GAME_PROFILE_REALTIME = 1, /**< UDP with encryption enabled; application tolerates losses. */
  RNET_GAME_PROFILE_RELIABLE_REALTIME = 2, /**< KCP with encryption enabled; reliable real-time traffic. */
  RNET_GAME_PROFILE_SESSION = 3 /**< TCP with encryption enabled; ordered session traffic. */
};

enum {
  RNET_SECURITY_PLAINTEXT = 1, /**< Unprotected business bytes; protected controls and peer handshake remain enabled. */
  RNET_SECURITY_ENCRYPTED = 2 /**< Authenticated encrypted business bytes and protected controls. */
};

enum {
  RNET_SECURITY_OPERATION_MODE_SWITCH = 1, /**< Business encryption policy barrier completed locally. */
  RNET_SECURITY_OPERATION_REKEY = 2 /**< Key-generation barrier completed locally. */
};

enum {
  RNET_EVENT_RUNTIME_STARTED = 1, /**< Runtime started; endpoint/session zero. */
  RNET_EVENT_ENDPOINT_OPENED = 2, /**< Endpoint socket became available; not a ready session. */
  RNET_EVENT_ENDPOINT_ERROR = 3, /**< Endpoint-related failure; may be recoverable, query endpoint before assuming closure. */
  RNET_EVENT_SESSION_OPENED = 4, /**< Transport handshake and authorization completed; not game readiness. */
  RNET_EVENT_SESSION_CLOSED = 5, /**< Peer closed; status is local reason, data may contain a diagnostic. */
  RNET_EVENT_MESSAGE = 6, /**< Opaque transport payload with optional correlation metadata. */
  RNET_EVENT_WRITABLE = 7, /**< Advisory send-capacity recovery; another send can still encounter backpressure. */
  RNET_EVENT_RUNTIME_STOPPED = 8, /**< Runtime stop completed; release remaining buffers before destroy. */
  RNET_EVENT_AUTH_REQUEST = 9, /**< Server must decide; data contains key[32] followed by opaque join payload. */
  RNET_EVENT_JOIN_FAILED = 10, /**< Outbound connection/handshake failed; status/detail explain the failure. */
  RNET_EVENT_SECURITY_CHANGED = 11, /**< Local barrier committed; data has mode, big-endian epoch, operation (10 bytes). */
  /* Reserved for authenticated game-library controls; not application data. */
  RNET_EVENT_GAME_CONTROL = 12 /**< Internal protected game-control frame; not business data. */
};

enum {
  RNET_LOG_TRACE = 0, /**< Most verbose diagnostics. */
  RNET_LOG_DEBUG = 1, /**< Development/debug diagnostics. */
  RNET_LOG_INFO = 2, /**< Normal lifecycle/summary information. */
  RNET_LOG_WARN = 3, /**< Recoverable issue or anomalous peer behavior. */
  RNET_LOG_ERROR = 4 /**< Operation/endpoint/runtime failure. */
};

enum {
  RNET_LATENCY_CONNECT = 1, /**< TCP connect operation duration. */
  RNET_LATENCY_CRYPTO_HANDSHAKE = 2, /**< Cryptographic handshake duration. */
  RNET_LATENCY_AUTH_WAIT = 3, /**< Server application authorization wait duration. */
  RNET_LATENCY_SEND_QUEUE = 4, /**< Time waiting for transport send processing. */
  RNET_LATENCY_EVENT_QUEUE = 5, /**< Time waiting for application event polling. */
  RNET_LATENCY_KCP_RTT = 6, /**< KCP acknowledgement-based RTT samples, not game heartbeat RTT. */
  RNET_LATENCY_KCP_UPDATE_DELAY = 7, /**< Observed KCP update scheduling delay. */
  RNET_LATENCY_LOGGER_CALLBACK = 8 /**< Time spent executing the installed log callback. */
};

/** Length-delimited borrowed bytes; strings use UTF-8 without a required terminator. */
typedef struct rnet_slice {
  const uint8_t *ptr; /**< Borrowed bytes; may be NULL only when len is zero. */
  size_t len; /**< Byte count, not a NUL-terminated string length including its terminator. */
} rnet_slice_t;

/**
 * Logging callback: user_data is the registered context; timestamp_unix_ms is UTC
 * milliseconds, level is RNET_LOG_*. event_name/message are borrowed length-delimited
 * UTF-8 valid only during this call, not necessarily NUL-terminated. Handles,
 * transport and correlation_id describe the source; zero means unspecified.
 * error_code is RNET_OK or the local RNET_E_* category.
 * Runs serially on this runtime's logging thread; keep it short to avoid bounded
 * queue drops. Must not throw/unwind across C, or call stop/destroy from the callback.
 */
typedef void (*rnet_log_fn)(
    void *user_data, uint64_t timestamp_unix_ms, uint32_t level,
    const uint8_t *event_name, size_t event_name_len, rnet_runtime_t runtime,
    rnet_endpoint_t endpoint, rnet_session_t session, uint32_t transport,
    int32_t error_code, uint64_t correlation_id, const uint8_t *message,
    size_t message_len);

/** Optional bounded asynchronous log sink. Descriptor is copied; context is not owned. */
typedef struct rnet_logger {
  uint32_t struct_size; /**< Set to sizeof this exact struct; initializers fill it when provided. */
  uint32_t abi_version; /**< Set to RNET_ABI_VERSION; only the exact current layout is accepted. */
  rnet_log_fn log; /**< Required callback; called serially on a dedicated logging thread. */
  void *user_data; /**< Opaque callback context; must remain valid until runtime destruction returns. */
  uint32_t min_level; /**< Minimum RNET_LOG_* severity, inclusive; values above ERROR clamp to ERROR. */
} rnet_logger_t;

/** Runtime resource/security settings; initialize with rnet_config_init, then customize.
 * Copied at creation; editing this struct afterwards does not reconfigure a runtime. */
typedef struct rnet_config {
  uint32_t struct_size; /**< Set to sizeof this exact struct; initializers fill it when provided. */
  uint32_t abi_version; /**< Set to RNET_ABI_VERSION; only the exact current layout is accepted. */
  uint32_t worker_threads; /**< Positive I/O worker count; initialize defaults with rnet_config_init. */
  uint32_t event_queue_capacity; /**< Positive transport event count limit; includes lifecycle events. */
  uint32_t write_queue_capacity; /**< Positive outbound command queue capacity in entries. */
  uint32_t max_body_len; /**< Positive logical frame body limit in bytes; game envelopes consume this budget too. */
  uint32_t max_datagram_size; /**< UDP/KCP datagram byte ceiling including protocol/security overhead; 20..65507, default 1200. */
  uint64_t max_event_bytes; /**< Positive total queued event payload budget in bytes. */
  uint64_t max_runtime_queued_bytes; /**< Positive runtime-wide send byte budget including framing reservations. */
  uint64_t max_session_queued_bytes; /**< Positive per-session send budget, at most max_runtime_queued_bytes. */
  uint32_t max_sessions_per_endpoint; /**< Positive active/pending session cap per listener/endpoint. */
  uint32_t max_sessions_per_ip; /**< Positive peer admission cap per IPv4 address or configured IPv6 prefix. */
  uint32_t handshake_rate_per_ip; /**< Positive handshake token refill rate per second and IP/prefix. */
  uint32_t handshake_burst_per_ip; /**< Positive handshake token-bucket burst allowance per IP/prefix. */
  uint32_t ipv6_admission_prefix_bits; /**< IPv6 admission prefix length, 0..128; zero groups all IPv6 peers, IPv4 uses full addresses. */
  uint64_t handshake_timeout_ms; /**< Positive cryptographic/application authorization deadline in milliseconds. */
  uint64_t connect_timeout_ms; /**< Positive TCP connection-attempt timeout in milliseconds. */
  uint64_t dns_timeout_ms; /**< Positive synchronous DNS resolution wait limit in milliseconds. */
  uint64_t datagram_idle_timeout_ms; /**< Positive established UDP/KCP idle timeout in milliseconds. */
  uint32_t allow_plaintext_business_data; /**< Zero rejects local server plaintext mode; nonzero permits it. Clients follow server policy. */
  uint64_t rekey_after_ms; /**< Server automatic key-rotation age in milliseconds; zero disables this trigger. */
  uint64_t rekey_after_bytes; /**< Server automatic protected-byte rotation threshold; zero disables this trigger. */
  const rnet_logger_t *logger; /**< Optional logger descriptor, copied at creation; callback context remains caller-owned. */
  uint32_t tcp_nodelay; /**< Zero disables TCP_NODELAY; nonzero enables it; initializer enables it. */
  uint64_t tcp_send_buffer_bytes; /**< Requested TCP kernel send buffer bytes; zero keeps OS defaults. */
  uint64_t tcp_recv_buffer_bytes; /**< Requested TCP kernel receive buffer bytes; zero keeps OS defaults. */
  uint32_t max_endpoints; /**< Positive runtime endpoint cap, including listeners and outbound client endpoints. */
  uint32_t max_pending_handshakes; /**< Positive runtime-wide pending-handshake admission cap. */
} rnet_config_t;

/**
 * Trust decision for a server public_key (public_key_len is 32), borrowed only
 * during the call; user_data is the registered context. Nonzero accepts, zero
 * rejects. May run concurrently on I/O workers: context must be thread-safe.
 * Callback/context must outlive runtime destruction. Do not block, throw/unwind
 * across C, or stop/destroy this runtime from an I/O callback.
 */
typedef uint32_t (*rnet_peer_verify_fn)(void *user_data,
                                        const uint8_t *public_key,
                                        size_t public_key_len);

/** Client identity and server trust for all outbound connections from a runtime. */
typedef struct rnet_client_security {
  uint32_t struct_size; /**< Set to sizeof this exact struct; initializers fill it when provided. */
  uint32_t abi_version; /**< Set to RNET_ABI_VERSION; only the exact current layout is accepted. */
  rnet_slice_t local_private_key; /**< Exactly 32 private-key bytes, copied during creation; caller erases its own copy. */
  /* Used as an exact pin when verify_server is NULL. */
  rnet_slice_t expected_server_public_key; /**< Exactly 32 pinned public-key bytes when verify_server is NULL; otherwise ignored. */
  rnet_peer_verify_fn verify_server; /**< Optional server-key trust callback; overrides the exact public-key pin. */
  void *user_data; /**< Opaque verifier context; must outlive runtime destruction and in-flight verification. */
} rnet_client_security_t;

/** Authenticated listener configuration; transport is fixed for its lifetime. */
typedef struct rnet_server_config {
  uint32_t struct_size; /**< Set to sizeof this exact struct; initializers fill it when provided. */
  uint32_t abi_version; /**< Set to RNET_ABI_VERSION; only the exact current layout is accepted. */
  uint32_t transport; /**< One RNET_TRANSPORT_* value, fixed for this listener and its sessions. */
  uint32_t initial_security; /**< RNET_SECURITY_* business-data mode; plaintext requires runtime opt-in. */
  rnet_slice_t bind_host; /**< UTF-8 numeric IPv4/IPv6 without brackets, URL scheme or port; DNS is not accepted. */
  uint16_t bind_port; /**< Local port; zero asks the OS to choose an ephemeral port. */
  uint16_t reserved; /**< Reserved; must be zero. */
  rnet_slice_t local_private_key; /**< Exactly 32 server identity bytes, copied by listen; never log them. */
} rnet_server_config_t;

/** Outbound endpoint configuration; identity/trust belong to the runtime. */
typedef struct rnet_client_config {
  uint32_t struct_size; /**< Set to sizeof this exact struct; initializers fill it when provided. */
  uint32_t abi_version; /**< Set to RNET_ABI_VERSION; only the exact current layout is accepted. */
  uint32_t transport; /**< RNET_TRANSPORT_* matching the listener; fixed for the connection. */
  uint32_t reserved0; /**< Reserved; must be zero. */
  rnet_slice_t remote_host; /**< UTF-8 DNS name or numeric IP, without URL scheme, path or embedded port. */
  uint16_t remote_port; /**< Remote service port; must be nonzero. */
  uint16_t reserved1; /**< Reserved; must be zero. */
  rnet_slice_t join_payload; /**< Opaque credential/application join bytes inside the protected handshake; copied on connect. */
} rnet_client_config_t;

/** Generated identity; contains secret material that the caller must erase. */
typedef struct rnet_keypair {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t abi_version; /**< Output ABI identifier, written by the library. */
  uint8_t private_key[32]; /**< Secret 32-byte key; caller must erase it when no longer needed. */
  uint8_t public_key[32]; /**< Corresponding 32-byte public key for authenticated peer trust configuration. */
} rnet_keypair_t;

/** Optional transport trace metadata; zero-initialize then set the exact ABI header. */
typedef struct rnet_send_options {
  uint32_t struct_size; /**< Set to sizeof this exact struct; initializers fill it when provided. */
  uint32_t abi_version; /**< Set to RNET_ABI_VERSION; only the exact current layout is accepted. */
  uint64_t correlation_id; /**< Optional request/trace metadata copied to the peer; zero means absent. */
  uint32_t flags; /**< Must be zero; no user-selectable framing flags exist in this ABI. */
  uint32_t reserved; /**< Reserved; must be zero. */
} rnet_send_options_t;

/** Transport event. Release every nonzero token, including diagnostic/auth payloads. */
typedef struct rnet_event {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t event_type; /**< One RNET_EVENT_* discriminant; determines the meaning of data and status. */
  rnet_endpoint_t endpoint; /**< Local owning endpoint; zero for runtime-wide events. */
  rnet_session_t session; /**< Local session; zero for runtime/endpoint events or failures before allocation. */
  uint64_t correlation_id; /**< MESSAGE request/trace metadata; zero when absent. */
  const uint8_t *data; /**< Owned-by-library payload or diagnostic bytes; valid until buffer_token is released or runtime destroyed. */
  size_t data_len; /**< Number of data bytes; no trailing NUL is implied. Zero permits a NULL data pointer. */
  uint64_t buffer_token; /**< Release each nonzero token exactly once using this runtime's rnet_buffer_release. */
  int32_t status; /**< Local RNET_OK or negative RNET_E_* category; not a peer's business error code. */
} rnet_event_t;

/* SESSION_CLOSED / JOIN_FAILED / ENDPOINT_ERROR may carry optional local diagnostic UTF-8 text.
 * Diagnostic bytes can be absent under event-byte pressure; status remains authoritative.
 * Release every nonzero buffer_token, including on non-message events. */

/* SECURITY_CHANGED data is exactly mode:u8 | epoch:u64 big-endian | operation:u8.
 * Both peers emit it after their local barrier commits. AUTH_REQUEST data is
 * client_public_key[32] followed by the opaque join payload. */

/** Runtime-wide counters/gauges; concurrent fields are not a globally atomic snapshot. */
typedef struct rnet_metrics {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t abi_version; /**< Output ABI identifier, written by the library. */
  uint64_t frames_received; /**< Cumulative logical frames received, not raw IP packets. */
  uint64_t frames_sent; /**< Cumulative logical frames accepted for transport sending, not remote acknowledgements. */
  uint64_t bytes_received; /**< Cumulative decoded frame body bytes received; excludes outer framing. */
  uint64_t bytes_sent; /**< Cumulative logical encoded frame bytes sent; includes logical framing. */
  uint64_t events_dropped; /**< Cumulative transport event queue admission failures. */
  uint64_t send_would_block; /**< Cumulative transport send backpressure rejections. */
  uint64_t protocol_errors; /**< Cumulative malformed/invalid transport-protocol input observations. */
  uint64_t lifecycle_events_rejected; /**< Cumulative failed lifecycle-event admissions; cleanup does not depend on delivery. */
  uint64_t admission_rejected; /**< Cumulative endpoint/session/handshake admission failures. */
  uint64_t queued_send_bytes; /**< Current send byte reservations; includes framing, not total process RSS. */
  uint64_t peak_queued_send_bytes; /**< Highest runtime send byte reservation observed. */
  uint64_t queued_event_bytes; /**< Current payload bytes awaiting transport poll; excludes exported buffer tokens. */
  uint64_t session_closed_by_reason[19]; /**< Cumulative closes indexed by negated error code, 0..18 (RNET_OK maps to 0). */
  uint64_t logs_dropped; /**< Cumulative bounded logger admission drops; zero also when no logger is installed. */
  uint64_t logger_panics; /**< Cumulative caught Rust logger panics; foreign exceptions must never cross the C boundary. */
  uint64_t current_endpoints; /**< Current registered endpoints, including listeners and client connections. */
  uint64_t current_sessions; /**< Current registered sessions including unfinished handshakes. */
  uint64_t established_sessions; /**< Current transport-open sessions; not proof of game readiness. */
  uint64_t pending_handshakes; /**< Current sessions awaiting transport handshake/authorization. */
  uint64_t peak_pending_handshakes; /**< Highest pending-handshake count observed. */
  uint64_t admission_rejected_by_reason[8]; /**< Counts: endpoint, endpoint-session, pending, IP-active, IP-rate, IP-table, KCP-peer, unauthenticated. */
} rnet_metrics_t;

/** Approximate histogram percentiles; microseconds, with sample_count indicating availability. */
typedef struct rnet_latency_metric {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t kind; /**< RNET_LATENCY_* operation sampled; units below are microseconds. */
  uint64_t sample_count; /**< Samples represented; zero means unavailable, not measured zero latency. */
  uint64_t p50_us; /**< Estimated median latency; histogram bucket approximation. */
  uint64_t p90_us; /**< Estimated 90th percentile latency. */
  uint64_t p95_us; /**< Estimated 95th percentile latency. */
  uint64_t p99_us; /**< Estimated 99th percentile latency. */
  uint64_t p999_us; /**< Estimated 99.9th percentile latency. */
  uint64_t max_us; /**< Largest sampled duration; concurrent snapshots are not atomic across all fields. */
} rnet_latency_metric_t;

/* Check this against RNET_ABI_VERSION before any initializer/output call.
 * Only this exact ABI/layout is supported. SDK, header and library must deploy together. */
/** Returns this library's exact ABI identifier; safe to call before layout checks. */
uint32_t rnet_abi_version(void);
/** Writes defaults to required config; this overwrites the full current layout. */
int32_t rnet_config_init(rnet_config_t *config);
/** Creates a transport runtime and writes its handle to required out.
 * config is required/initialized; client_security may be NULL for a server-only runtime.
 * Keys/configs are copied; callback contexts remain caller-owned. Does not listen/connect. */
int32_t rnet_runtime_create(const rnet_config_t *config,
                               const rnet_client_security_t *client_security,
                               rnet_runtime_t *out);
/** Writes runtime-wide cumulative counters and live gauges to required out; does not reset. */
int32_t rnet_metrics_snapshot(rnet_runtime_t runtime,
                                 rnet_metrics_t *out);
/** Writes histogram snapshots to metrics[capacity]; required out_count receives needed length.
 * capacity=0 permits NULL metrics and only queries length, without draining.
 * Positive insufficient capacity is INVALID_ARGUMENT. drain_window=0 reads cumulative
 * samples; nonzero rotates the window. Concurrent samples are not globally atomic. */
int32_t rnet_latency_snapshot(rnet_runtime_t runtime,
                                 rnet_latency_metric_t *metrics,
                                 size_t capacity, size_t *out_count,
                                 uint32_t drain_window);
/** Binds config's numeric address and fixed transport; writes endpoint to required out.
 * Port zero is supported (query local_port). Server authorization follows AUTH_REQUEST. */
int32_t rnet_server_listen(rnet_runtime_t runtime,
                            const rnet_server_config_t *config,
                            rnet_endpoint_t *out);
/** Starts an authenticated outbound join; writes endpoint to required out.
 * Requires client security at runtime creation. DNS may block up to its configured
 * timeout; connect success is not SESSION_OPENED. Continue polling and handle JOIN_FAILED. */
int32_t rnet_client_connect(rnet_runtime_t runtime,
                               const rnet_client_config_t *config,
                               rnet_endpoint_t *out);
/** Writes a fresh 32-byte private/public identity to required out; caller erases the secret. */
int32_t rnet_keypair_generate(rnet_keypair_t *out);
/** Copies exactly 32 private_key bytes and derives the public key into required out.
 * Caller remains responsible for erasing both input and output private-key copies. */
int32_t rnet_keypair_from_private(rnet_slice_t private_key,
                                  rnet_keypair_t *out);
/** Answers a pending server AUTH_REQUEST; accept must be 0 (deny) or 1 (allow).
 * Success queues the decision; wait for SESSION_OPENED before sending. */
int32_t rnet_session_auth_decide(rnet_runtime_t runtime,
                                 rnet_session_t session,
                                 uint32_t accept);
/** Server-only asynchronous business-mode switch to RNET_SECURITY_* mode.
 * Plaintext requires runtime opt-in and has no confidentiality/integrity. Controls
 * stay protected. Clients follow automatically; observe SECURITY_CHANGED for commit.
 * Requesting the already active mode may succeed without emitting another change event. */
int32_t rnet_session_security_set(rnet_runtime_t runtime,
                                  rnet_session_t session, uint32_t mode);
/** Server-only asynchronous key rotation. Observe SECURITY_CHANGED with REKEY operation;
 * admission is not completion. Does not change the business encryption policy. */
int32_t rnet_session_rekey(rnet_runtime_t runtime,
                           rnet_session_t session);
/** Writes an existing endpoint's bound local port to required out_port; invalid handles fail. */
int32_t rnet_endpoint_local_port(rnet_runtime_t runtime,
                                 rnet_endpoint_t endpoint,
                                 uint16_t *out_port);
/** Copies opaque payload for a transport-open session; success is local admission only.
 * Empty payloads are valid; oversize, unavailable session and backpressure return errors. */
int32_t rnet_session_send(rnet_runtime_t runtime, rnet_session_t session,
                          rnet_slice_t payload);
/** Like rnet_session_send with required, ABI-initialized options and optional correlation ID. */
int32_t rnet_session_send_ex(rnet_runtime_t runtime, rnet_session_t session,
                             rnet_slice_t payload,
                             const rnet_send_options_t *options);
/** Exports up to capacity events into events and writes required out_count.
 * Positive capacity requires an array of that many elements. timeout_ms bounds idle
 * waiting, not total call time; zero is nonblocking. capacity=0 allows NULL events,
 * validates runtime and drives periodic transport logs without consuming events.
 * Release all returned nonzero tokens, including the exported prefix if this call fails. */
int32_t rnet_poll_events(rnet_runtime_t runtime, rnet_event_t *events,
                            size_t capacity, uint32_t timeout_ms,
                            size_t *out_count);
/** Releases one nonzero exported transport buffer token on its owning runtime.
 * Data becomes invalid immediately. Zero/stale/already released tokens fail in that runtime;
 * token values are not globally unique, so using a foreign runtime is a caller error. */
int32_t rnet_buffer_release(rnet_runtime_t runtime, uint64_t buffer_token);
/** Closes session with a valid local RNET_OK/RNET_E_* reason; listener remains usable. */
int32_t rnet_session_close(rnet_runtime_t runtime, rnet_session_t session,
                           int32_t reason);
/** Closes endpoint and its sessions; later queries/sends fail. Does not stop the runtime. */
int32_t rnet_endpoint_close(rnet_runtime_t runtime,
                            rnet_endpoint_t endpoint);
/* On WOULD_BLOCK, poll pending events and retry to publish RUNTIME_STOPPED. */
/** Stops I/O with an optional send-drain budget in drain_timeout_ms (zero means no drain).
 * WOULD_BLOCK can mean the stopped event is pending: poll and retry. Keep the handle
 * for final events/buffer release, then destroy. Forbidden from logger callbacks. */
int32_t rnet_runtime_stop(rnet_runtime_t runtime,
                          uint32_t drain_timeout_ms);
/** Destroys a stopped runtime after all exported buffers are released and calls finish.
 * INVALID_STATE indicates lifecycle/outstanding buffers; WOULD_BLOCK indicates active
 * calls. On failure keep the handle and resolve/retry. Forbidden from logger callbacks. */
int32_t rnet_runtime_destroy(rnet_runtime_t runtime);
/* Thread-local UTF-8 diagnostic for the most recent failed call. The pointer is
 * valid until the next RNet call on the same thread and must not be freed. */
/** Returns borrowed thread-local UTF-8 detail; never free it. Read immediately after failure. */
const char *rnet_last_error_message(void);
/* A zero interval disables periodic summaries. Emission is driven by polling. */
/** Sets the poll-driven summary interval in milliseconds; zero disables summaries. */
int32_t rnet_metrics_log_interval_set(rnet_runtime_t runtime,
                                      uint64_t interval_ms);

/* Game facade. Game runtime handles are not interchangeable with
 * transport runtime handles. The application never supplies msg_type or stream_id. */
enum {
  RNET_GAME_RUNTIME_STARTED = 1, /**< Game runtime started; no peer readiness implied. */
  RNET_GAME_ENDPOINT_OPENED = 2, /**< Endpoint socket opened; continue polling for authorization/readiness. */
  RNET_GAME_ENDPOINT_ERROR = 3, /**< Endpoint-related failure; query validity rather than assuming terminal closure. */
  RNET_GAME_AUTH_REQUEST = 4, /**< Server authorization: data=login ticket, client key/build/capabilities identify request. */
  RNET_GAME_RESUME_REQUEST = 5, /**< Server reauthorization: data=identity, aux_data=fresh login, related_session=old handle. */
  RNET_GAME_PROTOCOL_REJECTED = 6, /**< Business protocol/range mismatch before application authorization. */
  RNET_GAME_SESSION_READY = 7, /**< Initial game join completed; session can send business data. */
  RNET_GAME_SESSION_RESUMED = 8, /**< Resume committed; session=new handle, related_session=old invalidated handle. */
  RNET_GAME_RESUME_TICKET = 9, /**< Client receives sensitive single-runtime, single-use resume credential in data. */
  RNET_GAME_SESSION_CLOSED = 10, /**< Session ended; status holds local close category. */
  RNET_GAME_MESSAGE = 11, /**< Opaque business data plus optional sequence/tick/correlation metadata. */
  RNET_GAME_WRITABLE = 12, /**< Advisory send-capacity recovery; not a reservation or delivery receipt. */
  RNET_GAME_JOIN_FAILED = 13, /**< Join/negotiation failed; status describes local category. */
  RNET_GAME_SECURITY_CHANGED = 14, /**< Local mode/key barrier completed; inspect encrypted/epoch/operation. */
  RNET_GAME_QUALITY_CHANGED = 15, /**< Quality classification changed; inspect grade/basis/RTT/jitter/samples. */
  RNET_GAME_PROTOCOL_VIOLATION = 16, /**< Game/control protocol rejected; never a business message. */
  RNET_GAME_RUNTIME_STOPPED = 17 /**< Game runtime stop completed; release buffers then destroy. */
};

enum {
  RNET_GAME_PRIORITY_LOW = 1, /**< Lowest local scheduler priority; no effect on remote dispatch. */
  RNET_GAME_PRIORITY_NORMAL = 2, /**< Default priority used by ordinary game send. */
  RNET_GAME_PRIORITY_HIGH = 3, /**< Higher local scheduler share, not a guaranteed deadline. */
  RNET_GAME_PRIORITY_CRITICAL = 4 /**< Highest local scheduler share; lower queues still receive service. */
};

/* Complete game configuration. Zero queue budgets select production defaults. */
/* Game records are delivered asynchronously without payloads, credentials or
 * tickets. The callback runtime field is the public game ABI runtime handle.
 * Logger user_data must stay valid until destroy returns. The callback may
 * query metrics but must not call runtime_stop/runtime_destroy. */
/** Game runtime options. Initialize first; zero fallback semantics are field-specific. */
typedef struct rnet_game_config {
  uint32_t struct_size; /**< Set to sizeof this exact struct; initializers fill it when provided. */
  uint32_t abi_version; /**< Set to RNET_ABI_VERSION; only the exact current layout is accepted. */
  uint32_t heartbeat_interval_ms; /**< Probe interval in milliseconds; both heartbeat fields zero select defaults (5 s / 15 s). */
  uint32_t heartbeat_timeout_ms; /**< Acknowledgement timeout; if either heartbeat field is nonzero, both must be positive and timeout >= interval. */
  uint32_t allow_plaintext_business_data; /**< 0/1 server plaintext permission; overrides the same field in network_config. */
  uint32_t reserved; /**< Reserved; must be zero. */
  const rnet_config_t *network_config; /**< NULL uses transport defaults; otherwise copied. Its logger must be NULL: use this config's logger. */
  uint64_t realtime_max_queued_bytes; /**< Snapshot staging bytes across runtime; zero selects 16 MiB. */
  uint64_t realtime_max_session_queued_bytes; /**< Per-session snapshot staging bytes; zero selects 256 KiB; cannot exceed total. */
  uint64_t realtime_max_keys_per_session; /**< Distinct pending snapshot keys per session; zero selects 64. */
  uint64_t realtime_flush_batch; /**< Snapshot forwarding attempts per poll; zero selects 256. */
  uint64_t scheduled_max_queued_bytes; /**< Ordinary/priority staging bytes across runtime; zero selects 16 MiB. */
  uint64_t scheduled_max_session_queued_bytes; /**< Per-session ordinary/priority staging bytes; zero selects 256 KiB; cannot exceed total. */
  uint64_t scheduled_max_queued_messages; /**< Ordinary/priority staging message cap; zero selects 65,536. */
  uint64_t scheduled_flush_batch; /**< Ordinary/priority forwarding attempts per poll; zero selects 256. */
  const rnet_logger_t *logger; /**< Optional async logger descriptor, copied at creation; userdata lives until destroy returns. */
} rnet_game_config_t;

/* Inclusive application versions. Equal min/max specifies an exact version. */
/** Listener policy and inclusive business-version range; copied by listen. */
typedef struct rnet_game_server_config {
  uint32_t struct_size; /**< Set to sizeof this exact struct; initializers fill it when provided. */
  uint32_t abi_version; /**< Set to RNET_ABI_VERSION; only the exact current layout is accepted. */
  uint32_t transport; /**< RNET_TRANSPORT_* fixed for the listener and accepted sessions. */
  uint32_t initial_encryption; /**< 0 plaintext or 1 encrypted business data; 0 requires runtime plaintext permission. */
  rnet_slice_t bind_host; /**< UTF-8 numeric IPv4/IPv6 only, without brackets, URL scheme or embedded port. */
  uint16_t bind_port; /**< Local listening port; zero requests an OS-assigned port. */
  uint16_t reserved; /**< Reserved; must be zero. */
  rnet_slice_t local_private_key; /**< Exactly 32 server identity bytes, copied by listen. */
  uint64_t protocol_id; /**< Nonzero application protocol identity, equal on both peers; not an SDK/transport version. */
  uint32_t min_version; /**< Nonzero lowest supported application version, inclusive. */
  uint32_t max_version; /**< Highest supported version, inclusive and >= min_version; equal bounds require an exact version. */
} rnet_game_server_config_t;

/** Outbound game join metadata. Connect returns an endpoint, not a ready session. */
typedef struct rnet_game_client_config {
  uint32_t struct_size; /**< Set to sizeof this exact struct; initializers fill it when provided. */
  uint32_t abi_version; /**< Set to RNET_ABI_VERSION; only the exact current layout is accepted. */
  uint32_t transport; /**< RNET_TRANSPORT_* matching the remote listener; fixed after connect. */
  uint32_t reserved; /**< Reserved; must be zero. */
  rnet_slice_t remote_host; /**< UTF-8 DNS hostname or numeric IP, not a URL; port is supplied separately. */
  uint16_t remote_port; /**< Remote listening port; nonzero. */
  uint16_t reserved2; /**< Reserved; must be zero. */
  rnet_slice_t join_ticket; /**< Opaque application credential, copied on connect; can be empty if server authorization allows. */
  uint64_t protocol_id; /**< Nonzero application protocol identity, matching the listener. */
  uint32_t min_version; /**< Nonzero minimum application version, inclusive. */
  uint32_t max_version; /**< Maximum application version, inclusive and >= min_version. */
  uint32_t reserved3; /**< Reserved; must be zero. */
  uint64_t build_id; /**< Authenticated client build metadata; zero unspecified, not interpreted by the library. */
  uint64_t capabilities; /**< Authenticated application-defined capability bits; zero none, not library feature grants. */
} rnet_game_client_config_t;

/** Game event; fields are meaningful only for the indicated event kind. Both tokens are owned. */
typedef struct rnet_game_event {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t event_type; /**< RNET_GAME_* event discriminant; only the documented fields for that event are meaningful. */
  rnet_endpoint_t endpoint; /**< Local endpoint; zero for runtime-wide events. */
  rnet_session_t session; /**< Current/pending local session, including NEW handle on SESSION_RESUMED; zero when not applicable. */
  rnet_session_t related_session; /**< Old local handle on RESUME_REQUEST/SESSION_RESUMED; zero otherwise. */
  int32_t status; /**< RNET_E_* on error/rejection/close events; other kinds (including PROTOCOL_VIOLATION) use RNET_OK. */
  /* Join ticket, identity, resume ticket, or opaque business payload. */
  const uint8_t *data; /**< AUTH_REQUEST login ticket; RESUME_REQUEST identity; RESUME_TICKET credential; MESSAGE business bytes. */
  size_t data_len; /**< data byte count, no NUL terminator; empty data may have NULL pointer. */
  uint64_t buffer_token; /**< Token owning data; release each nonzero token once before destroy. */
  /* The resume request's separate join ticket. */
  const uint8_t *aux_data; /**< RESUME_REQUEST's fresh login ticket, separate from the identity in data; otherwise empty. */
  size_t aux_data_len; /**< aux_data byte count; zero when absent. */
  uint64_t aux_buffer_token; /**< Independent ownership token for aux_data; release separately from buffer_token. */
  uint8_t client_public_key[32]; /**< Authenticated 32-byte client key on AUTH_REQUEST/RESUME_REQUEST. */
  uint64_t build_id; /**< Client build metadata on AUTH_REQUEST/RESUME_REQUEST. */
  uint64_t capabilities; /**< Client capability metadata on AUTH_REQUEST/RESUME_REQUEST; not authorization. */
  uint32_t encrypted; /**< SECURITY_CHANGED resulting business mode: 0 plaintext, 1 encrypted; controls stay protected. */
  uint64_t security_epoch; /**< SECURITY_CHANGED session-local key/mode generation, not a wall-clock time. */
  uint32_t security_operation; /**< SECURITY_CHANGED RNET_SECURITY_OPERATION_* value. */
  /* Grades: 0 unknown, 1 excellent, 2 good, 3 fair, 4 poor.
   * Basis: 1 latency, 2 UDP sequence gap, 3 KCP retransmission. */
  uint32_t quality_grade; /**< QUALITY_CHANGED grade: 0 unknown, 1 excellent, 2 good, 3 fair, 4 poor. */
  uint32_t quality_basis; /**< QUALITY_CHANGED basis: 1 latency, 2 latency+UDP gaps, 3 latency+KCP retransmission. */
  uint64_t last_rtt_us; /**< QUALITY_CHANGED latest authenticated heartbeat RTT in microseconds. */
  uint64_t jitter_us; /**< QUALITY_CHANGED smoothed absolute RTT variation in microseconds. */
  uint64_t quality_samples; /**< QUALITY_CHANGED accepted heartbeat sample count. */
  uint32_t has_sequence; /**< MESSAGE: 0 absent, 1 sender supplied optional sequence metadata. */
  uint32_t sequence; /**< MESSAGE sequence if has_sequence=1; not internal UDP loss numbering. */
  uint32_t has_tick; /**< MESSAGE: 0 absent, 1 sender supplied a simulation tick. */
  uint32_t tick; /**< MESSAGE simulation tick if has_tick=1; not a trusted clock. */
  uint64_t correlation_id; /**< MESSAGE optional request/trace metadata; zero means absent. */
} rnet_game_event_t;

/* Optional scheduling and correlation metadata; business message types belong in payloads. */
/** Game metadata/scheduling. Priority must be explicit; zero is not Normal. */
typedef struct rnet_game_send_options {
  uint32_t struct_size; /**< Set to sizeof this exact struct; initializers fill it when provided. */
  uint32_t abi_version; /**< Set to RNET_ABI_VERSION; only the exact current layout is accepted. */
  uint32_t has_sequence; /**< 0 omits sequence, 1 includes it; no ordering or deduplication is enabled. */
  uint32_t sequence; /**< Caller-defined sequence copied unchanged when has_sequence=1; zero is valid. */
  uint32_t has_tick; /**< 0 omits tick, 1 includes it. */
  uint32_t tick; /**< Caller-defined simulation tick copied unchanged when has_tick=1. */
  uint64_t correlation_id; /**< Optional request/trace identifier copied to the peer; zero absent. */
  uint32_t priority; /**< Required RNET_GAME_PRIORITY_*; zero is invalid. Only local scheduling is affected. */
  /* Relative local queue lifetime; zero means no expiry. */
  uint64_t expiry_ms; /**< Relative local staging lifetime in milliseconds; zero means no expiry, not immediate expiry. */
} rnet_game_send_options_t;

/* Quality loss is a UDP sequence-gap estimate; KCP reports retransmission,
 * not raw IP loss. TCP has neither loss nor retransmission availability. */
/** Heartbeat quality with transport-specific loss evidence, not universal packet loss. */
typedef struct rnet_game_quality {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t abi_version; /**< Output ABI identifier, written by the library. */
  uint32_t available; /**< 0 before the first heartbeat sample; 1 permits interpreting the remaining measurements. */
  uint32_t grade; /**< 0 unknown, 1 excellent, 2 good, 3 fair, 4 poor; sample minima may keep grade unknown. */
  uint32_t basis; /**< 1 latency only, 2 latency+UDP gaps, 3 latency+KCP retransmissions. */
  uint32_t has_udp_loss; /**< 0 unavailable, 1 UDP loss fields present; not the same as measured zero loss. */
  uint32_t has_kcp_retransmissions; /**< 0 unavailable, 1 KCP fields present; TCP has neither transport signal. */
  uint32_t udp_recent_loss_per_mille; /**< Recent UDP sequence-gap estimate per 1,000 positions (10 means 1%); check availability. */
  uint32_t kcp_recent_retransmission_per_mille; /**< Recent KCP retransmission ratio per 1,000 PUSH transmissions; not raw network loss. */
  uint64_t last_rtt_us; /**< Latest authenticated heartbeat RTT in microseconds, including queueing effects. */
  uint64_t smoothed_rtt_us; /**< Smoothed heartbeat RTT in microseconds; not a percentile. */
  uint64_t jitter_us; /**< Smoothed absolute RTT variation in microseconds. */
  uint64_t samples; /**< Accepted heartbeat samples on this session. */
  uint64_t udp_expected; /**< Cumulative expected UDP data positions, excluding implausible forward jumps. */
  uint64_t udp_missing; /**< Expected minus received UDP positions; late arrivals may reduce this value. */
  uint64_t kcp_segments_sent; /**< Cumulative KCP PUSH transmissions, including retransmissions, excluding ACKs. */
  uint64_t kcp_retransmitted; /**< Cumulative repeated KCP PUSH transmissions. */
} rnet_game_quality_t;

/* Four-timestamp sample. Offset maps the client runtime-local monotonic clock
 * to the server runtime-local monotonic clock; never use it as UTC or trust. */
/** Approximate client-side monotonic clock sample; not UTC or an anti-cheat time authority. */
typedef struct rnet_game_clock_sync {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t abi_version; /**< Output ABI identifier, written by the library. */
  uint32_t available; /**< 0 on server sessions or before first sample; 1 when a client sample is available. */
  uint32_t reserved; /**< Reserved output, written as zero by the library. */
  int64_t server_minus_client_us; /**< Signed microseconds added to client runtime clock to approximate server runtime clock. */
  uint64_t rtt_us; /**< Round-trip estimate in microseconds after subtracting reported server turnaround. */
  uint64_t samples; /**< Accepted clock samples on this client session, not heartbeat samples. */
} rnet_game_clock_sync_t;

/* Cumulative, low-cardinality game telemetry. RTT values are cumulative
 * histogram estimates. logger_available distinguishes disabled logging from
 * a logger with zero drops; callback timing is recorded on the dispatch thread. */
/** Cumulative game telemetry and current gauges; independent counters can race a snapshot. */
typedef struct rnet_game_metrics {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t abi_version; /**< Output ABI identifier, written by the library. */
  uint32_t logger_available; /**< 0 logging disabled, 1 logger installed; distinguishes no logger from zero drops. */
  uint32_t reserved; /**< Reserved output, written as zero by the library. */
  uint64_t heartbeat_probes_sent; /**< Cumulative protected probes admitted locally, not confirmed delivered. */
  uint64_t heartbeat_probe_send_failures; /**< Cumulative failed local probe sends. */
  uint64_t heartbeat_replies_sent; /**< Cumulative protected replies admitted locally. */
  uint64_t heartbeat_reply_send_failures; /**< Cumulative failed local reply sends. */
  uint64_t heartbeat_replies_matched; /**< Cumulative timely replies matching outstanding local challenges. */
  uint64_t heartbeat_replies_rejected; /**< Cumulative invalid/late challenge replies; do not refresh liveness. */
  uint64_t heartbeat_probes_rate_limited; /**< Cumulative inbound probes suppressed by response rate limits. */
  uint64_t heartbeat_timeouts; /**< Cumulative heartbeat deadlines causing session-close requests. */
  uint64_t heartbeat_rtt_samples; /**< Cumulative accepted RTT samples; zero means unavailable. */
  uint64_t heartbeat_rtt_p50_us; /**< Cumulative estimated heartbeat RTT median in microseconds. */
  uint64_t heartbeat_rtt_p90_us; /**< Cumulative estimated heartbeat RTT P90 in microseconds. */
  uint64_t heartbeat_rtt_p95_us; /**< Cumulative estimated heartbeat RTT P95 in microseconds. */
  uint64_t heartbeat_rtt_p99_us; /**< Cumulative estimated heartbeat RTT P99 in microseconds. */
  uint64_t heartbeat_rtt_p999_us; /**< Cumulative estimated heartbeat RTT P99.9 in microseconds. */
  uint64_t heartbeat_rtt_max_us; /**< Maximum sampled heartbeat RTT in microseconds. */
  uint64_t resume_tickets_issued; /**< Cumulative tickets issued and admitted to protected send, not confirmed delivered. */
  uint64_t resume_requests_received; /**< Cumulative decoded resume joins reaching ticket validation. */
  uint64_t resume_tickets_rejected; /**< Cumulative invalid, expired, replayed or incompatible ticket claims. */
  uint64_t resume_authorization_denied; /**< Cumulative valid resume claims denied by business authorization. */
  uint64_t resume_pending_revoked; /**< Cumulative pending/in-flight resume claims revoked by explicit cleanup. */
  uint64_t resume_sessions_resumed; /**< Cumulative completed resume publications on this runtime. */
  uint64_t resume_outstanding_tickets; /**< Current retained registry entries; expiry cleanup is opportunistic. */
  uint64_t clock_probes_sent; /**< Cumulative locally admitted client clock probes. */
  uint64_t clock_replies_sent; /**< Cumulative locally admitted server clock replies. */
  uint64_t clock_samples; /**< Cumulative accepted client clock samples. */
  uint64_t clock_rejected; /**< Cumulative rate-limited probes or tracker-rejected replies, not every parser error. */
  uint64_t clock_send_failures; /**< Cumulative probe generation/admission and reply send-admission failures. */
  uint64_t logger_dropped; /**< Cumulative records rejected by bounded logger queues. */
  uint64_t logger_sink_panics; /**< Cumulative caught sink panics; do not throw foreign exceptions through the callback. */
  uint64_t logger_callback_samples; /**< Completed callback samples, including caught panics. */
  uint64_t logger_callback_p99_us; /**< Cumulative estimated callback execution P99 in microseconds; excludes queue wait. */
  uint64_t logger_callback_max_us; /**< Largest measured callback execution time in microseconds. */
} rnet_game_metrics_t;

/* Runtime-wide LatestOnly staging metrics. Gauges describe the current
 * pre-transport queue; forwarded counts transport admission, not delivery. */
/** Game staging gauges and cumulative LatestOnly admission/forwarding/drop counters. */
typedef struct rnet_game_realtime_queue {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t abi_version; /**< Output ABI identifier, written by the library. */
  uint64_t queued_messages; /**< Current distinct pending (session,key) entries in game staging. */
  uint64_t queued_bytes; /**< Current payload plus game-envelope reservations, not allocator RSS. */
  uint64_t admission_rejected; /**< Cumulative staging budget/key-cap rejections, including failed requeues. */
  uint64_t replaced; /**< Cumulative old pending snapshots superseded by newer same-key values. */
  uint64_t closed_dropped; /**< Cumulative staged values discarded during session cleanup/shutdown. */
  uint64_t backpressure_dropped; /**< Cumulative values unable to reenter staging after transport backpressure. */
  uint64_t send_failed; /**< Cumulative values discarded after non-backpressure forwarding failures. */
  uint64_t forwarded; /**< Cumulative snapshots accepted by transport, not delivered to the peer. */
} rnet_game_realtime_queue_t;

/* Runtime-wide priority/expiry staging metrics. */
/** Priority-queue gauges/counters; expiry and delay apply only before transport admission. */
typedef struct rnet_game_scheduled_queue {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t abi_version; /**< Output ABI identifier, written by the library. */
  uint64_t queued_messages; /**< Current admitted messages awaiting forwarding or drop. */
  uint64_t queued_bytes; /**< Current payload plus worst-case game-envelope reservations. */
  uint64_t admission_rejected; /**< Cumulative staging byte/message budget rejections. */
  uint64_t expired_dropped; /**< Cumulative messages expiring locally before forwarding. */
  uint64_t closed_dropped; /**< Cumulative messages discarded on session cleanup/shutdown. */
  uint64_t send_failed; /**< Cumulative non-backpressure transport forwarding failures. */
  uint64_t forwarded; /**< Cumulative transport admissions, not deliveries. */
  uint64_t backpressure_requeued; /**< Cumulative retries returned to staging; one message can be counted repeatedly. */
  /* Index order: low, normal, high, critical. */
  uint64_t admitted_by_priority[4]; /**< Cumulative admissions indexed low, normal, high, critical. */
  uint64_t forwarded_by_priority[4]; /**< Cumulative transport forwards in low, normal, high, critical order. */
  uint64_t queue_delay_samples; /**< Cumulative successful-forward delay samples; admission to transport, not delivery. */
  uint64_t queue_delay_p90_us; /**< Estimated cumulative local queue-delay P90 in microseconds. */
  uint64_t queue_delay_p95_us; /**< Estimated cumulative local queue-delay P95 in microseconds. */
  uint64_t queue_delay_p99_us; /**< Estimated cumulative local queue-delay P99 in microseconds. */
  uint64_t queue_delay_max_us; /**< Largest sampled local queue delay in microseconds. */
  uint64_t tick_queue_delay_samples; /**< Same delay sample count, restricted to messages carrying a tick. */
  uint64_t tick_queue_delay_p90_us; /**< Tick-bearing messages' estimated cumulative queue-delay P90, microseconds. */
  uint64_t tick_queue_delay_p95_us; /**< Tick-bearing messages' estimated cumulative queue-delay P95, microseconds. */
  uint64_t tick_queue_delay_p99_us; /**< Tick-bearing messages' estimated cumulative queue-delay P99, microseconds. */
  uint64_t tick_queue_delay_max_us; /**< Largest sampled tick-bearing message queue delay, microseconds. */
} rnet_game_scheduled_queue_t;

/* Runtime-wide wire-v4 business data retained until readiness is observable.
 * Rejection fields are cumulative; all other usage fields are gauges. */
/** Early-data buffering during business-version negotiation, scoped to this runtime. */
typedef struct rnet_game_range_buffer {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t abi_version; /**< Output ABI identifier, written by the library. */
  uint64_t buffered_messages; /**< Current early messages retained, including ready-but-not-yet-polled messages. */
  uint64_t buffered_bytes; /**< Current early payload bytes; excludes framing/allocator overhead. */
  uint64_t peak_buffered_messages; /**< Highest retained early-message count observed. */
  uint64_t peak_buffered_bytes; /**< Highest retained early payload bytes observed. */
  uint64_t max_buffered_messages; /**< Runtime-wide cap inherited from configured event queue capacity. */
  uint64_t max_buffered_bytes; /**< Runtime-wide cap inherited from configured event-byte budget. */
  uint64_t session_admission_rejected; /**< Cumulative per-session early-data budget rejections. */
  uint64_t runtime_admission_rejected; /**< Cumulative runtime-wide early-data budget rejections. */
} rnet_game_range_buffer_t;

/* Cumulative LatestOnly telemetry after the game staging queue. A worker
 * pickup is the boundary after which TCP/KCP data can no longer be recalled. */
/** TCP/KCP pending-slot statistics; worker pickup ends replacement eligibility. */
typedef struct rnet_game_transport_latest {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t abi_version; /**< Output ABI identifier, written by the library. */
  uint64_t pending_replaced; /**< Cumulative TCP/KCP pending values replaced before I/O worker pickup. */
  uint64_t worker_pickups; /**< Cumulative slots consumed by I/O workers; no recall is possible after pickup. */
  uint64_t admission_would_block; /**< Cumulative bounded transport-capacity rejections, including retries. */
  uint64_t admission_invalid_handle; /**< Cumulative admissions referencing retired/unknown sessions. */
  uint64_t admission_invalid_state; /**< Cumulative admissions when runtime/session was not accepting work. */
  uint64_t admission_handshake_required; /**< Cumulative admissions before transport readiness. */
  uint64_t admission_not_supported; /**< Cumulative unsupported-path rejections; UDP uses ordinary forwarding. */
  uint64_t admission_message_too_large; /**< Cumulative size-limit rejections. */
  uint64_t admission_other_failures; /**< Cumulative admission failures outside the explicit categories. */
} rnet_game_transport_latest_t;

/** Owned export buffer; caller releases its token instead of freeing data. */
typedef struct rnet_game_buffer {
  uint32_t struct_size; /**< Output layout size in bytes, written by the library. */
  uint32_t abi_version; /**< Output ABI identifier, written by the library. */
  const uint8_t *data; /**< Library-owned bytes, valid until token release or runtime destruction; not NUL-terminated. */
  size_t len; /**< Number of bytes in data. */
  uint64_t token; /**< Release each nonzero token once with rnet_game_buffer_release on the owning runtime. */
} rnet_game_buffer_t;

/** Writes game defaults to required out, including layout headers; customize after this call. */
int32_t rnet_game_config_init(rnet_game_config_t *out);
/* Expands a scenario profile into existing config fields without changing wire semantics. */
/** Writes both required outputs for RNET_GAME_PROFILE_* profile: transport and 0/1 encryption.
 * Expands local defaults only; still fill address, trust and business protocol before joining. */
int32_t rnet_game_profile_defaults(uint32_t profile, uint32_t *out_transport,
                                   uint32_t *out_initial_encryption);
/** Creates a game runtime from required config and writes required out handle.
 * NULL client_security creates server-only usage. Keys/config copied; callbacks must
 * outlive destruction. Both peers must continue polling to drive game state and timers. */
int32_t rnet_game_runtime_create(const rnet_game_config_t *config,
                                 const rnet_client_security_t *client_security,
                                 rnet_runtime_t *out);
/** Binds required config and writes required out endpoint. transport is fixed; protocol
 * bounds are inclusive (min=max for exact). AUTH_REQUEST/RESUME_REQUEST require a decision. */
int32_t rnet_game_server_listen(rnet_runtime_t runtime,
                                const rnet_game_server_config_t *config,
                                rnet_endpoint_t *out);
/** Starts a game join using required config; writes outbound endpoint to required out.
 * Requires runtime client trust. Continue polling; send only after SESSION_READY. */
int32_t rnet_game_client_connect(rnet_runtime_t runtime,
                                 const rnet_game_client_config_t *config,
                                 rnet_endpoint_t *out);
/* Resume always yields new network handles, then requires another AUTH decision.
 * The server emits RESUME_REQUEST and both sides receive SESSION_RESUMED. */
/** Starts resume using required config, nonzero old_session and copied resume_ticket;
 * writes a new endpoint to required out. Ticket works only on its issuing server runtime,
 * once, before expiry, with the bound client identity. Server must authorize RESUME_REQUEST.
 * Old live session is retained until ready; SESSION_RESUMED maps old to new handle.
 * No recovery across server restart, rolling deployment or another server instance. */
int32_t rnet_game_client_resume_connect(
    rnet_runtime_t runtime, const rnet_game_client_config_t *config,
    rnet_session_t old_session, rnet_slice_t resume_ticket,
    rnet_endpoint_t *out);
/* Selection may already exist during server authorization; it is not readiness. */
/** Writes selected business version to required out, or errors if unavailable.
 * Selection may precede readiness; do not use this query as permission to send. */
int32_t rnet_game_selected_protocol_version(rnet_runtime_t runtime,
                                             rnet_session_t session, uint32_t *out);
/** Server issues a single-use ticket for a ready session and copied identity (1..64 bytes).
 * Success admits the protected send; client obtains RESUME_TICKET asynchronously.
 * Treat ticket and identity as sensitive; preserve neither in logs. */
int32_t rnet_game_issue_resume_ticket(rnet_runtime_t runtime,
                                      rnet_session_t session,
                                      rnet_slice_t identity);
/** Writes bound port of a live game endpoint to required out_port. */
int32_t rnet_game_endpoint_local_port(rnet_runtime_t runtime,
                                      rnet_endpoint_t endpoint, uint16_t *out_port);
/** Answers server AUTH_REQUEST or RESUME_REQUEST: accept must be 0 or 1.
 * Authorizing does not complete negotiation; wait for SESSION_READY/SESSION_RESUMED. */
int32_t rnet_game_auth_decide(rnet_runtime_t runtime,
                              rnet_session_t session, uint32_t accept);
/** Copies opaque payload into the Normal game queue for a ready session.
 * Continued poll forwards it; success is admission, not delivery. No msg_type/stream_id
 * is required. Empty payload is valid; limits/backpressure may reject the call. */
int32_t rnet_game_send(rnet_runtime_t runtime, rnet_session_t session,
                       rnet_slice_t payload);
/** Like rnet_game_send with required ABI-initialized options; priority must be 1..4.
 * Sequence/tick/correlation are optional metadata; expiry applies only in game staging. */
int32_t rnet_game_send_ex(rnet_runtime_t runtime, rnet_session_t session,
                          rnet_slice_t payload,
                          const rnet_game_send_options_t *options);
/* Same-session/key replacement covers game staging and TCP/KCP transport slots
 * until worker pickup. UDP replacement ends at game staging; socket writes and
 * KCP's reliable send cache cannot be recalled. */
/** Copies a replaceable snapshot for ready session/key (key zero is valid).
 * Same-key replacement is best effort, scoped to session. Continue polling to forward;
 * once the transport worker takes it, neither socket nor KCP queued data can be recalled. */
int32_t rnet_game_send_latest(rnet_runtime_t runtime, rnet_session_t session,
                              uint64_t key, rnet_slice_t payload);
/** Explicitly closes session, drops staged work and revokes its resume state; keeps listener. */
int32_t rnet_game_session_close(rnet_runtime_t runtime, rnet_session_t session);
/** Closes endpoint and owned sessions, discarding their staged work/resume state. */
int32_t rnet_game_endpoint_close(rnet_runtime_t runtime,
                                 rnet_endpoint_t endpoint);
/** Server queues key rotation; wait for SECURITY_CHANGED with REKEY operation for local commit. */
int32_t rnet_game_rekey(rnet_runtime_t runtime, rnet_session_t session);
/** Server requests encrypted=0/1; clients automatically follow after the protected barrier.
 * Disabling requires runtime plaintext opt-in; plaintext business data lacks integrity.
 * Wait for SECURITY_CHANGED to observe local commit; admission is not peer completion.
 * An already active mode may be a no-op without a new event. */
int32_t rnet_game_security_set(rnet_runtime_t runtime,
                               rnet_session_t session, uint32_t encrypted);
/** Writes a session's quality to required out; available=0 means no heartbeat sample.
 * UDP gaps and KCP retransmissions have different meanings; TCP has no loss estimate. */
int32_t rnet_game_network_quality(rnet_runtime_t runtime,
                                  rnet_session_t session,
                                  rnet_game_quality_t *out);
/** Writes this runtime's monotonic microseconds to required out; not Unix time. */
int32_t rnet_game_clock_micros(rnet_runtime_t runtime, uint64_t *out);
/** Writes approximate client-side server-minus-client offset to required out.
 * available=0 before sampling and on server sessions; not a trusted anti-cheat clock. */
int32_t rnet_game_clock_sync_snapshot(rnet_runtime_t runtime,
                                      rnet_session_t session,
                                      rnet_game_clock_sync_t *out);
/** Writes cumulative counters/gauges to required out; no reset or globally atomic snapshot. */
int32_t rnet_game_metrics_snapshot(rnet_runtime_t runtime,
                                   rnet_game_metrics_t *out);
/* Poll-driven cumulative latency logs; default 30000 ms, zero disables. */
/** Sets poll-driven cumulative game log interval_ms (default 30000); zero disables. */
int32_t rnet_game_metrics_log_interval_set(rnet_runtime_t runtime, uint64_t interval_ms);
/** Writes runtime-wide LatestOnly game queue gauges/counters to required out; forwarded is not delivered. */
int32_t rnet_game_realtime_queue_snapshot(
    rnet_runtime_t runtime, rnet_game_realtime_queue_t *out);
/** Writes priority queue gauges, cumulative delays and outcomes to required out; no reset.
 * tick_* delays cover only tick-bearing messages, not a per-poll time window. */
int32_t rnet_game_scheduled_queue_snapshot(
    rnet_runtime_t runtime, rnet_game_scheduled_queue_t *out);
/** Writes negotiation early-data queue gauges and admission-failure counters to required out. */
int32_t rnet_game_range_buffer_snapshot(rnet_runtime_t runtime,
                                        rnet_game_range_buffer_t *out);
/* TCP/KCP snapshots replaced after game staging but before I/O worker pickup. */
/** Writes cumulative TCP/KCP pending-slot replacement count to required out. */
int32_t rnet_game_transport_latest_replacements(rnet_runtime_t runtime,
                                                 uint64_t *out);
/** Writes TCP/KCP slot admissions, replacements and worker pickups to required out. */
int32_t rnet_game_transport_latest_snapshot(
    rnet_runtime_t runtime, rnet_game_transport_latest_t *out);
/* Prometheus text has no per-player labels; release its nonzero token. */
/** Exports low-cardinality Prometheus text into required out; not NUL-terminated.
 * Release its nonzero token with rnet_game_buffer_release; does not reset counters. */
int32_t rnet_game_prometheus_snapshot(rnet_runtime_t runtime,
                                      rnet_game_buffer_t *out);
/* Both nonzero event buffer tokens must be released separately. Sensitive
 * event bytes are erased by the library when their tokens are released. */
/** Drives negotiation, heartbeat, clock, queues and logs while exporting up to capacity
 * events into events; required out_count gives the exported prefix, also on failure.
 * Positive capacity requires a full array. timeout_ms bounds idle waiting, not lock/work
 * time; zero is nonblocking. capacity=0 permits NULL events and still drives maintenance
 * without returning application events. Both peers must poll regularly even when idle.
 * Ordinary staged events have separate count/byte limits inherited from the network event
 * queue. At those limits, transport draining pauses but timers continue. Use positive
 * capacity regularly: zero-capacity calls alone can block ACK processing and time out.
 * Release both nonzero tokens of every returned event separately; sensitive native
 * buffers are erased on release. Copies made by the application remain its responsibility. */
int32_t rnet_game_poll_events(rnet_runtime_t runtime, rnet_game_event_t *events,
                              size_t capacity, uint32_t timeout_ms,
                              size_t *out_count);
/** Releases one nonzero game event/export token; data becomes invalid immediately.
 * Zero/stale/already released tokens fail in that runtime; foreign runtimes can have
 * coincident token values. Do not mix owners or use transport release here. */
int32_t rnet_game_buffer_release(rnet_runtime_t runtime, uint64_t token);
/** Stops game/transport work with drain_timeout_ms send budget (zero means no drain).
 * If WOULD_BLOCK, poll and retry to publish final events. Keep handle for buffer release;
 * stop/destroy may not be called from logger callbacks. */
int32_t rnet_game_runtime_stop(rnet_runtime_t runtime,
                               uint32_t drain_timeout_ms);
/** Destroys a successfully stopped game runtime after all tokens are released.
 * Active calls yield WOULD_BLOCK; unstopped runtime/outstanding tokens yield INVALID_STATE.
 * Only success invalidates handle; serialize lifetime changes and retry after resolving errors. */
int32_t rnet_game_runtime_destroy(rnet_runtime_t runtime);

#ifdef __cplusplus
}
#endif

#endif
