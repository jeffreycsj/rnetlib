//go:build linux && rnet_test_faults

package rnet

import (
	"errors"
	"fmt"
	"testing"
)

func TestGamePollFailureReleasesOnlyExportedPrefix(t *testing.T) {
	for _, capacity := range []int{0, 3} {
		t.Run(fmt.Sprintf("capacity_%d", capacity), func(t *testing.T) {
			r, err := NewGameRuntime(nil)
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { cleanupGamePollFailure(r); _ = r.Close() })
			_, want := r.SelectedProtocolVersion(0)
			if want == nil {
				t.Fatal("invalid session unexpectedly has a selected version")
			}
			armGamePollFailure(r)
			events, got := r.Poll(capacity, 0)
			var status StatusError
			if !errors.As(got, &status) || got.Error() != want.Error() || events != nil {
				t.Fatalf("poll lost its original native error: events=%v got=%v want=%v", events, got, want)
			}
			if capacity != 0 && releaseUnexportedGameBuffer(r) != 0 {
				t.Fatal("poll released a token outside the reported event prefix")
			}
			// Native destroy rejects outstanding tokens. This detects leaks of either
			// the main or auxiliary buffer independently of the wrapper implementation.
			if err := r.Close(); err != nil {
				t.Fatalf("partial poll leaked native buffers: %v", err)
			}
		})
	}
}
