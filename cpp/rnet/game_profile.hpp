#ifndef RNET_GAME_PROFILE_HPP
#define RNET_GAME_PROFILE_HPP

#include "rnet/game.hpp"

namespace rnet {

/// Local scenario defaults; no protocol, trust, queue or connection is negotiated.
struct GameProfileDefaults {
  Transport transport; ///< Transport selected for the scenario; no connection is created.
  bool initial_encryption; ///< Server's initial business-data encryption default.
};

/// Expands local scenario defaults; unknown profile throws Error. Does not create a connection.
inline GameProfileDefaults game_profile_defaults(GameProfile profile) {
  uint32_t transport = 0;
  uint32_t encrypted = 0;
  check(rnet_game_profile_defaults(static_cast<uint32_t>(profile), &transport,
                                   &encrypted));
  return {static_cast<Transport>(transport), encrypted != 0};
}

/// Returns the profile's transport; peers must choose the same transport independently.
inline Transport game_profile_transport(GameProfile profile) {
  return game_profile_defaults(profile).transport;
}

/// Creates options with a fresh identity/profile defaults; fill protocol/address before listen.
inline GameServerOptions game_server_options(GameProfile profile) {
  GameServerOptions options;
  const GameProfileDefaults defaults = game_profile_defaults(profile);
  options.transport = defaults.transport;
  options.initial_encryption = defaults.initial_encryption;
  return options;
}

/// Creates profile options; fill protocol and remote port before connecting with runtime-scoped trust.
inline GameClientOptions game_client_options(GameProfile profile) {
  GameClientOptions options;
  options.transport = game_profile_transport(profile);
  return options;
}

}  // namespace rnet

#endif
