#include "rnet.h"
#include <assert.h>

/* Exercise the generated declarations and size_t output using an actual C caller. */
int main(void) {
  assert(rnet_abi_version() == RNET_ABI_VERSION);
  size_t forwarded = SIZE_MAX;
  assert(rnet_game_flush_realtime(0, 0, &forwarded) == RNET_E_INVALID_HANDLE);
  assert(forwarded == 0);
  assert(rnet_game_flush_realtime(0, 0, NULL) == RNET_E_INVALID_ARGUMENT);
  rnet_game_config_t config;
  assert(rnet_game_config_init(&config) == RNET_OK);
  uint64_t runtime = 0;
  assert(rnet_game_runtime_create(&config, NULL, &runtime) == RNET_OK);
  assert(rnet_game_flush_realtime(runtime, SIZE_MAX, &forwarded) == RNET_OK);
  assert(forwarded == 0);
  const rnet_slice_t empty = {NULL, 0};
  assert(rnet_game_send_latest_with_tick(runtime, 0, 0, empty, UINT32_MAX) == RNET_E_INVALID_HANDLE);
  assert(rnet_game_runtime_stop(runtime, 0) == RNET_OK);
  assert(rnet_game_flush_realtime(runtime, 0, &forwarded) == RNET_OK);
  assert(forwarded == 0);
  assert(rnet_game_runtime_destroy(runtime) == RNET_OK);
  forwarded = SIZE_MAX;
  assert(rnet_game_flush_realtime(runtime, 0, &forwarded) == RNET_E_INVALID_HANDLE);
  assert(forwarded == 0);
  return 0;
}
