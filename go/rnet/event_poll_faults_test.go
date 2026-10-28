//go:build linux && rnet_test_faults

package rnet

import (
	"bytes"
	"math"
	"testing"
)

func TestTransportPollValidatesBuffersAndReleasesExportedPrefix(t *testing.T) {
	for _, tc := range []struct {
		name                              string
		length                            uint64
		nullData, pollError, releaseError bool
	}{
		{name: "binary", length: 3},
		{name: "empty", nullData: true},
		{name: "over_int32", length: math.MaxInt32 + 1},
		{name: "uint32_max", length: math.MaxUint32},
		{name: "null_nonempty", length: 1, nullData: true},
		{name: "partial_native_error", length: 3, pollError: true},
		{name: "release_error", length: 3, releaseError: true},
		{name: "copy_and_release_error", length: math.MaxInt32 + 1, releaseError: true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			r, err := NewRuntime()
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() {
				disarmTransportPollFault()
				if err := r.Close(); err != nil {
					t.Error(err)
				}
			})
			_, nativeError := r.LocalPort(0)
			if nativeError == nil {
				t.Fatal("expected invalid endpoint diagnostic")
			}
			armTransportPollFault(r, tc.length, tc.nullData, tc.pollError, tc.releaseError)
			defer func() {
				if recovered := recover(); recovered != nil {
					t.Errorf("Poll panicked: %v", recovered)
				}
				if got := transportReleaseCounts(); got != [4]uint32{1, 1, 1, 0} {
					t.Errorf("release counts = %v; must release only exported prefix exactly once", got)
				}
			}()
			events, err := r.Poll(4, 0)
			invalid := tc.length > math.MaxInt32 || (tc.nullData && tc.length != 0)
			if invalid || tc.pollError || tc.releaseError {
				if err == nil {
					t.Fatal("expected error")
				}
				want := nativeError.Error()
				if invalid {
					want = "rnet: invalid native buffer"
				}
				if err.Error() != want {
					t.Errorf("error = %v, want %s", err, want)
				}
				if events != nil {
					t.Errorf("error must not expose a partial batch: %v", events)
				}
				return
			}
			if err != nil || len(events) != 3 {
				t.Fatalf("events=%v err=%v", events, err)
			}
			for i, event := range events {
				want := []byte{65, 0, 255}
				if i == 1 && tc.length == 0 {
					want = nil
				}
				if !bytes.Equal(event.Data, want) {
					t.Errorf("event %d payload = %v, want %v", i, event.Data, want)
				}
			}
		})
	}
}
