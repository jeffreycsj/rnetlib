//go:build linux && rnet_test_faults

package rnet

import "testing"

func TestGameCloseRetainsHandleAndLoggerUntilNativeDestroySucceeds(t *testing.T) {
	r, err := NewGameRuntime(nil, GameConfig{Logger: func(LogRecord) {}})
	if err != nil {
		t.Fatal(err)
	}
	handle, token, err := retainGameLifecycleBuffer(r)
	if err != nil || token == 0 {
		_ = r.Close()
		t.Fatalf("retain native buffer: token=%d err=%v", token, err)
	}
	t.Cleanup(func() {
		if token != 0 {
			if err := releaseGameLifecycleBuffer(handle, token); err != nil {
				t.Error(err)
			}
		}
		if err := r.Close(); err != nil {
			t.Error(err)
		}
	})
	logger := r.logger
	// A second failed attempt must neither lose the handle nor release the logger.
	for attempt := 0; attempt < 2; attempt++ {
		requireGameLifecycleStatus(t, r.Close(), -3, "release all game event buffers")
		if _, err := r.MetricsSnapshot(); err != nil {
			t.Fatalf("failed destroy lost queryable runtime: %v", err)
		}
		if r.logger != logger || logger.Value() != r.loggerSink {
			t.Fatal("failed destroy released or replaced callback context")
		}
	}
	events, err := r.Poll(64, 0)
	if err != nil {
		t.Fatal(err)
	}
	stopped := 0
	for _, event := range events {
		if event.Type == GameRuntimeStopped {
			stopped++
		}
	}
	if stopped != 1 {
		t.Fatalf("failed destroy must preserve the single successful stop event: %v", events)
	}
	if err := releaseGameLifecycleBuffer(handle, token); err != nil {
		t.Fatal(err)
	}
	token = 0
	if err := r.Close(); err != nil {
		t.Fatalf("retry after resolving ownership: %v", err)
	}
	if r.logger != 0 {
		t.Fatal("successful destroy retained callback handle")
	}
	// Query the saved native handle as well: closing only the Go wrapper is not enough.
	requireGameLifecycleStatus(t, stopGameLifecycleHandle(handle), -2, "invalid game runtime")
	if err := r.Close(); err != nil {
		t.Fatalf("repeated close: %v", err)
	}
}
