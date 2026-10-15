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

/// Business-data protection policy; controls remain authenticated in both modes.
enum class SecurityMode : uint32_t {
  Plaintext = RNET_SECURITY_PLAINTEXT, ///< Unprotected business bytes; protected controls and peer handshake remain enabled.
  Encrypted = RNET_SECURITY_ENCRYPTED ///< Authenticated encrypted business bytes and protected controls.
};

/// Kind of locally completed security barrier.
enum class SecurityOperation : uint8_t {
  ModeSwitch = RNET_SECURITY_OPERATION_MODE_SWITCH, ///< Business encryption policy barrier completed locally.
  Rekey = RNET_SECURITY_OPERATION_REKEY ///< Key-generation barrier completed locally.
};

/// Local barrier-completion details decoded from SECURITY_CHANGED.
struct SecurityChange {
  SecurityOperation operation; ///< Completed mode switch or key rotation.
  SecurityMode mode; ///< Resulting business-data mode; controls remain protected.
  uint64_t epoch; ///< Session-local security generation, not wall-clock time.
};

/// Native failure with owned diagnostic text and machine-readable status.
class Error : public std::runtime_error {
 public:
  /// Copies message; an empty message falls back to a formatted numeric status.
  Error(int32_t status, const std::string &message)
      : std::runtime_error(message.empty()
                               ? "rnet status " + std::to_string(status)
                               : message),
        status_(status) {}

  /// Original native RNET_E_* value; use this rather than parsing what().
  int32_t status() const noexcept { return status_; }

 private:
  int32_t status_;
};

/// Throws Error on non-OK status, copying current thread-local native detail immediately.
inline void check(int32_t status) {
  if (status != RNET_OK) {
    const char *message = rnet_last_error_message();
    throw Error(status, message == NULL ? std::string() : std::string(message));
  }
}

// Check before any initializer writes a native layout into SDK-owned storage.
/// Throws runtime_error if this SDK's exact layout does not match the native library.
inline void check_abi() {
  if (rnet_abi_version() != RNET_ABI_VERSION)
    throw std::runtime_error("RNet SDK/native ABI mismatch; rebuild and deploy together");
}

/// Owned transport event. Data survives later polls/runtime destruction; sensitive copies are not auto-erased.
struct Event {
  uint32_t type = 0; ///< RNET_EVENT_* discriminant; determines meaningful fields.
  rnet_endpoint_t endpoint = 0; ///< Local owning endpoint; zero for runtime-wide events.
  rnet_session_t session = 0; ///< Local session; zero for runtime/endpoint events or failures before allocation.
  uint64_t correlation_id = 0; ///< MESSAGE request/trace metadata; zero when absent.
  int32_t status = RNET_OK; ///< Local RNET_OK or negative RNET_E_* category; not a peer's business error code.
  std::vector<uint8_t> data; ///< Owned payload/diagnostic copy; AUTH_REQUEST includes key[32] then join bytes. Caller erases sensitive copies.

  /// Copies authenticated client key; wrong event type or short payload throws logic_error.
  std::array<uint8_t, 32> auth_client_public_key() const {
    if (type != RNET_EVENT_AUTH_REQUEST || data.size() < 32) {
      throw std::logic_error("event is not a valid auth request");
    }
    std::array<uint8_t, 32> result;
    std::copy(data.begin(), data.begin() + 32, result.begin());
    return result;
  }

  /// Copies opaque AUTH_REQUEST join bytes; wrong type/short payload throws logic_error.
  /// Caller owns credential erasure for this new copy.
  std::vector<uint8_t> auth_join_payload() const {
    if (type != RNET_EVENT_AUTH_REQUEST || data.size() < 32) {
      throw std::logic_error("event is not a valid auth request");
    }
    return std::vector<uint8_t>(data.begin() + 32, data.end());
  }

  /// Decodes ten-byte SECURITY_CHANGED; wrong type/length or unknown operation throws logic_error.
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

/// Optional peer-visible trace metadata; no message type or transport selector.
struct SendOptions {
  uint64_t correlation_id = 0; ///< Optional request/trace metadata copied to the peer; zero means absent.
};

/// Transport-wide counters and live gauges; independent fields are not globally atomic.
struct Metrics {
  uint64_t frames_received = 0; ///< Cumulative logical frames received, not raw IP packets.
  uint64_t frames_sent = 0; ///< Cumulative logical frames accepted for transport sending, not remote acknowledgements.
  uint64_t bytes_received = 0; ///< Cumulative decoded frame body bytes received; excludes outer framing.
  uint64_t bytes_sent = 0; ///< Cumulative logical encoded frame bytes sent; includes logical framing.
  uint64_t events_dropped = 0; ///< Cumulative transport event queue admission failures.
  uint64_t send_would_block = 0; ///< Cumulative transport send backpressure rejections.
  uint64_t protocol_errors = 0; ///< Cumulative malformed/invalid transport-protocol input observations.
  uint64_t lifecycle_events_rejected = 0; ///< Cumulative failed lifecycle-event admissions; cleanup does not depend on delivery.
  uint64_t admission_rejected = 0; ///< Cumulative endpoint/session/handshake admission failures.
  uint64_t queued_send_bytes = 0; ///< Current send byte reservations; includes framing, not total process RSS.
  uint64_t peak_queued_send_bytes = 0; ///< Highest runtime send byte reservation observed.
  uint64_t queued_event_bytes = 0; ///< Current payload bytes awaiting transport poll; excludes exported buffer tokens.
  uint64_t current_endpoints = 0; ///< Current registered endpoints, including listeners and client connections.
  uint64_t current_sessions = 0; ///< Current registered sessions including unfinished handshakes.
  uint64_t established_sessions = 0; ///< Current transport-open sessions; not proof of game readiness.
  uint64_t pending_handshakes = 0; ///< Current sessions awaiting transport handshake/authorization.
  uint64_t peak_pending_handshakes = 0; ///< Highest pending-handshake count observed.
  std::array<uint64_t, 8> admission_rejected_by_reason{}; ///< Counts: endpoint, endpoint-session, pending, IP-active, IP-rate, IP-table, KCP-peer, unauthenticated.
  std::array<uint64_t, 19> session_closed_by_reason{}; ///< Cumulative closes indexed by negated error code, 0..18 (RNET_OK maps to 0).
  uint64_t logs_dropped = 0; ///< Cumulative bounded logger admission drops; zero also when no logger is installed.
  uint64_t logger_panics = 0; ///< Cumulative caught Rust logger panics; foreign exceptions must never cross the C boundary.
};

/// Histogram estimates in microseconds; zero sample_count means unavailable.
struct LatencyMetric {
  uint32_t kind = 0; ///< RNET_LATENCY_* operation sampled; units below are microseconds.
  uint64_t sample_count = 0; ///< Samples represented; zero means unavailable, not measured zero latency.
  uint64_t p50_us = 0; ///< Estimated median latency; histogram bucket approximation.
  uint64_t p90_us = 0; ///< Estimated 90th percentile latency.
  uint64_t p95_us = 0; ///< Estimated 95th percentile latency.
  uint64_t p99_us = 0; ///< Estimated 99th percentile latency.
  uint64_t p999_us = 0; ///< Estimated 99.9th percentile latency.
  uint64_t max_us = 0; ///< Largest sampled duration; concurrent snapshots are not atomic across all fields.
};

}  // namespace rnet

#endif
