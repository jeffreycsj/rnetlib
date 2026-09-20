#ifndef RNET_TYPES_HPP
#define RNET_TYPES_HPP

#include "rnet.h"

#include <algorithm>
#include <array>
#include <cstdint>
#include <cstddef>
#include <stdexcept>
#include <string>
#include <vector>

namespace rnet {

enum class SecurityMode : uint32_t {
  Plaintext = RNET_SECURITY_PLAINTEXT,
  Encrypted = RNET_SECURITY_ENCRYPTED
};

enum class SecurityOperation : uint8_t {
  ModeSwitch = RNET_SECURITY_OPERATION_MODE_SWITCH,
  Rekey = RNET_SECURITY_OPERATION_REKEY
};

struct SecurityChange {
  SecurityOperation operation;
  SecurityMode mode;
  uint64_t epoch;
};

class Error : public std::runtime_error {
 public:
  Error(int32_t status, const std::string &message)
      : std::runtime_error(message.empty()
                               ? "rnet status " + std::to_string(status)
                               : message),
        status_(status) {}

  int32_t status() const noexcept { return status_; }

 private:
  int32_t status_;
};

inline void check(int32_t status) {
  if (status != RNET_OK) {
    const char *message = rnet_last_error_message();
    throw Error(status, message == NULL ? std::string() : std::string(message));
  }
}

struct Event {
  uint32_t type = 0;
  rnet_endpoint_t endpoint = 0;
  rnet_session_t session = 0;
  uint32_t msg_type = 0;
  uint32_t stream_id = 0;
  uint64_t request_id = 0;
  int32_t status = RNET_OK;
  std::vector<uint8_t> data;

  std::array<uint8_t, 32> auth_client_public_key() const {
    if (type != RNET_EVENT_AUTH_REQUEST || data.size() < 32) {
      throw std::logic_error("event is not a valid auth request");
    }
    std::array<uint8_t, 32> result;
    std::copy(data.begin(), data.begin() + 32, result.begin());
    return result;
  }

  std::vector<uint8_t> auth_join_payload() const {
    if (type != RNET_EVENT_AUTH_REQUEST || data.size() < 32) {
      throw std::logic_error("event is not a valid auth request");
    }
    return std::vector<uint8_t>(data.begin() + 32, data.end());
  }

  SecurityChange security_change() const {
    if (type != RNET_EVENT_SECURITY_CHANGED || data.size() != 10) {
      throw std::logic_error("event is not a valid security change");
    }
    uint64_t epoch = 0;
    for (size_t index = 1; index < 9; ++index) {
      epoch = (epoch << 8) | data[index];
    }
    if (data[9] != RNET_SECURITY_OPERATION_MODE_SWITCH &&
        data[9] != RNET_SECURITY_OPERATION_REKEY) {
      throw std::logic_error("security event has an unknown operation");
    }
    return SecurityChange{static_cast<SecurityOperation>(data[9]),
                          static_cast<SecurityMode>(data[0]), epoch};
  }
};

struct SendOptions {
  uint64_t correlation_id = 0;
};

struct Metrics {
  uint64_t frames_received = 0;
  uint64_t frames_sent = 0;
  uint64_t bytes_received = 0;
  uint64_t bytes_sent = 0;
  uint64_t events_dropped = 0;
  uint64_t send_would_block = 0;
  uint64_t protocol_errors = 0;
  uint64_t lifecycle_events_rejected = 0;
  uint64_t admission_rejected = 0;
  uint64_t queued_send_bytes = 0;
  uint64_t peak_queued_send_bytes = 0;
  uint64_t queued_event_bytes = 0;
  uint64_t current_endpoints = 0;
  uint64_t current_sessions = 0;
  uint64_t established_sessions = 0;
  uint64_t pending_handshakes = 0;
  uint64_t peak_pending_handshakes = 0;
  std::array<uint64_t, 8> admission_rejected_by_reason{};
  std::array<uint64_t, 19> session_closed_by_reason{};
  uint64_t logs_dropped = 0;
  uint64_t logger_panics = 0;
};

struct LatencyMetric {
  uint32_t kind = 0;
  uint64_t sample_count = 0;
  uint64_t p50_us = 0;
  uint64_t p90_us = 0;
  uint64_t p95_us = 0;
  uint64_t p99_us = 0;
  uint64_t p999_us = 0;
  uint64_t max_us = 0;
};

}  // namespace rnet

#endif
