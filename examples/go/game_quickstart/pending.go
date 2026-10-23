package main

import (
	"bytes"
	"errors"
	"time"

	"rnet.local/netlib/go/rnet"
)

// pendingSends is an application-owned, single-goroutine demo queue, not SDK API.
// Two messages of at most 128 bytes bound retries independently of native budgets.
// Empty messages occupy a slot. Multiplayer applications need per-session limits.
type pendingSends struct {
	items [][]byte // Owned copies in admission order; nil is a valid empty payload.
}

func (q *pendingSends) stage(payload []byte) error {
	if len(payload) > 128 || len(q.items) >= 2 {
		return errors.New("example pending queue limit exceeded")
	}
	q.items = append(q.items, bytes.Clone(payload))
	return nil
}

// flush returns blocked=true only for native WouldBlock (-4). The caller must
// resume positive-capacity polling before retrying; Writable alone is insufficient
// for game scheduler capacity. send borrows bytes and must not mutate/reenter q.
// deadline bounds new attempts, not time spent inside a callback/native call.
func (q *pendingSends) flush(deadline time.Time, send func([]byte) error) (bool, error) {
	for len(q.items) != 0 {
		if !time.Now().Before(deadline) {
			return false, errors.New("pending send deadline exceeded")
		}
		if err := send(q.items[0]); err != nil {
			if isWouldBlock(err) {
				return true, nil
			}
			return false, err
		}
		q.items[0] = nil
		q.items = q.items[1:]
	}
	return false, nil
}

func isWouldBlock(err error) bool {
	var status rnet.StatusError
	return errors.As(err, &status) && status.Code == -4
}

// finish always attempts every closer. Joining keeps both cleanup diagnostics and
// the primary native category accessible through errors.Is/As (primary first).
func finish(primary error, closers ...func() error) error {
	for _, close := range closers {
		primary = errors.Join(primary, close())
	}
	return primary
}
