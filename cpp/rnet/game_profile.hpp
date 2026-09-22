#ifndef RNET_GAME_PROFILE_HPP
#define RNET_GAME_PROFILE_HPP

#include "rnet/game.hpp"

namespace rnet {

struct GameProfileDefaults {
  Transport transport;
  bool initial_encryption;
};

inline GameProfileDefaults game_profile_defaults(GameProfile profile) {
  uint32_t transport = 0;
  uint32_t encrypted = 0;
  check(rnet_game_profile_defaults(static_cast<uint32_t>(profile), &transport,
                                   &encrypted));
  return {static_cast<Transport>(transport), encrypted != 0};
}

inline Transport game_profile_transport(GameProfile profile) {
  return game_profile_defaults(profile).transport;
}

inline GameServerOptions game_server_options(GameProfile profile) {
  GameServerOptions options;
  const GameProfileDefaults defaults = game_profile_defaults(profile);
  options.transport = defaults.transport;
  options.initial_encryption = defaults.initial_encryption;
  return options;
}

inline GameClientOptions game_client_options(GameProfile profile) {
  GameClientOptions options;
  options.transport = game_profile_transport(profile);
  return options;
}

inline GameRangeServerOptions game_range_server_options(GameProfile profile) {
  GameRangeServerOptions options;
  const GameProfileDefaults defaults = game_profile_defaults(profile);
  options.transport = defaults.transport;
  options.initial_encryption = defaults.initial_encryption;
  return options;
}

inline GameRangeClientOptions game_range_client_options(GameProfile profile) {
  GameRangeClientOptions options;
  options.transport = game_profile_transport(profile);
  return options;
}

}  // namespace rnet

#endif
