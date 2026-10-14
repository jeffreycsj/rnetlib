#include "rnet.hpp"

#include <chrono>
#include <cstdint>
#include <stdexcept>
#include <string>

int main() {
  if (rnet_abi_version() != RNET_ABI_VERSION) {
    return 1;
  }
  rnet_config_t config{};
  rnet::check(rnet_config_init(&config));
  config.allow_plaintext_business_data = 1;
  rnet::ServerOptions server;
  server.transport = rnet::Transport::Tcp;
  server.initial_security = rnet::SecurityMode::Plaintext;
  rnet::Keypair client_key;
  std::array<uint8_t, 32> expected_server_key;
  std::copy(server.keypair.public_key(), server.keypair.public_key() + 32,
            expected_server_key.begin());
  rnet::Runtime runtime(config, client_key, expected_server_key);
  const auto listener = runtime.listen(server);
  rnet::ClientOptions client_options;
  client_options.transport = rnet::Transport::Tcp;
  client_options.port = runtime.local_port(listener);
  const auto client = runtime.connect(client_options);

  rnet_session_t client_session = 0;
  rnet_session_t server_session = 0;
  const auto deadline = std::chrono::steady_clock::now() +
                        std::chrono::seconds(2);
  while (std::chrono::steady_clock::now() < deadline &&
         (client_session == 0 || server_session == 0)) {
    for (const auto &event : runtime.poll(16, 50)) {
      if (event.type == RNET_EVENT_AUTH_REQUEST) {
        runtime.auth_decide(event.session, true);
      }
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

  runtime.send(client_session, "cpp-echo");
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
