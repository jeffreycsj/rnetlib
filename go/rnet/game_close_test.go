package rnet

import (
	"errors"
	"strings"
	"sync"
	"testing"
	"time"
)

func requireGameLifecycleStatus(t *testing.T, err error, code int32, detail string) {
	t.Helper()
	var status StatusError
	if !errors.As(err, &status) || status.Code != code || !strings.Contains(status.Message, detail) {
		t.Fatalf("got %v; want status %d containing %q", err, code, detail)
	}
}

func TestGameCallbackShutdownFailureLeavesRuntimeForOwnerToClose(t *testing.T) {
	result := make(chan [3]error, 1)
	ready := make(chan *GameRuntime, 1)
	var once sync.Once
	r, err := NewGameRuntime(nil, GameConfig{Logger: func(LogRecord) {
		once.Do(func() {
			r := <-ready
			stopErr := r.Stop(0)
			closeErr := r.Close()
			_, queryErr := r.MetricsSnapshot()
			result <- [3]error{stopErr, closeErr, queryErr}
		})
	}})
	if err != nil {
		t.Fatal(err)
	}
	ready <- r
	t.Cleanup(func() {
		if err := r.Close(); err != nil {
			t.Error(err)
		}
	})
	if _, err := r.Poll(1, 0); err != nil {
		t.Fatal(err)
	}
	select {
	case got := <-result:
		requireGameLifecycleStatus(t, got[0], -3, "logger callback")
		requireGameLifecycleStatus(t, got[1], -3, "logger callback")
		if got[2] != nil {
			t.Fatalf("shutdown rejection prevented callback query: %v", got[2])
		}
	case <-time.After(3 * time.Second):
		t.Fatal("callback shutdown did not return")
	}
	if r.logger == 0 || r.logger.Value() != r.loggerSink {
		t.Fatal("failed stop released callback context")
	}
	if err := r.Stop(0); err != nil {
		t.Fatal(err)
	}
	if err := r.Close(); err != nil {
		t.Fatal(err)
	}
	if r.logger != 0 {
		t.Fatal("owner close did not release callback handle")
	}
	if _, err := r.MetricsSnapshot(); err == nil {
		t.Fatal("closed runtime accepted metrics query")
	}
}

func TestGameCloseAllowsAnInflightLoggerToFinishQuerying(t *testing.T) {
	started := make(chan struct{})
	query := make(chan struct{})
	var once sync.Once
	var r *GameRuntime
	var err error
	r, err = NewGameRuntime(nil, GameConfig{Logger: func(LogRecord) {
		once.Do(func() { close(started); <-query; _, _ = r.MetricsSnapshot() })
	}})
	if err != nil {
		t.Fatal(err)
	}
	if _, err = r.Poll(1, 0); err != nil {
		t.Fatal(err)
	}
	select {
	case <-started:
	case <-time.After(2 * time.Second):
		t.Fatal("logger did not start")
	}
	var release sync.Once
	unblock := func() { release.Do(func() { close(query) }) }
	defer unblock()
	done := make(chan error, 1)
	go func() { done <- r.Close() }()
	deadline := time.Now().Add(time.Second)
	for time.Now().Before(deadline) {
		if r.mu.TryRLock() {
			closed := r.handle == 0
			r.mu.RUnlock()
			if closed {
				// Destroy is joining the blocked logger. Another closer must return
				// promptly without freeing the callback context still in use.
				requireGameLifecycleStatus(t, r.Close(), -4, "closing")
				if _, err := r.MetricsSnapshot(); err == nil {
					t.Fatal("runtime accepted a query during destruction")
				}
				r.mu.RLock()
				loggerAlive := r.logger != 0 && r.logger.Value() == r.loggerSink
				r.mu.RUnlock()
				if !loggerAlive {
					t.Fatal("in-flight callback lost its context")
				}
				unblock()
				select {
				case err := <-done:
					if err != nil {
						t.Fatal(err)
					}
				case <-time.After(2 * time.Second):
					t.Fatal("destroy did not finish after query")
				}
				return
			}
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatal("Close holds the wrapper lock while native destroy joins the logger")
}
