//go:build linux && rnet_test_faults

package rnet

import (
	"math"
	"testing"
	"time"
)

func TestGameClockSnapshotDurationDoesNotWrap(t *testing.T) {
	r, err := NewGameRuntime(nil)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		clearGameClockSnapshot()
		if err := r.Close(); err != nil {
			t.Error(err)
		}
	})
	const maxUS = int64(math.MaxInt64) / int64(time.Microsecond)
	const minUS = int64(math.MinInt64) / int64(time.Microsecond)
	for _, test := range []struct {
		name  string
		input int64
		want  time.Duration
	}{
		{"zero_sample", 0, 0}, {"positive", 42, 42 * time.Microsecond}, {"negative", -42, -42 * time.Microsecond},
		{"largest_representable", maxUS, time.Duration(maxUS) * time.Microsecond},
		{"smallest_representable", minUS, time.Duration(minUS) * time.Microsecond},
		{"above_limit", maxUS + 1, time.Duration(maxUS) * time.Microsecond},
		{"below_limit", minUS - 1, time.Duration(minUS) * time.Microsecond},
		{"native_max", math.MaxInt64, time.Duration(maxUS) * time.Microsecond},
		{"native_min", math.MinInt64, time.Duration(minUS) * time.Microsecond},
	} {
		t.Run(test.name, func(t *testing.T) {
			armGameClockSnapshot(r, test.input, true)
			got, err := r.ClockSyncSnapshot(1)
			if err != nil || !got.Available || got.ServerMinusClient != test.want || got.RTT != 25*time.Microsecond || got.Samples != 3 {
				t.Fatalf("native offset=%d us: got=%+v err=%v want=%v", test.input, got, err, test.want)
			}
		})
	}
	armGameClockSnapshot(r, 0, false)
	got, err := r.ClockSyncSnapshot(1)
	if err != nil || got != (GameClockSync{}) {
		t.Fatalf("unavailable sample: %+v, %v", got, err)
	}
	// The hook is one-shot. Invalid native session errors must not become "no sample".
	if _, err := r.ClockSyncSnapshot(0); err == nil {
		t.Fatal("unknown native session accepted")
	}
}
