package rnet

/*
#include "native.h"
*/
import "C"

import (
	"runtime/cgo"
	"unsafe"
)

//export rnet_go_log_v2_bridge
func rnet_go_log_v2_bridge(
	userData unsafe.Pointer,
	timestampUnixMillis C.uint64_t,
	level C.uint32_t,
	eventName *C.uint8_t,
	eventNameLen C.size_t,
	runtimeID C.rnet_runtime_t,
	endpoint C.rnet_endpoint_t,
	session C.rnet_session_t,
	transport C.uint32_t,
	errorCode C.int32_t,
	correlationID C.uint64_t,
	message *C.uint8_t,
	messageLen C.size_t,
) {
	defer func() { _ = recover() }()
	logger, ok := cgo.Handle(uintptr(userData)).Value().(func(LogRecord))
	if !ok {
		return
	}
	logger(LogRecord{
		TimestampUnixMillis: uint64(timestampUnixMillis),
		Level:               LogLevel(level),
		EventName:           goString(eventName, eventNameLen),
		Runtime:             uint64(runtimeID),
		Endpoint:            Endpoint(endpoint),
		Session:             Session(session),
		Transport:           Transport(transport),
		ErrorCode:           int32(errorCode),
		CorrelationID:       uint64(correlationID),
		Message:             goString(message, messageLen),
	})
}

func goString(data *C.uint8_t, length C.size_t) string {
	if data == nil || length == 0 {
		return ""
	}
	return C.GoStringN((*C.char)(unsafe.Pointer(data)), C.int(length))
}
