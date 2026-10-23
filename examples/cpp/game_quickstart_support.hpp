#ifndef RNET_EXAMPLE_GAME_QUICKSTART_SUPPORT_HPP
#define RNET_EXAMPLE_GAME_QUICKSTART_SUPPORT_HPP

#include "rnet.hpp"
#include <chrono>
#include <deque>
#include <cstdio>
#include <exception>

// Example-only application policy, not a new SDK surface or a multiplayer queue.
namespace game_example {
using Clock = std::chrono::steady_clock;
using Payload = std::vector<uint8_t>;

class PendingSends {
 public:
  // Copies borrowed data; both message count and per-message bytes are bounded.
  void stage(const Payload &payload) {
    if (payload.size() > 128)
      throw rnet::Error(RNET_E_MESSAGE_TOO_LARGE, "demo payload limit is 128 bytes");
    if (pending_.size() >= 2)
      throw rnet::Error(RNET_E_WOULD_BLOCK, "demo application retry queue is full");
    pending_.push_back(payload);
  }

  // true defers this peer until after poll. Only WouldBlock is retryable here;
  // native terminal errors and allocation failures propagate unchanged.
  // send borrows one payload for the call; it must not reenter/mutate this queue.
  template <typename Send>
  bool flush(Send send, Clock::time_point deadline) {
    while (!pending_.empty()) {
      if (Clock::now() >= deadline)
        throw rnet::Error(RNET_E_TIMEOUT, "demo retry deadline exceeded");
      try { send(pending_.front()); }
      catch (const rnet::Error &error) {
        if (error.status() == RNET_E_WOULD_BLOCK) return true;
        throw;
      }
      pending_.pop_front();
    }
    return false;
  }

 private:
  std::deque<Payload> pending_; // Empty payloads still occupy one queue entry.
};

inline void report_error(const char *phase, std::exception_ptr error) noexcept {
  if (!error) return;
  try { std::rethrow_exception(error); }
  catch (const rnet::Error &e) {
    std::fprintf(stderr, "%s code=%d %s\n", phase, static_cast<int>(e.status()), e.what());
  } catch (const std::exception &e) {
    std::fprintf(stderr, "%s %s\n", phase, e.what());
  } catch (...) {
    std::fprintf(stderr, "%s unknown exception\n", phase);
  }
}

// Both owners are cleaned even if one fails. Retain the original exception type
// and detail; report secondary cleanup failures without replacing the root cause.
template <typename CloseClient, typename CloseServer>
void finish(std::exception_ptr primary, CloseClient close_client, CloseServer close_server) {
  try { close_client(); }
  catch (...) {
    const auto error = std::current_exception();
    report_error("client cleanup", error);
    if (!primary) primary = error;
  }
  try { close_server(); }
  catch (...) {
    const auto error = std::current_exception();
    report_error("server cleanup", error);
    if (!primary) primary = error;
  }
  if (primary) std::rethrow_exception(primary);
}
} // namespace game_example
#endif
