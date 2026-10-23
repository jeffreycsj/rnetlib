/* Linux/POSIX C11 game quickstart. Two independent runtimes model two processes.
 * Demo credentials/ephemeral keys are NOT a production login or trust service. */
#define _POSIX_C_SOURCE 200809L
#include "game_poll.h"
#include <inttypes.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

typedef struct peer {
  rnet_runtime_t runtime;
  rnet_endpoint_t endpoint;
  rnet_session_t session;
  int server;
  int deny;
  unsigned received;
  int pending; /* Separate from length: an empty application message is valid. */
  size_t length;
  uint8_t bytes[128]; /* One in-flight request; bounded copied retry storage. */
  demo_error error;
} peer;

static const uint8_t ticket[] = "demo-only";
static const uint8_t message[] = {'h', 'e', 'l', 'l', 'o', 0, 'g', 'a', 'm', 'e'};

static uint64_t monotonic_ms(void) {
  struct timespec now;
  if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) {
    perror("clock_gettime");
    exit(EXIT_FAILURE);
  }
  return (uint64_t)now.tv_sec * 1000 + (uint64_t)now.tv_nsec / 1000000;
}

/* Volatile writes erase this example's key copies; library-owned copies have
 * their own lifecycle. Production loads persistent server identity securely. */
static void erase_key(rnet_keypair_t *key) {
  volatile uint8_t *bytes = key->private_key;
  for (size_t i = 0; i < sizeof(key->private_key); ++i) bytes[i] = 0;
}

static int32_t fail(peer *p, int32_t code, const char *reason) {
  if (p->error.code == RNET_OK) {
    p->error.code = code;
    snprintf(p->error.detail, sizeof(p->error.detail), "%s", reason);
  }
  return code;
}

/* Logging uses only library diagnostics, never credentials or application bytes.
 * stderr is convenient for this short demo; deployments should use a fast sink.
 * No runtime stop/destroy or borrowed pointer retention inside this callback. */
static void log_record(void *context, uint64_t timestamp, uint32_t level,
                       const uint8_t *name, size_t name_len, rnet_runtime_t runtime,
                       rnet_endpoint_t endpoint, rnet_session_t session, uint32_t transport,
                       int32_t code, uint64_t correlation, const uint8_t *detail, size_t detail_len) {
  (void)context; (void)level; (void)transport; (void)correlation;
  fprintf(stderr, "time=%" PRIu64 " runtime=%" PRIu64 " endpoint=%" PRIu64
          " session=%" PRIu64 " code=%" PRId32 " %.*s %.*s\n",
          timestamp, runtime, endpoint, session, code,
          (int)(name_len > 128 ? 128 : name_len), name ? (const char *)name : "",
          (int)(detail_len > 1024 ? 1024 : detail_len), detail ? (const char *)detail : "");
}

/* Copy before poll releases native tokens. This example intentionally supports
 * one outstanding request, not an unbounded application send queue. */
static int32_t stage(peer *p, const uint8_t *data, size_t length) {
  if (p->pending || length > sizeof(p->bytes))
    return fail(p, RNET_E_WOULD_BLOCK, "example application retry slot is full");
  if (length != 0) memcpy(p->bytes, data, length);
  p->length = length;
  p->pending = 1;
  return RNET_OK;
}

static int32_t flush(peer *p) {
  if (!p->pending) return RNET_OK;
  int32_t status = rnet_game_send(p->runtime, p->session,
                                 (rnet_slice_t){p->bytes, p->length});
  /* One attempt per iteration, followed by positive-capacity polls on both peers.
   * Do not wait only for Writable: game-stage admission may recover during poll. */
  if (status == RNET_E_WOULD_BLOCK) return RNET_OK;
  demo_capture_error(&p->error, status);
  if (status == RNET_OK) p->pending = 0;
  return status;
}

static int32_t on_event(void *context, const rnet_game_event_t *event) {
  peer *p = context;
  switch (event->event_type) {
  case RNET_GAME_AUTH_REQUEST:
    if (!p->server) return fail(p, RNET_E_PROTOCOL_ERROR, "unexpected client authorization request");
    /* Replace with ticket signature/expiry/audience and authenticated-key binding
     * checks. Never unconditionally authorize real clients based on this demo. */
    return rnet_game_auth_decide(p->runtime, event->session,
        !p->deny && event->data_len == sizeof(ticket) - 1 &&
        memcmp(event->data, ticket, sizeof(ticket) - 1) == 0);
  case RNET_GAME_SESSION_READY:
    if (event->endpoint != p->endpoint || p->session != 0)
      return fail(p, RNET_E_PROTOCOL_ERROR, "unexpected ready route");
    p->session = event->session; /* connect/listen returned endpoints, NOT sessions. */
    return p->server ? RNET_OK : stage(p, message, sizeof(message));
  case RNET_GAME_MESSAGE:
    if (event->session != p->session || p->session == 0)
      return fail(p, RNET_E_PROTOCOL_ERROR, "message before game readiness");
    if (p->server) return stage(p, event->data, event->data_len);
    if (p->received >= 2 || event->data_len != (p->received == 0 ? sizeof(message) : 0) ||
        (p->received == 0 && memcmp(event->data, message, sizeof(message)) != 0))
      return fail(p, RNET_E_PROTOCOL_ERROR, "echo payload mismatch");
    ++p->received;
    return p->received == 1 ? stage(p, NULL, 0) : RNET_OK;
  case RNET_GAME_SESSION_CLOSED:
  case RNET_GAME_JOIN_FAILED:
  case RNET_GAME_ENDPOINT_ERROR:
    return fail(p, event->status ? event->status : RNET_E_INVALID_STATE,
                "connection failed before echo completed; inspect lifecycle log");
  case RNET_GAME_PROTOCOL_VIOLATION:
    return fail(p, RNET_E_PROTOCOL_ERROR, "game protocol rejected");
  default:
    return RNET_OK;
  }
}

/* Serial lifecycle: no in-flight calls or callback-owned resources are destroyed.
 * Stop can require event space, so drain/retry with a deadline. It does not promise
 * peer delivery; success is established above by an application echo. */
static int shutdown_peer(peer *p) {
  if (p->runtime == 0) return 1;
  demo_error error = {0};
  const uint64_t deadline = monotonic_ms() + 5000;
  for (;;) {
    int32_t status = rnet_game_runtime_stop(p->runtime, 0);
    if (status == RNET_OK) break;
    if (status != RNET_E_WOULD_BLOCK) {
      demo_capture_error(&error, status);
      break;
    }
    if (monotonic_ms() >= deadline) {
      error.code = RNET_E_TIMEOUT;
      snprintf(error.detail, sizeof(error.detail), "stop deadline exceeded");
      break;
    }
    if (demo_poll(p->runtime, NULL, NULL, &error) != RNET_OK) break;
  }
  if (error.code == RNET_OK)
    demo_capture_error(&error, rnet_game_runtime_destroy(p->runtime));
  if (error.code != RNET_OK) {
    fprintf(stderr, "shutdown code=%" PRId32 " %s\n", error.code, error.detail);
    return 0; /* Do not pretend an unsuccessful destroy invalidated the handle. */
  }
  p->runtime = 0;
  return 1;
}

int main(int argc, char **argv) {
  uint32_t transport = RNET_TRANSPORT_TCP;
  if (argc < 2 || argc > 3 ||
      (strcmp(argv[1], "tcp") && strcmp(argv[1], "udp") && strcmp(argv[1], "kcp")) ||
      (argc == 3 && strcmp(argv[2], "--deny"))) {
    fprintf(stderr, "usage: %s tcp|udp|kcp [--deny]\n", argv[0]);
    return 2;
  }
  if (!strcmp(argv[1], "udp")) transport = RNET_TRANSPORT_UDP;
  if (!strcmp(argv[1], "kcp")) transport = RNET_TRANSPORT_KCP;
  if (rnet_abi_version() != RNET_ABI_VERSION) {
    fprintf(stderr, "SDK/native ABI mismatch\n");
    return 1;
  }
  peer server = {0}, client = {0};
  server.server = 1;
  server.deny = argc == 3;
  rnet_keypair_t server_key = {0}, client_key = {0};
  demo_error setup_error = {0};
  int result = 1;
  /* Every setup error converges on cleanup, including partially created peers. */
#define CHECK(call) do { int32_t code_ = (call); \
  if (code_ != RNET_OK) { demo_capture_error(&setup_error, code_); goto cleanup; } } while (0)
  CHECK(rnet_keypair_generate(&server_key));
  CHECK(rnet_keypair_generate(&client_key));
  rnet_logger_t logger = {0};
  logger.struct_size = sizeof(logger);
  logger.abi_version = RNET_ABI_VERSION;
  logger.log = log_record;
  logger.min_level = RNET_LOG_INFO;
  rnet_game_config_t config;
  CHECK(rnet_game_config_init(&config));
  config.logger = &logger;
  CHECK(rnet_game_runtime_create(&config, NULL, &server.runtime));
  rnet_client_security_t trust = {0};
  trust.struct_size = sizeof(trust);
  trust.abi_version = RNET_ABI_VERSION;
  trust.local_private_key = (rnet_slice_t){client_key.private_key, 32};
  /* The example hands the pin across locally. A real client gets it from a trusted
   * distribution channel, never from the unauthenticated remote socket. */
  trust.expected_server_public_key = (rnet_slice_t){server_key.public_key, 32};
  CHECK(rnet_game_runtime_create(&config, &trust, &client.runtime));
  CHECK(rnet_game_metrics_log_interval_set(server.runtime, 30000));
  rnet_game_server_config_t listener = {0};
  listener.struct_size = sizeof(listener);
  listener.abi_version = RNET_ABI_VERSION;
  listener.transport = transport;
  listener.initial_encryption = 1;
  listener.bind_host = (rnet_slice_t){(const uint8_t *)"127.0.0.1", 9};
  listener.local_private_key = (rnet_slice_t){server_key.private_key, 32};
  listener.protocol_id = 42;
  listener.min_version = listener.max_version = 1;
  CHECK(rnet_game_server_listen(server.runtime, &listener, &server.endpoint));
  erase_key(&server_key);
  erase_key(&client_key); /* Native create/listen copied both private keys. */
  rnet_game_client_config_t connection = {0};
  connection.struct_size = sizeof(connection);
  connection.abi_version = RNET_ABI_VERSION;
  connection.transport = transport;
  connection.remote_host = (rnet_slice_t){(const uint8_t *)"localhost", 9};
  CHECK(rnet_game_endpoint_local_port(server.runtime, server.endpoint, &connection.remote_port));
  connection.protocol_id = 42;
  connection.min_version = connection.max_version = 1;
  connection.join_ticket = (rnet_slice_t){ticket, sizeof(ticket) - 1};
  CHECK(rnet_game_client_connect(client.runtime, &connection, &client.endpoint));
  const uint64_t deadline = monotonic_ms() + 10000;
  while (client.received < 2 && monotonic_ms() < deadline) {
    if (demo_poll(server.runtime, on_event, &server, &server.error) != RNET_OK ||
        demo_poll(client.runtime, on_event, &client, &client.error) != RNET_OK ||
        flush(&server) != RNET_OK || flush(&client) != RNET_OK) goto cleanup;
  }
  if (client.received != 2) {
    fail(&client, RNET_E_TIMEOUT, "echo deadline exceeded (UDP does not guarantee delivery)");
    goto cleanup;
  }
  /* Explicitly end the local session, then the shared listener. Do not close the
   * outgoing endpoint twice: TCP may already reclaim it after session close. */
  CHECK(rnet_game_session_close(client.runtime, client.session));
  CHECK(rnet_game_endpoint_close(server.runtime, server.endpoint));
  result = 0;
cleanup:
  erase_key(&server_key);
  erase_key(&client_key);
  if (setup_error.code) fprintf(stderr, "setup/close code=%" PRId32 " %s\n", setup_error.code, setup_error.detail);
  if (server.error.code) fprintf(stderr, "server code=%" PRId32 " %s\n", server.error.code, server.error.detail);
  if (client.error.code) fprintf(stderr, "client code=%" PRId32 " %s\n", client.error.code, client.error.detail);
  if (!shutdown_peer(&client)) result = 1;
  if (!shutdown_peer(&server)) result = 1;
  if (result == 0) printf("%s game echo complete: binary and empty payloads; runtimes destroyed\n", argv[1]);
  return result;
#undef CHECK
}
