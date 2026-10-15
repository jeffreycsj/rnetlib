package rnet

/*
#include "native.h"
*/
import "C"

// GameProfile chooses safe connection defaults once; it is not sent with messages.
type GameProfile uint32

const (
	// GameProfileRealtime — UDP with encryption enabled; application tolerates losses.
	GameProfileRealtime GameProfile = C.RNET_GAME_PROFILE_REALTIME
	// GameProfileReliableRealtime — KCP with encryption enabled; reliable real-time traffic.
	GameProfileReliableRealtime GameProfile = C.RNET_GAME_PROFILE_RELIABLE_REALTIME
	// GameProfileSession — TCP with encryption enabled; ordered session traffic.
	GameProfileSession GameProfile = C.RNET_GAME_PROFILE_SESSION
)

// GameProfileDefaults returns a profile's fixed transport and encrypted server default.
// Unknown values return an error; the profile does not change per send.
func GameProfileDefaults(profile GameProfile) (Transport, SecurityMode, error) {
	defer lockNativeThread()()
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

// GameServerConfigForProfile fills only transport and initial security from profile.
// The caller must supply bind address, identity and business protocol before Listen.
func GameServerConfigForProfile(profile GameProfile) (GameServerConfig, error) {
	transport, security, err := GameProfileDefaults(profile)
	return GameServerConfig{Transport: transport, InitialSecurity: security}, err
}

// GameClientConfigForProfile fills only transport from profile.
// The caller must supply remote address/port and business protocol before Connect.
func GameClientConfigForProfile(profile GameProfile) (GameClientConfig, error) {
	transport, _, err := GameProfileDefaults(profile)
	return GameClientConfig{Transport: transport}, err
}
