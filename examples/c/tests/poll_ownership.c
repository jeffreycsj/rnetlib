/* Linux linker interception supplies failure timing; ownership is checked by the
 * real native buffer registry and destroy precondition, not a fake release count. */
#include "../game_poll.h"
#include <stdlib.h>
#include <string.h>

static int injected_status;
static unsigned handler_calls;
static unsigned fail_on_call = 1;
static int empty_prefix;
static uint64_t outside_prefix;
static char original_error[1024];

static void require(int condition, const char *message) {
  if (!condition) { fprintf(stderr, "%s\n", message); exit(1); }
}

int32_t __wrap_rnet_game_poll_events(rnet_runtime_t runtime, rnet_game_event_t *events,
                                    size_t capacity, uint32_t timeout, size_t *count) {
  (void)timeout;
  require(capacity >= 3, "fixture needs three slots");
  memset(events, 0, capacity * sizeof(*events));
  *count = 0;
  if (empty_prefix) {
    uint32_t version;
    int32_t status = rnet_game_selected_protocol_version(runtime, 0, &version);
    snprintf(original_error, sizeof(original_error), "%s", rnet_last_error_message());
    return status;
  }
  rnet_game_buffer_t buffers[4] = {{0}};
  for (size_t i = 0; i < 4; ++i)
    require(rnet_game_prometheus_snapshot(runtime, &buffers[i]) == RNET_OK, "export failed");
  events[0].buffer_token = buffers[0].token;
  events[0].aux_buffer_token = buffers[1].token;
  events[1].buffer_token = buffers[2].token;
  events[2].buffer_token = outside_prefix = buffers[3].token;
  *count = 2; /* Third slot is not exported: the fixture still owns its token. */
  if (injected_status != RNET_OK) {
    uint32_t version;
    int32_t status = rnet_game_selected_protocol_version(runtime, 0, &version);
    snprintf(original_error, sizeof(original_error), "%s", rnet_last_error_message());
    return status;
  }
  return RNET_OK;
}

static int32_t handler(void *context, const rnet_game_event_t *event) {
  (void)event;
  ++handler_calls;
  if (context != NULL && handler_calls == fail_on_call) {
    uint32_t version;
    int32_t status = rnet_game_selected_protocol_version(*(rnet_runtime_t *)context, 0, &version);
    snprintf(original_error, sizeof(original_error), "%s", rnet_last_error_message());
    return status;
  }
  return RNET_OK;
}

int main(int argc, char **argv) {
  require(argc == 2, "choose a fixture mode");
  empty_prefix = strcmp(argv[1], "empty-error") == 0;
  injected_status = strcmp(argv[1], "poll-error") == 0 || empty_prefix ? RNET_E_INVALID_HANDLE : RNET_OK;
  int handler_error = strncmp(argv[1], "handler-error", 13) == 0;
  if (strcmp(argv[1], "handler-error-last") == 0) fail_on_call = 2;
  int discard = strcmp(argv[1], "discard") == 0;
  require(rnet_abi_version() == RNET_ABI_VERSION, "ABI mismatch");
  rnet_game_config_t config;
  rnet_runtime_t runtime = 0;
  require(rnet_game_config_init(&config) == RNET_OK, "config failed");
  require(rnet_game_runtime_create(&config, NULL, &runtime) == RNET_OK, "create failed");
  demo_error error = {0};
  int32_t status = demo_poll(runtime, discard ? NULL : handler, handler_error ? &runtime : NULL, &error);
  require(status == (injected_status || handler_error ? RNET_E_INVALID_HANDLE : RNET_OK),
          "wrong original status");
  require(handler_calls == (injected_status || discard ? 0u : handler_error ? fail_on_call : 2u),
          "must not dispatch after poll or handler failure");
  if (status != RNET_OK)
    require(strcmp(error.detail, original_error) == 0, "original error overwritten by cleanup");
  if (!empty_prefix)
    require(rnet_game_buffer_release(runtime, outside_prefix) == RNET_OK,
            "released token beyond out_count");
  require(rnet_game_runtime_stop(runtime, 0) == RNET_OK, "stop failed");
  require(rnet_game_runtime_destroy(runtime) == RNET_OK,
          "exported prefix leaked main/aux tokens; destroy refused");
  return 0;
}
