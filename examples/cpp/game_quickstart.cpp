// Two independent C++11 game runtimes. See game_quickstart_support.hpp for the
// bounded application retry/exception policy; it is example code, not SDK API.
#include "game_quickstart_support.hpp"
#include <memory>
#include <thread>

namespace {
using game_example::Clock;
using game_example::Payload;

struct Options {
  rnet::Transport transport = rnet::Transport::Kcp; // Fixed once for each endpoint.
  bool deny = false;       // Demo rejection; never a substitute for real authorization.
  bool one_slot = false;   // Real scheduler admission limit, not simulated packet loss.
  bool wrong_key = false;  // Negative trust scenario: deliberately mismatched server pin.
};

Options parse(int argc, char **argv) {
  Options options;
  if (argc > 1) {
    const std::string transport(argv[1]);
    if (transport == "tcp") options.transport = rnet::Transport::Tcp;
    else if (transport == "udp") options.transport = rnet::Transport::Udp;
    else if (transport != "kcp") throw std::invalid_argument("unknown transport");
  }
  for (int i = 2; i < argc; ++i) {
    const std::string flag(argv[i]);
    if (flag == "--deny" && !options.deny) options.deny = true;
    else if (flag == "--one-slot" && !options.one_slot) options.one_slot = true;
    else if (flag == "--wrong-key" && !options.wrong_key) options.wrong_key = true;
    else throw std::invalid_argument("unknown or duplicate option");
  }
  return options;
}

extern "C" void demo_log(void *, uint64_t, uint32_t, const uint8_t *name, size_t name_len,
    uint64_t runtime, uint64_t endpoint, uint64_t session, uint32_t, int32_t status,
    uint64_t, const uint8_t *message, size_t message_len) {
  // No C++ exceptions, borrowed-pointer retention, or runtime stop/close here.
  // stderr is illustrative; production injects a fast sink and monitors dropped logs.
  std::fprintf(stderr, "runtime=%llu endpoint=%llu session=%llu code=%d %.*s %.*s\n",
      static_cast<unsigned long long>(runtime), static_cast<unsigned long long>(endpoint),
      static_cast<unsigned long long>(session), static_cast<int>(status),
      static_cast<int>(std::min(name_len, size_t(128))), name ? reinterpret_cast<const char *>(name) : "",
      static_cast<int>(std::min(message_len, size_t(1024))), message ? reinterpret_cast<const char *>(message) : "");
}

void check_event(const rnet::GameEvent &event) {
  switch (event.type) {
    case RNET_GAME_JOIN_FAILED:
    case RNET_GAME_PROTOCOL_REJECTED:
    case RNET_GAME_SESSION_CLOSED:
    case RNET_GAME_ENDPOINT_ERROR:
      // Conservative demo policy: even a potentially recoverable endpoint error
      // aborts the exchange. Production may query endpoint validity and continue.
      throw rnet::Error(event.status == RNET_OK ? RNET_E_INVALID_STATE : event.status,
                        "connection failed before echo completion; inspect lifecycle logs");
    case RNET_GAME_PROTOCOL_VIOLATION:
      throw rnet::Error(RNET_E_PROTOCOL_ERROR, "game protocol rejected");
    case RNET_GAME_RUNTIME_STOPPED:
      throw rnet::Error(RNET_E_INVALID_STATE, "runtime stopped before echo completion");
    default: return;
  }
}

// Explicit close observes errors. The wrapper destructor remains a no-throw
// fallback, not evidence that shutdown succeeded. No concurrent users in this demo.
void close_runtime(rnet::GameRuntime &runtime) {
  const auto deadline = Clock::now() + std::chrono::seconds(5);
  for (;;) {
    try { runtime.close(); return; }
    catch (const rnet::Error &error) {
      if (error.status() != RNET_E_WOULD_BLOCK || Clock::now() >= deadline) throw;
    }
    (void)runtime.poll(64, 1); // Discard copied final events during shutdown.
    std::this_thread::sleep_for(std::chrono::milliseconds(1));
  }
}

struct Report {
  size_t received = 0; // Verified peer echoes, not local admissions.
  size_t blocked = 0;  // Flush calls deferred by WouldBlock across both peers.
};

Report exchange(rnet::GameRuntime &server, rnet::GameRuntime &client,
                const rnet::GameServerOptions &listener_options, const Options &options) {
  const auto deadline = Clock::now() + std::chrono::seconds(10);
  const auto listener = server.listen(listener_options);
  rnet::GameClientOptions connection;
  connection.transport = options.transport;
  connection.host = "127.0.0.1";
  connection.port = server.local_port(listener);
  connection.protocol = listener_options.protocol;
  connection.join_ticket = Payload{'d', 'e', 'm', 'o'};
  const auto endpoint = client.connect(connection);
  const std::vector<Payload> expected{{'g', 0, 255, 'm'}, {}};
  rnet_session_t server_session = 0, client_session = 0;
  game_example::PendingSends server_pending, client_pending;
  Report report;
  while (report.received < expected.size() && Clock::now() < deadline) {
    for (const auto &event : server.poll(64, 1)) {
      check_event(event);
      if (event.type == RNET_GAME_AUTH_REQUEST) {
        // Demo only. Real authorization validates signature/expiry/audience and
        // binding to event.client_public_key. Never deploy the literal credential.
        server.auth_decide(event.session, !options.deny && event.data == connection.join_ticket);
      } else if (event.type == RNET_GAME_SESSION_READY && event.endpoint == listener) {
        server_session = event.session;
      } else if (event.type == RNET_GAME_MESSAGE) {
        if (server_session == 0 || event.session != server_session)
          throw rnet::Error(RNET_E_INVALID_STATE, "server message before game readiness");
        server_pending.stage(event.data); // Copy before this event vector goes out of scope.
      }
    }
    for (const auto &event : client.poll(64, 1)) {
      check_event(event);
      if (event.type == RNET_GAME_SESSION_READY && event.endpoint == endpoint) {
        if (client_session != 0) throw rnet::Error(RNET_E_PROTOCOL_ERROR, "duplicate ready");
        client_session = event.session;
        for (const auto &payload : expected) client_pending.stage(payload);
      } else if (event.type == RNET_GAME_MESSAGE) {
        if (client_session == 0 || event.session != client_session ||
            report.received >= expected.size() || event.data != expected[report.received])
          throw rnet::Error(RNET_E_PROTOCOL_ERROR, "binary/empty echo mismatch");
        ++report.received;
      }
    }
    // Retry after positive-capacity polls, not only on Writable (transport capacity).
    if (server_session != 0)
      report.blocked += server_pending.flush([&](const Payload &p) { server.send(server_session, p); }, deadline);
    if (client_session != 0)
      report.blocked += client_pending.flush([&](const Payload &p) { client.send(client_session, p); }, deadline);
  }
  if (report.received != expected.size())
    throw rnet::Error(RNET_E_TIMEOUT, "echo deadline exceeded; UDP is not reliable");
  client.close_session(client_session);
  server.close_endpoint(listener); // Shared listener shutdown is explicit, not a session kick.
  return report;
}

Report run(const Options &options) {
  rnet::GameServerOptions listener;
  listener.transport = options.transport;
  listener.protocol.id = 42;
  listener.protocol.min_version = listener.protocol.max_version = 1;
  rnet::Keypair client_key;
  std::array<uint8_t, 32> pin;
  // Real deployments load persistent server identity and obtain this pin through
  // trusted distribution, not through an unauthenticated network connection.
  std::copy(listener.keypair.public_key(), listener.keypair.public_key() + 32, pin.begin());
  if (options.wrong_key) pin[0] ^= 1;
  rnet_game_config_t config{};
  rnet::check(rnet_game_config_init(&config));
  if (options.one_slot) config.scheduled_max_queued_messages = 1;
  rnet_logger_t logger{};
  logger.struct_size = sizeof(logger);
  logger.abi_version = RNET_ABI_VERSION;
  logger.min_level = RNET_LOG_INFO;
  logger.log = demo_log;
  rnet::GameRuntime server(config, logger);
  std::unique_ptr<rnet::GameRuntime> client;
  Report report;
  std::exception_ptr primary;
  try {
    client.reset(new rnet::GameRuntime(config, client_key, pin, logger));
    report = exchange(server, *client, listener, options);
  } catch (...) { primary = std::current_exception(); }
  game_example::finish(primary, [&] { if (client) close_runtime(*client); },
                               [&] { close_runtime(server); });
  return report;
}
} // namespace

int main(int argc, char **argv) {
  try {
    rnet::check_abi();
    const Options options = parse(argc, argv);
    const Report report = run(options);
    std::printf("game echo complete: received=%zu blocked=%zu; both runtimes closed\n",
                report.received, report.blocked);
    return 0;
  } catch (const std::invalid_argument &error) {
    std::fprintf(stderr, "%s; usage: %s [tcp|udp|kcp] [--one-slot] [--deny] [--wrong-key]\n", error.what(), argv[0]);
    return 2;
  } catch (...) {
    game_example::report_error("example", std::current_exception());
    return 1;
  }
}
