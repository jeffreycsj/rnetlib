#include "rnet.hpp"

#include <chrono>
#include <string>

int main() {
  rnet::GameRangeServerOptions server =
      rnet::game_range_server_options(rnet::GameProfile::Session);
  server.protocol.id = 91;
  server.protocol.min_version = 2;
  server.protocol.max_version = 8;
  rnet::Keypair client_key;
  std::array<uint8_t, 32> server_public_key;
  std::copy(server.keypair.public_key(), server.keypair.public_key() + 32,
            server_public_key.begin());
  rnet::GameRuntime runtime(client_key, server_public_key);
  const rnet_endpoint_t listener = runtime.listen_range(server);
  rnet::GameRangeClientOptions client =
      rnet::game_range_client_options(rnet::GameProfile::Session);
  client.host = "localhost";
  client.port = runtime.local_port(listener);
  client.protocol.id = 91;
  client.protocol.min_version = 4;
  client.protocol.max_version = 6;
  const rnet_endpoint_t client_endpoint = runtime.connect_range(client);
  bool server_ready = false;
  bool client_ready = false;
  bool received = false;
  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (!received && std::chrono::steady_clock::now() < deadline) {
    const std::vector<rnet::GameEvent> events = runtime.poll(16, 10);
    for (const auto &event : events) {
      if (event.type == RNET_GAME_AUTH_REQUEST) {
        if (runtime.selected_protocol_version(event.session) != 6)
          return 1;
        runtime.auth_decide(event.session, true);
      } else if (event.type == RNET_GAME_SESSION_READY && event.endpoint == listener) {
        server_ready = runtime.selected_protocol_version(event.session) == 6;
      } else if (event.type == RNET_GAME_SESSION_READY && event.endpoint == client_endpoint) {
        client_ready = runtime.selected_protocol_version(event.session) == 6;
        runtime.send(event.session, std::string("range-v4"));
      } else if (event.type == RNET_GAME_MESSAGE) {
        received = std::string(event.data.begin(), event.data.end()) == "range-v4";
      }
    }
  }
  const rnet::GameTransportLatest latest = runtime.transport_latest_snapshot();
  if (!server_ready || !client_ready || !received ||
      runtime.transport_latest_replacements() != 0 ||
      latest.pending_replaced != 0 || latest.worker_pickups != 0)
    return 2;
  runtime.close();
  return 0;
}
