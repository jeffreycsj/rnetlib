package rnet

import (
	"runtime"
	"strings"
	"sync"
	"testing"
)

// Distinct errors share the native TLS slot. Concurrent callers must copy only
// their own cause even when Go reschedules them between wrapper operations.
func TestGameConcurrentErrorsRetainTheirNativeCause(t *testing.T) {
	r, err := NewGameRuntime(nil)
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()
	key, err := GenerateKeypair()
	if err != nil {
		t.Fatal(err)
	}
	var workers sync.WaitGroup
	for worker := 0; worker < 8; worker++ {
		workers.Add(1)
		go func(rangeError bool) {
			defer workers.Done()
			for attempt := 0; attempt < 100; attempt++ {
				runtime.Gosched()
				var got error
				want := "unknown transport"
				if rangeError {
					_, got = r.Listen(GameServerConfig{Transport: TransportTCP, Host: "127.0.0.1", Keypair: key})
					want = "invalid protocol range"
				} else {
					// Exact and range versions now share validation. Use another native
					// failure so this test still detects cross-thread cause corruption.
					_, got = r.Listen(GameServerConfig{Transport: 0, Host: "127.0.0.1", Keypair: key})
				}
				if got == nil || !strings.Contains(got.Error(), want) {
					t.Errorf("want %q, got %v", want, got)
					return
				}
			}
		}(worker%2 == 0)
	}
	workers.Wait()
}
