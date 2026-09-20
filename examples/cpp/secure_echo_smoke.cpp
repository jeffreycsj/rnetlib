#include "rnet.hpp"

#include <chrono>
#include <string>

int main() {
  rnet::ServerOptions server;
  rnet::Keypair client_key;
  std::array<uint8_t, 32> expected_server_key;
  std::copy(server.keypair.public_key(), server.keypair.public_key() + 32,
            expected_server_key.begin());
  rnet_config_v5_t runtime_config = {};
  rnet::check(rnet_config_v5_init(&runtime_config));
  rnet::Runtime runtime(runtime_config, client_key, expected_server_key);
  server.transport = rnet::Transport::Tcp;
  server.initial_security = rnet::SecurityMode::Encrypted;
  const rnet_endpoint_t listener = runtime.listen(server);
  rnet::ClientOptions client_options;
  client_options.transport = rnet::Transport::Tcp;
  client_options.port = runtime.local_port(listener);
  client_options.join_payload = "cpp-ticket";
  const rnet_endpoint_t client = runtime.connect(client_options);

  rnet_session_t server_session = 0;
  rnet_session_t client_session = 0;
  const std::chrono::steady_clock::time_point deadline =
      std::chrono::steady_clock::now() + std::chrono::seconds(2);
  while (std::chrono::steady_clock::now() < deadline &&
         (server_session == 0 || client_session == 0)) {
    const std::vector<rnet::Event> events = runtime.poll(16, 50);
    for (std::vector<rnet::Event>::const_iterator it = events.begin();
         it != events.end(); ++it) {
      if (it->type == RNET_EVENT_AUTH_REQUEST) {
        const std::array<uint8_t, 32> peer_key = it->auth_client_public_key();
        const std::vector<uint8_t> join_payload = it->auth_join_payload();
        if (!std::equal(peer_key.begin(), peer_key.end(),
                        client_key.public_key()) ||
            std::string(join_payload.begin(), join_payload.end()) != "cpp-ticket") {
          return 5;
        }
        runtime.auth_decide(it->session, true);
      } else if (it->type == RNET_EVENT_SESSION_OPENED) {
        if (it->endpoint == listener) {
          server_session = it->session;
        } else if (it->endpoint == client) {
          client_session = it->session;
        }
      }
    }
  }
  if (server_session == 0 || client_session == 0) {
    return 2;
  }
  runtime.send(client_session, 1, std::string("cpp-secure"));
  while (std::chrono::steady_clock::now() < deadline) {
    const std::vector<rnet::Event> events = runtime.poll(16, 50);
    for (std::vector<rnet::Event>::const_iterator it = events.begin();
         it != events.end(); ++it) {
      if (it->type == RNET_EVENT_MESSAGE && it->session == server_session) {
        if (std::string(reinterpret_cast<const char *>(it->data.data()),
                        it->data.size()) != "cpp-secure") {
          return 3;
        }
        const rnet::Metrics metrics = runtime.metrics();
        const std::vector<rnet::LatencyMetric> latencies =
            runtime.latency_metrics(true);
        if (metrics.frames_received == 0 || latencies.size() != 8) {
          return 6;
        }
        for (std::vector<rnet::LatencyMetric>::const_iterator metric =
                 latencies.begin();
             metric != latencies.end(); ++metric) {
          if (metric->p99_us > metric->p999_us) {
            return 7;
          }
        }
        return 0;
      }
    }
  }
  return 4;
}
