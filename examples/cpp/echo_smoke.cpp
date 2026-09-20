#include "rnet.hpp"

#include <chrono>
#include <cstdint>
#include <stdexcept>
#include <string>

int main() {
  if (rnet_abi_version() != RNET_ABI_VERSION) {
    return 1;
  }
  rnet_config_v3_t config{};
  rnet::check(rnet_config_v3_init(&config));
  config.allow_legacy_unauthenticated_endpoints = 1;
  rnet::Runtime runtime(config);
  const auto listener = runtime.open_tcp_listener("127.0.0.1", 0);
  const auto client =
      runtime.open_tcp_client("127.0.0.1", runtime.local_port(listener));

  rnet_session_t client_session = 0;
  rnet_session_t server_session = 0;
  const auto deadline = std::chrono::steady_clock::now() +
                        std::chrono::seconds(2);
  while (std::chrono::steady_clock::now() < deadline &&
         (client_session == 0 || server_session == 0)) {
    for (const auto &event : runtime.poll(16, 50)) {
      if (event.type != RNET_EVENT_SESSION_OPENED) {
        continue;
      }
      if (event.endpoint == client) {
        client_session = event.session;
      } else if (event.endpoint == listener) {
        server_session = event.session;
      }
    }
  }
  if (client_session == 0 || server_session == 0) {
    return 2;
  }

  runtime.send(client_session, 17, "cpp-echo");
  while (std::chrono::steady_clock::now() < deadline) {
    for (const auto &event : runtime.poll(16, 50)) {
      if (event.type == RNET_EVENT_MESSAGE && event.session == server_session) {
        return std::string(reinterpret_cast<const char *>(event.data.data()),
                           event.data.size()) == "cpp-echo"
                   ? 0
                   : 3;
      }
    }
  }
  return 4;
}
