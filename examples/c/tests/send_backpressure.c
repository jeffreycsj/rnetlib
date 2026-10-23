/* Test-only admission failure. The example still handshakes, polls, sends and
 * receives through the real library; each peer must retain its copied payload. */
#include "rnet.h"
#include <stdio.h>
#include <stdlib.h>

int32_t __real_rnet_game_send(rnet_runtime_t, rnet_session_t, rnet_slice_t);

int32_t __wrap_rnet_game_send(rnet_runtime_t runtime, rnet_session_t session, rnet_slice_t payload) {
  static rnet_runtime_t owners[2];
  static unsigned attempts[2];
  for (size_t i = 0; i < 2; ++i) {
    if (owners[i] == 0) owners[i] = runtime;
    if (owners[i] != runtime) continue;
    ++attempts[i];
    if (attempts[i] <= 2 || getenv("RNET_EXAMPLE_ALWAYS_BLOCK") != NULL) {
      if (attempts[i] <= 2) fprintf(stderr, "injected send backpressure peer=%zu\n", i);
      return RNET_E_WOULD_BLOCK;
    }
    return __real_rnet_game_send(runtime, session, payload);
  }
  abort(); /* Fixture contract: exactly two independent runtimes. */
}
