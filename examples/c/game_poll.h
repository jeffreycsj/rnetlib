/* Example-only event ownership helper, not part of the public SDK. */
#ifndef RNET_EXAMPLE_GAME_POLL_H
#define RNET_EXAMPLE_GAME_POLL_H

#include "rnet.h"
#include <stdio.h>

/* Handlers borrow event bytes until return. Copy anything retained for a retry. */
typedef int32_t (*demo_event_handler)(void *, const rnet_game_event_t *);

typedef struct demo_error {
  int32_t code;
  char detail[1024]; /* Copy before release/cleanup overwrites thread-local diagnostics. */
} demo_error;

static inline void demo_capture_error(demo_error *error, int32_t code) {
  if (error->code == RNET_OK && code != RNET_OK) {
    error->code = code;
    snprintf(error->detail, sizeof(error->detail), "%s", rnet_last_error_message());
  }
}

/* Single consumer; handler may be NULL to discard events during shutdown.
 * error is caller-owned, initially zero, and latches the first failure. Returning
 * nonzero suppresses further handlers but never skips exported-token cleanup. */
static inline int32_t demo_poll(rnet_runtime_t runtime, demo_event_handler handler,
                                void *context, demo_error *error) {
  rnet_game_event_t events[32];
  size_t count = 0;
  int32_t status = rnet_game_poll_events(runtime, events, 32, 1, &count);
  demo_capture_error(error, status);
  /* out_count owns a prefix even on failure. Never inspect the unreported tail;
   * suppress further business dispatch after an error, but release the whole prefix. */
  for (size_t i = 0; i < count; ++i) {
    if (error->code == RNET_OK && handler != NULL) {
      status = handler(context, &events[i]);
      demo_capture_error(error, status);
    }
    if (events[i].buffer_token != 0)
      demo_capture_error(error, rnet_game_buffer_release(runtime, events[i].buffer_token));
    if (events[i].aux_buffer_token != 0)
      demo_capture_error(error, rnet_game_buffer_release(runtime, events[i].aux_buffer_token));
  }
  return error->code;
}
#endif
