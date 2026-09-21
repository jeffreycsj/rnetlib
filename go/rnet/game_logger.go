package rnet

import "sync/atomic"

// Go panics cannot cross the C callback boundary. Keep their count beside the
// cgo handle so the game facade can report them without exposing a Go pointer.
type gameLoggerSink struct {
	callback func(LogRecord)
	panics   atomic.Uint64
}
