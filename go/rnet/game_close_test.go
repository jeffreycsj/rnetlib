package rnet

import (
	"sync"
	"testing"
	"time"
)

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
