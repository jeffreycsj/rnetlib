package main

import (
	"bytes"
	"errors"
	"fmt"
	"testing"
	"time"

	"rnet.local/netlib/go/rnet"
)

func TestPendingRetainsCopiesAcrossBackpressure(t *testing.T) {
	var q pendingSends
	source := []byte{'g', 0, 255}
	if err := q.stage(source); err != nil {
		t.Fatal(err)
	}
	if err := q.stage(nil); err != nil {
		t.Fatal(err)
	}
	source[0] = 'x'
	deadline := time.Now().Add(time.Second)
	blocked, err := q.flush(deadline, func([]byte) error {
		return fmt.Errorf("admission: %w", rnet.StatusError{Code: -4})
	})
	if !blocked || err != nil {
		t.Fatalf("WouldBlock should defer, got %v, %v", blocked, err)
	}
	var received [][]byte
	blocked, err = q.flush(deadline, func(p []byte) error {
		received = append(received, bytes.Clone(p))
		return nil
	})
	if blocked || err != nil || len(received) != 2 || !bytes.Equal(received[0], []byte{'g', 0, 255}) || len(received[1]) != 0 {
		t.Fatalf("lost copied binary/empty message: %v, %v, %v", received, blocked, err)
	}
	_, err = q.flush(deadline, func([]byte) error { t.Fatal("accepted message repeated"); return nil })
	if err != nil {
		t.Fatal(err)
	}
}

func TestPendingDeadlineAndTerminalErrorPreserveMessage(t *testing.T) {
	var q pendingSends
	if err := q.stage([]byte("keep")); err != nil {
		t.Fatal(err)
	}
	_, err := q.flush(time.Now().Add(-time.Second), func([]byte) error {
		t.Fatal("expired message submitted")
		return nil
	})
	if err == nil {
		t.Fatal("missing deadline error")
	}
	terminal := rnet.StatusError{Code: -3, Message: "session retired"}
	_, err = q.flush(time.Now().Add(time.Second), func([]byte) error { return terminal })
	if !errors.Is(err, terminal) {
		t.Fatalf("terminal diagnostic replaced: %v", err)
	}
	calls := 0
	_, err = q.flush(time.Now().Add(time.Second), func(p []byte) error {
		calls++
		if string(p) != "keep" {
			t.Fatal("pending value changed")
		}
		return nil
	})
	if err != nil || calls != 1 {
		t.Fatalf("failed admission removed message: calls=%d err=%v", calls, err)
	}
}

func TestPendingBoundsAndPartialAdmission(t *testing.T) {
	var q pendingSends
	if err := q.stage(make([]byte, 129)); err == nil {
		t.Fatal("oversize accepted")
	}
	if err := q.stage(make([]byte, 128)); err != nil {
		t.Fatal(err)
	}
	if err := q.stage(nil); err != nil {
		t.Fatal(err)
	}
	if err := q.stage(nil); err == nil {
		t.Fatal("empty message bypassed count limit")
	}
	deadline := time.Now().Add(time.Second)
	calls := 0
	blocked, err := q.flush(deadline, func([]byte) error {
		calls++
		if calls == 2 {
			return rnet.StatusError{Code: -4}
		}
		return nil
	})
	if !blocked || err != nil || calls != 2 {
		t.Fatalf("partial admission: %d %v %v", calls, blocked, err)
	}
	calls = 0
	_, err = q.flush(deadline, func(p []byte) error {
		calls++
		if len(p) != 0 {
			t.Fatal("accepted prefix repeated")
		}
		return nil
	})
	if err != nil || calls != 1 {
		t.Fatalf("pending suffix: %d %v", calls, err)
	}
}

func TestFinishAttemptsBothClosersAndPreservesErrors(t *testing.T) {
	primary := rnet.StatusError{Code: -13, Message: "auth denied"}
	cleanup := errors.New("close failed")
	for _, original := range []error{primary, nil} {
		calls := 0
		err := finish(original, func() error { calls++; return cleanup }, func() error { calls++; return nil })
		if calls != 2 || !errors.Is(err, cleanup) {
			t.Fatalf("cleanup skipped: calls=%d err=%v", calls, err)
		}
		if original != nil {
			var status rnet.StatusError
			if !errors.Is(err, primary) || !errors.As(err, &status) || status.Code != -13 {
				t.Fatalf("primary lost: %v", err)
			}
		}
	}
	if err := finish(nil, func() error { return nil }); err != nil {
		t.Fatal(err)
	}
}
