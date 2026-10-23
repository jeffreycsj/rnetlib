#include "../../examples/cpp/game_quickstart_support.hpp"
#include <cstdio>
#include <functional>

using game_example::Clock;
using game_example::Payload;
using game_example::PendingSends;

static void require(bool value, const char *reason) {
  if (!value) throw std::runtime_error(reason);
}

template <typename Call>
static void expect_status(int32_t expected, Call call) {
  try { call(); }
  catch (const rnet::Error &error) {
    require(error.status() == expected, "wrong error category");
    return;
  }
  throw std::runtime_error("expected native-category error");
}

static void backpressure() {
  PendingSends queue;
  Payload original{1, 0, 255};
  queue.stage(original);
  queue.stage(Payload{});
  original.assign(3, 9);
  const auto deadline = Clock::now() + std::chrono::seconds(5);
  unsigned attempts = 0;
  require(queue.flush([&](const Payload &) {
    ++attempts;
    throw rnet::Error(RNET_E_WOULD_BLOCK, "full");
  }, deadline), "backpressure should defer rather than abort");
  require(attempts == 1, "blocked flush must not busy-loop");
  std::vector<Payload> received;
  require(!queue.flush([&](const Payload &p) { received.push_back(p); }, deadline), "retry failed");
  require(received == std::vector<Payload>{{1, 0, 255}, {}}, "copied binary/empty payload lost");
}

static void expired() {
  PendingSends queue;
  queue.stage(Payload{7});
  unsigned calls = 0;
  expect_status(RNET_E_TIMEOUT, [&] {
    queue.flush([&](const Payload &) { ++calls; }, Clock::now());
  });
  require(calls == 0, "expired retry called send");
  std::vector<Payload> received;
  queue.flush([&](const Payload &p) { received.push_back(p); }, Clock::now() + std::chrono::seconds(5));
  require(received == std::vector<Payload>{{7}}, "deadline discarded application-owned work");
}

static void terminal() {
  PendingSends queue;
  queue.stage(Payload{8});
  auto deadline = Clock::now() + std::chrono::seconds(5);
  expect_status(RNET_E_INVALID_HANDLE, [&] {
    queue.flush([](const Payload &) { throw rnet::Error(RNET_E_INVALID_HANDLE, "retired"); }, deadline);
  });
  std::vector<Payload> received;
  queue.flush([&](const Payload &p) { received.push_back(p); }, deadline);
  require(received == std::vector<Payload>{{8}}, "terminal failure silently removed work");
}

static void limits() {
  PendingSends queue;
  expect_status(RNET_E_MESSAGE_TOO_LARGE, [&] { queue.stage(Payload(129, 1)); });
  queue.stage(Payload(128, 1));
  queue.stage(Payload{});
  expect_status(RNET_E_WOULD_BLOCK, [&] { queue.stage(Payload{3}); });
  std::vector<Payload> received;
  queue.flush([&](const Payload &p) { received.push_back(p); }, Clock::now() + std::chrono::seconds(5));
  require(received == std::vector<Payload>{Payload(128, 1), {}}, "rejected enqueue evicted older work");
}

static void partial_admission() {
  PendingSends queue;
  queue.stage(Payload{1});
  queue.stage(Payload{});
  std::vector<Payload> accepted;
  const auto deadline = Clock::now() + std::chrono::seconds(5);
  require(queue.flush([&](const Payload &p) {
    if (!accepted.empty()) throw rnet::Error(RNET_E_WOULD_BLOCK, "second blocked");
    accepted.push_back(p);
  }, deadline), "second admission must block");
  queue.flush([&](const Payload &p) { accepted.push_back(p); }, deadline);
  require(accepted == std::vector<Payload>{{1}, {}}, "retry duplicated accepted prefix");
}

static void allocation_exception() {
  PendingSends queue;
  queue.stage(Payload{7});
  const auto deadline = Clock::now() + std::chrono::seconds(5);
  bool caught = false;
  try { queue.flush([](const Payload &) { throw std::bad_alloc(); }, deadline); }
  catch (const std::bad_alloc &) { caught = true; }
  require(caught, "non-native exception swallowed");
  std::vector<Payload> received;
  queue.flush([&](const Payload &p) { received.push_back(p); }, deadline);
  require(received == std::vector<Payload>{{7}}, "allocation failure discarded retry");
}

static void cleanup() {
  std::exception_ptr primary;
  try { throw rnet::Error(RNET_E_AUTH_REJECTED, "original authorization error"); }
  catch (...) { primary = std::current_exception(); }
  unsigned closed = 0;
  try {
    game_example::finish(primary, [&] { ++closed; throw std::bad_alloc(); },
                                  [&] { ++closed; throw rnet::Error(RNET_E_WOULD_BLOCK, "cleanup"); });
    require(false, "primary failure hidden");
  } catch (const rnet::Error &error) {
    require(error.status() == RNET_E_AUTH_REJECTED &&
            std::string(error.what()) == "original authorization error", "primary exception replaced");
  }
  require(closed == 2, "failed client cleanup skipped server");
  closed = 0;
  expect_status(RNET_E_IO_ERROR, [&] {
    game_example::finish(std::exception_ptr(), [&] { ++closed; throw rnet::Error(RNET_E_IO_ERROR, "cleanup"); },
                                              [&] { ++closed; });
  });
  require(closed == 2, "cleanup error became success or skipped second owner");
  game_example::finish(std::exception_ptr(), [&] { ++closed; }, [&] { ++closed; });
  require(closed == 4, "successful cleanup incomplete");
}

int main() {
  unsigned failures = 0;
  for (const auto &test : std::vector<std::pair<const char *, std::function<void()> > >{
         {"backpressure", backpressure}, {"expired", expired}, {"terminal", terminal}, {"limits", limits},
         {"partial_admission", partial_admission}, {"allocation_exception", allocation_exception}, {"cleanup", cleanup}}) {
    try { test.second(); }
    catch (const std::exception &error) {
      std::fprintf(stderr, "%s: %s\n", test.first, error.what());
      ++failures;
    }
  }
  return failures ? 1 : 0;
}
