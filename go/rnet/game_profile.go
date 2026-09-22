package rnet

/*
#include "native.h"
*/
import "C"

// GameProfile chooses safe connection defaults once; it is not sent with messages.
type GameProfile uint32

const (
	GameProfileRealtime         GameProfile = C.RNET_GAME_PROFILE_REALTIME
	GameProfileReliableRealtime GameProfile = C.RNET_GAME_PROFILE_RELIABLE_REALTIME
	GameProfileSession          GameProfile = C.RNET_GAME_PROFILE_SESSION
)

// GameProfileDefaults returns the immutable transport and server security default.
func GameProfileDefaults(profile GameProfile) (Transport, SecurityMode, error) {
	var transport C.uint32_t
	var encrypted C.uint32_t
	if err := statusError(C.rnet_game_profile_defaults(C.uint32_t(profile), &transport, &encrypted)); err != nil {
		return 0, 0, err
	}
	security := SecurityPlaintext
	if encrypted != 0 {
		security = SecurityEncrypted
	}
	return Transport(transport), security, nil
}

func GameServerConfigForProfile(profile GameProfile) (GameServerConfig, error) {
	transport, security, err := GameProfileDefaults(profile)
	return GameServerConfig{Transport: transport, InitialSecurity: security}, err
}

func GameClientConfigForProfile(profile GameProfile) (GameClientConfig, error) {
	transport, _, err := GameProfileDefaults(profile)
	return GameClientConfig{Transport: transport}, err
}

func GameRangeServerConfigForProfile(profile GameProfile) (GameRangeServerConfig, error) {
	transport, security, err := GameProfileDefaults(profile)
	return GameRangeServerConfig{Transport: transport, InitialSecurity: security}, err
}

func GameRangeClientConfigForProfile(profile GameProfile) (GameRangeClientConfig, error) {
	transport, _, err := GameProfileDefaults(profile)
	return GameRangeClientConfig{Transport: transport}, err
}
