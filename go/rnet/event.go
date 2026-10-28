package rnet

/*
#include "native.h"
*/
import "C"

import (
	"fmt"
	"math"
	"time"
	"unsafe"
)

// Event owns a copied transport payload. Interpret fields according to Type.
// Erase copied authorization credentials when no longer needed; do not log Data blindly.
type Event struct {
	// Type transport event kind; determines meaningful fields.
	Type EventType
	// Endpoint local owning endpoint; zero for runtime-wide events.
	Endpoint Endpoint
	// Session local session; zero for runtime/endpoint events or failures before allocation.
	Session Session
	// CorrelationID EventMessage request/trace metadata; zero when absent.
	CorrelationID uint64
	// Status local zero or negative native error codes category; not a peer's business error code.
	Status int32
	// Data owned payload/diagnostic bytes. AuthRequest contains client key[32] then join bytes; caller erases credentials.
	Data []byte
}

// SecurityChange describes a locally completed security barrier, not a peer delivery receipt.
type SecurityChange struct {
	// Operation locally completed mode switch or key rotation.
	Operation SecurityOperation
	// Mode resulting business-data protection; controls stay authenticated.
	Mode SecurityMode
	// Epoch session-local security generation, not wall time.
	Epoch uint64
}

// ClientPublicKey copies the authenticated key from EventAuthRequest.
// It returns an error for another event type or a truncated authorization payload.
func (e Event) ClientPublicKey() ([32]byte, error) {
	var key [32]byte
	if e.Type != EventAuthRequest || len(e.Data) < len(key) {
		return key, fmt.Errorf("rnet: event is not a valid auth request")
	}
	copy(key[:], e.Data[:len(key)])
	return key, nil
}

// JoinPayload copies the opaque authorization bytes from EventAuthRequest.
// It returns an error for another type or missing key prefix; empty credentials are valid.
func (e Event) JoinPayload() ([]byte, error) {
	if e.Type != EventAuthRequest || len(e.Data) < 32 {
		return nil, fmt.Errorf("rnet: event is not a valid auth request")
	}
	return append([]byte(nil), e.Data[32:]...), nil
}

// SecurityChange decodes the ten-byte EventSecurityChanged payload.
// It rejects other event kinds, malformed lengths and unknown operation tags.
func (e Event) SecurityChange() (SecurityChange, error) {
	if e.Type != EventSecurityChanged || len(e.Data) != 10 {
		return SecurityChange{}, fmt.Errorf("rnet: event is not a valid security change")
	}
	operation := SecurityOperation(e.Data[9])
	if operation != SecurityOperationModeSwitch && operation != SecurityOperationRekey {
		return SecurityChange{}, fmt.Errorf("rnet: security event has an unknown operation")
	}
	var epoch uint64
	for _, value := range e.Data[1:9] {
		epoch = epoch<<8 | uint64(value)
	}
	return SecurityChange{Operation: operation, Mode: SecurityMode(e.Data[0]), Epoch: epoch}, nil
}

// Poll consumes at most capacity events and copies their payloads into Go-owned memory.
// Negative capacity is invalid; zero validates the runtime but returns no events.
// timeout is an idle wait, truncated to milliseconds and clamped to [0, MaxUint32],
// not a total-call deadline. Prefer one event consumer to preserve application order.
// Invalid native buffers (including lengths above MaxInt32) return an error;
// all exported tokens are released even if polling or copying fails.
func (r *Runtime) Poll(capacity int, timeout time.Duration) (result []Event, err error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return nil, err
	}
	if capacity < 0 {
		return nil, StatusError{Code: int32(C.RNET_E_INVALID_ARGUMENT), Message: "capacity must not be negative"}
	}
	if capacity == 0 {
		var count C.size_t
		err := statusError(C.rnet_poll_events(handle, nil, 0, 0, &count))
		return nil, err
	}
	native := make([]C.rnet_event_t, capacity)
	timeoutMS := timeout.Milliseconds()
	if timeoutMS < 0 {
		timeoutMS = 0
	} else if timeoutMS > int64(^uint32(0)) {
		timeoutMS = int64(^uint32(0))
	}
	var count C.size_t
	err = statusError(C.rnet_poll_events(handle, &native[0], C.size_t(capacity), C.uint32_t(timeoutMS), &count))
	// A failed native poll may still export a prefix. Capture its thread-local
	// diagnostic before release calls replace it, and never abandon later tokens.
	defer func() {
		for i := 0; i < int(count); i++ {
			if token := native[i].buffer_token; token != 0 {
				if releaseErr := statusError(C.rnet_buffer_release(handle, token)); err == nil {
					err = releaseErr
				}
			}
		}
		if err != nil {
			result = nil
		}
	}()
	if err != nil {
		return nil, err
	}
	events := make([]Event, 0, count)
	for i := 0; i < int(count); i++ {
		source := native[i]
		event := Event{
			Type:          EventType(source.event_type),
			Endpoint:      Endpoint(source.endpoint),
			Session:       Session(source.session),
			CorrelationID: uint64(source.correlation_id),
			Status:        int32(source.status),
		}
		data, copyErr := copyNativeBytes(source.data, source.data_len)
		if copyErr != nil {
			return nil, copyErr
		}
		event.Data = data
		events = append(events, event)
	}
	return events, nil
}

// C.GoBytes accepts a signed C int, not size_t. Validate before narrowing so
// malformed or oversized native buffers cannot panic or silently truncate.
func copyNativeBytes(pointer *C.uint8_t, length C.size_t) ([]byte, error) {
	if length == 0 {
		return nil, nil
	}
	if pointer == nil || uint64(length) > math.MaxInt32 {
		return nil, fmt.Errorf("rnet: invalid native buffer")
	}
	return C.GoBytes(unsafe.Pointer(pointer), C.int(length)), nil
}
