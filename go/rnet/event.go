package rnet

/*
#include "native.h"
*/
import "C"

import (
	"fmt"
	"time"
	"unsafe"
)

type Event struct {
	Type        EventType
	Endpoint    Endpoint
	Session     Session
	MessageType uint32
	StreamID    uint32
	RequestID   uint64
	Status      int32
	Data        []byte
}

type SecurityChange struct {
	Operation SecurityOperation
	Mode      SecurityMode
	Epoch     uint64
}

func (e Event) ClientPublicKey() ([32]byte, error) {
	var key [32]byte
	if e.Type != EventAuthRequest || len(e.Data) < len(key) {
		return key, fmt.Errorf("rnet: event is not a valid auth request")
	}
	copy(key[:], e.Data[:len(key)])
	return key, nil
}

func (e Event) JoinPayload() ([]byte, error) {
	if e.Type != EventAuthRequest || len(e.Data) < 32 {
		return nil, fmt.Errorf("rnet: event is not a valid auth request")
	}
	return append([]byte(nil), e.Data[32:]...), nil
}

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

func (r *Runtime) Poll(capacity int, timeout time.Duration) ([]Event, error) {
	handle, err := r.handleValue()
	if err != nil {
		return nil, err
	}
	if capacity < 0 {
		return nil, StatusError{Code: int32(C.RNET_E_INVALID_ARGUMENT), Message: "capacity must not be negative"}
	}
	if capacity == 0 {
		var count C.size_t
		err := statusError(C.rnet_poll_events_ex(handle, nil, 0, 0, &count))
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
	if err := statusError(C.rnet_poll_events_ex(handle, &native[0], C.size_t(capacity), C.uint32_t(timeoutMS), &count)); err != nil {
		return nil, err
	}
	events := make([]Event, 0, count)
	for i := 0; i < int(count); i++ {
		source := native[i]
		event := Event{
			Type:        EventType(source.event_type),
			Endpoint:    Endpoint(source.endpoint),
			Session:     Session(source.session),
			MessageType: uint32(source.msg_type),
			StreamID:    uint32(source.stream_id),
			RequestID:   uint64(source.request_id),
			Status:      int32(source.status),
		}
		if source.data_len != 0 {
			event.Data = C.GoBytes(unsafe.Pointer(source.data), C.int(source.data_len))
		}
		if source.buffer_token != 0 {
			if err := statusError(C.rnet_buffer_release(handle, source.buffer_token)); err != nil {
				return nil, err
			}
		}
		events = append(events, event)
	}
	return events, nil
}
