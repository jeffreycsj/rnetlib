package rnet

/*
#cgo CFLAGS: -I${SRCDIR}/../../include
#cgo linux LDFLAGS: -L${SRCDIR}/../../lib -Wl,-Bstatic -lrnet -Wl,-Bdynamic -lpthread -ldl -lm
#include "native.h"
*/
import "C"

import (
	"fmt"
	"runtime"
	"time"
	"unsafe"
)

type StatusError struct {
	Code    int32
	Message string
}

func (e StatusError) Error() string {
	if e.Message != "" {
		return e.Message
	}
	return fmt.Sprintf("rnet status %d", e.Code)
}

// Error text belongs to native OS-thread-local storage. Keep the failing C call
// and the subsequent string copy on that same thread, including across preemption.
func lockNativeThread() func() {
	runtime.LockOSThread()
	return runtime.UnlockOSThread
}

func statusError(status C.int32_t) error {
	if status == C.RNET_OK {
		return nil
	}
	message := C.rnet_last_error_message()
	text := ""
	if message != nil {
		text = C.GoString(message)
	}
	return StatusError{Code: int32(status), Message: text}
}

func bytePointer(bytes []byte) *C.uint8_t {
	if len(bytes) == 0 {
		return nil
	}
	return (*C.uint8_t)(unsafe.Pointer(&bytes[0]))
}

func microseconds(value C.uint64_t) time.Duration {
	const maxMicroseconds = uint64(^uint64(0)>>1) / uint64(time.Microsecond)
	value64 := uint64(value)
	if value64 > maxMicroseconds {
		value64 = maxMicroseconds
	}
	return time.Duration(value64) * time.Microsecond
}
