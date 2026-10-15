// Exercise the real wrappers against a controlled C ownership boundary. The
// normal smoke tests cover native networking; here allocation failures must be
// deterministic, without exhausting memory or relying on packet timing.
#include "rnet.hpp"

#include <cstdio>
#include <cstdlib>
#include <new>

namespace {
int fail_after = -1;
int arm_after_export = -1;
bool export_error = false;
unsigned releases[4] = {};
unsigned released_before_throw = 0;
bool invalid_release = false;
int failures = 0;
const char *last_error = "";
const uint8_t payload[] = {0, 42, 255};

void expect(bool condition, const char *message) {
  if (!condition) {
    std::fprintf(stderr, "%s\n", message);
    ++failures;
  }
}

int32_t release(uint64_t token) {
  if (token == 0 || token > 3) {
    invalid_release = true;
    return RNET_E_INVALID_HANDLE;
  }
  ++releases[token];
  // Successful FFI calls overwrite thread-local detail. Wrappers must capture
  // the original failure before their cleanup calls clear it.
  last_error = "";
  return RNET_OK;
}
}  // namespace

void *operator new(std::size_t size) {
  if (fail_after >= 0 && fail_after-- == 0) {
    fail_after = -1;
    released_before_throw = releases[1] + releases[2] + releases[3];
    throw std::bad_alloc();
  }
  if (void *ptr = std::malloc(size == 0 ? 1 : size)) return ptr;
  throw std::bad_alloc();
}
void operator delete(void *ptr) noexcept { std::free(ptr); }
void *operator new[](std::size_t size) { return ::operator new(size); }
void operator delete[](void *ptr) noexcept { ::operator delete(ptr); }

extern "C" {
uint32_t rnet_abi_version() { return RNET_ABI_VERSION; }
int32_t rnet_config_init(rnet_config_t *) { return RNET_OK; }
int32_t rnet_game_config_init(rnet_game_config_t *) { return RNET_OK; }
int32_t rnet_runtime_create(const rnet_config_t *,
                            const rnet_client_security_t *, rnet_runtime_t *out) {
  *out = 1;
  return RNET_OK;
}
int32_t rnet_game_runtime_create(const rnet_game_config_t *,
                                 const rnet_client_security_t *, rnet_runtime_t *out) {
  *out = 2;
  return RNET_OK;
}
int32_t rnet_runtime_stop(rnet_runtime_t, uint32_t) { return RNET_OK; }
int32_t rnet_runtime_destroy(rnet_runtime_t) { return RNET_OK; }
int32_t rnet_game_runtime_stop(rnet_runtime_t, uint32_t) { return RNET_OK; }
int32_t rnet_game_runtime_destroy(rnet_runtime_t) { return RNET_OK; }
const char *rnet_last_error_message() { return last_error; }
int32_t rnet_buffer_release(rnet_runtime_t runtime, uint64_t token) {
  expect(runtime == 1, "transport token released on wrong runtime");
  return release(token);
}
int32_t rnet_game_buffer_release(rnet_runtime_t runtime, uint64_t token) {
  expect(runtime == 2, "game token released on wrong runtime");
  return release(token);
}
int32_t rnet_poll_events(rnet_runtime_t, rnet_event_t *events, size_t capacity,
                         uint32_t, size_t *count) {
  *count = 0;
  if (capacity == 0) return RNET_OK;
  expect(capacity >= 3, "test requires three output slots");
  for (size_t i = 0; i < 3; ++i) {
    events[i] = {};
    events[i].event_type = i == 2 ? RNET_EVENT_WRITABLE : RNET_EVENT_MESSAGE;
    if (i != 2) {
      events[i].data = payload;
      events[i].data_len = sizeof(payload);
      events[i].buffer_token = i + 1;
    }
  }
  *count = 3;
  fail_after = arm_after_export;
  return RNET_OK;
}
int32_t rnet_game_poll_events(rnet_runtime_t, rnet_game_event_t *events,
                              size_t capacity, uint32_t, size_t *count) {
  *count = 0;
  if (capacity == 0) return RNET_OK;
  expect(capacity >= 3, "test requires three output slots");
  for (size_t i = 0; i < 3; ++i) {
    events[i] = {};
    events[i].event_type = i == 0 ? RNET_GAME_RESUME_REQUEST : RNET_GAME_MESSAGE;
    if (i != 2) {
      events[i].data = payload;
      events[i].data_len = sizeof(payload);
      events[i].buffer_token = i + 1;
    }
  }
  events[0].aux_data = payload;
  events[0].aux_data_len = sizeof(payload);
  events[0].aux_buffer_token = 3;
  // An error can follow a fully exported prefix; unused array slots are not owned.
  *count = export_error ? 1 : 3;
  if (export_error) {
    last_error = "injected export failure";
    return RNET_E_INTERNAL_PANIC;
  }
  fail_after = arm_after_export;
  return RNET_OK;
}
}  // extern "C"

template <typename Runtime>
void check_allocation_failures(Runtime &runtime, bool game) {
  const int allocations = game ? 4 : 3;  // reserve plus each owned payload copy
  for (int index = 0; index <= allocations; ++index) {
    std::fill(releases, releases + 4, 0);
    arm_after_export = index;
    bool threw = false;
    try {
      const auto events = runtime.poll(3, 0);
      fail_after = -1;
      expect(events.size() == 3 && events[0].data.size() == sizeof(payload) &&
                 events[0].data[2] == 255 && events[2].data.empty(),
             "successful poll must return owned binary payloads and empty events");
    } catch (const std::bad_alloc &) {
      fail_after = -1;
      threw = true;
    }
    expect(threw == (index < allocations), "allocation injection missed expected copy");
    if (!game && index == 2)
      expect(released_before_throw == 1,
             "transport must release copied payload before allocating the next copy");
    expect(releases[1] == 1 && releases[2] == 1 && releases[3] == (game ? 1U : 0U),
           game ? "game poll leaked/double-released a batch token"
                : "transport poll leaked/double-released a batch token");
  }
  arm_after_export = -1;
  std::fill(releases, releases + 4, 0);
  expect(runtime.poll(0, 0).empty(), "zero-capacity poll must be empty");
  expect(releases[1] + releases[2] + releases[3] == 0, "empty poll must not release tokens");
}

int main() {
  rnet::Runtime transport;
  rnet::GameRuntime game;
  check_allocation_failures(transport, false);
  check_allocation_failures(game, true);
  export_error = true;
  bool caught = false;
  try {
    (void)game.poll(3, 0);
  } catch (const rnet::Error &error) {
    caught = error.status() == RNET_E_INTERNAL_PANIC &&
             std::string(error.what()) == "injected export failure";
  }
  expect(caught, "partial export must preserve original status and diagnostic");
  expect(releases[1] == 1 && releases[2] == 0 && releases[3] == 1,
         "partial game export must release only out_count prefix and both tokens");
  expect(!invalid_release, "zero or invalid buffer token released");
  return failures == 0 ? EXIT_SUCCESS : EXIT_FAILURE;
}
