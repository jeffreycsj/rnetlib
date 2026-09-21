#include "rnet.hpp"

#include <chrono>
#include <string>

extern "C" void game_log_smoke(void *, uint64_t, uint32_t, const uint8_t *,
                                size_t, uint64_t, uint64_t, uint64_t, uint32_t,
                                int32_t, uint64_t, const uint8_t *, size_t) {}

int main() {
  rnet::GameServerOptions server;
  server.transport = rnet::Transport::Tcp;
  server.protocol.id = 77;
  server.protocol.version = 1;
  rnet::Keypair client_key;
  std::array<uint8_t, 32> server_public_key;
  std::copy(server.keypair.public_key(), server.keypair.public_key() + 32,
            server_public_key.begin());
  rnet_config_v5_t network_config{};
  rnet::check(rnet_config_v5_init(&network_config));
  network_config.max_endpoints = 4;
  rnet_game_config_t game_config{};
  rnet::check(rnet_game_config_init(&game_config));
  game_config.network_config = &network_config;
  rnet_logger_v2_t logger{};
  logger.struct_size = sizeof(logger);
  logger.abi_version = RNET_ABI_VERSION;
  logger.log = game_log_smoke;
  logger.min_level = RNET_LOG_INFO;
  rnet::GameRuntime runtime(game_config, client_key, server_public_key, logger);
  if (runtime.metrics_snapshot().logger_available != 1)
    return 9;
  const rnet_endpoint_t listener = runtime.listen(server);
  rnet::GameClientOptions client;
  client.transport = rnet::Transport::Tcp;
  client.host = "localhost";
  client.port = runtime.local_port(listener);
  client.protocol = server.protocol;
  client.join_ticket = {'c', 'p', 'p', '-', 'g', 'a', 'm', 'e'};
  const rnet_endpoint_t client_endpoint = runtime.connect(client);

  rnet_session_t server_session = 0;
  rnet_session_t client_session = 0;
  bool first_received = false;
  bool second_received = false;
  bool server_mapped = false;
  bool client_mapped = false;
  rnet_endpoint_t resumed_endpoint = 0;
  const auto deadline = std::chrono::steady_clock::now() +
                        std::chrono::seconds(3);
  while (std::chrono::steady_clock::now() < deadline) {
    const std::vector<rnet::GameEvent> events = runtime.poll(16, 10);
    for (const auto &event : events) {
      if (event.type == RNET_GAME_AUTH_REQUEST) {
        if (event.endpoint != listener || event.data != client.join_ticket)
          return 2;
        runtime.auth_decide(event.session, true);
      } else if (event.type == RNET_GAME_SESSION_READY &&
                 event.endpoint == listener) {
        server_session = event.session;
      } else if (event.type == RNET_GAME_SESSION_READY &&
                 event.endpoint == client_endpoint) {
        client_session = event.session;
        runtime.send(event.session, std::string("cpp-opaque"));
      } else if (event.type == RNET_GAME_MESSAGE) {
        if (event.session != server_session)
          return 3;
        const std::string payload(event.data.begin(), event.data.end());
        if (!first_received) {
          if (payload != "cpp-opaque" || client_session == 0)
            return 3;
          first_received = true;
          runtime.send_latest(client_session, 7, std::string("cpp-latest"));
          const rnet::GameRealtimeQueue queue = runtime.realtime_queue_snapshot();
          if (queue.queued_messages != 1 || queue.queued_bytes == 0)
            return 8;
          continue;
        }
        if (second_received || payload != "cpp-latest")
          return 3;
        second_received = true;
        const rnet::GameRealtimeQueue queue = runtime.realtime_queue_snapshot();
        if (queue.queued_messages != 0 || queue.forwarded == 0)
          return 8;
        const rnet::GameQuality quality = runtime.network_quality(server_session);
        const rnet::GameClockSync clock = runtime.clock_sync_snapshot(client_session);
        if (quality.has_udp_loss || quality.has_kcp_retransmissions ||
            runtime.clock_micros() == 0 || (clock.samples > 0) != clock.available ||
            runtime.prometheus_snapshot().find("rnet_game_heartbeat_") ==
                std::string::npos)
          return 7;
        runtime.issue_resume_ticket(server_session,
                                    std::vector<uint8_t>{'p', 'l', 'a', 'y', 'e', 'r'});
      } else if (event.type == RNET_GAME_RESUME_TICKET) {
        if (event.endpoint != client_endpoint)
          return 5;
        runtime.close_session(client_session);
        resumed_endpoint = runtime.connect_resume(client, client_session, event.data);
      } else if (event.type == RNET_GAME_RESUME_REQUEST) {
        if (event.related_session != server_session ||
            std::string(event.data.begin(), event.data.end()) != "player" ||
            event.aux_data != client.join_ticket)
          return 6;
        runtime.auth_decide(event.session, true);
      } else if (event.type == RNET_GAME_SESSION_RESUMED) {
        if (event.endpoint == listener)
          server_mapped = event.related_session == server_session &&
                          event.session != server_session;
        else if (event.endpoint == resumed_endpoint)
          client_mapped = event.related_session == client_session &&
                          event.session != client_session;
        if (server_mapped && client_mapped) {
          runtime.close();
          return 0;
        }
      }
    }
  }
  return 4;
}
