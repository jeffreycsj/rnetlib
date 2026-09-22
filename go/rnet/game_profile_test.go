package rnet

import "testing"

func TestGameProfileDefaults(t *testing.T) {
	tests := []struct {
		profile   GameProfile
		transport Transport
	}{
		{GameProfileRealtime, TransportUDP},
		{GameProfileReliableRealtime, TransportKCP},
		{GameProfileSession, TransportTCP},
	}
	for _, test := range tests {
		server, err := GameServerConfigForProfile(test.profile)
		if err != nil || server.Transport != test.transport || server.InitialSecurity != SecurityEncrypted {
			t.Fatalf("invalid server defaults for profile %d: %+v, %v", test.profile, server, err)
		}
		client, err := GameRangeClientConfigForProfile(test.profile)
		if err != nil || client.Transport != test.transport {
			t.Fatalf("invalid client defaults for profile %d: %+v, %v", test.profile, client, err)
		}
	}
	if _, _, err := GameProfileDefaults(0); err == nil {
		t.Fatal("unknown profile must be rejected")
	}
}
