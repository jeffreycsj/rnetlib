package rnet

/*
#include "native.h"
*/
import "C"

import "runtime"

type SendOptions struct {
	CorrelationID uint64
}

func (r *Runtime) AuthDecide(session Session, accept bool) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	var decision C.uint32_t
	if accept {
		decision = 1
	}
	return statusError(C.rnet_session_auth_decide(handle, C.rnet_session_t(session), decision))
}

// SetSecurity asks the server side of a session to change its data protection mode.
// The connected client follows the authenticated transition automatically.
func (r *Runtime) SetSecurity(session Session, mode SecurityMode) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_session_security_set(
		handle, C.rnet_session_t(session), C.uint32_t(mode)))
}

func (r *Runtime) Rekey(session Session) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_session_rekey(handle, C.rnet_session_t(session)))
}

func (r *Runtime) Send(session Session, messageType uint32, payload []byte) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	err = statusError(C.rnet_go_send(
		handle,
		C.rnet_session_t(session),
		C.uint32_t(messageType),
		bytePointer(payload),
		C.size_t(len(payload)),
	))
	runtime.KeepAlive(payload)
	return err
}

func (r *Runtime) SendWithOptions(session Session, messageType uint32, payload []byte, options SendOptions) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	err = statusError(C.rnet_go_send_ex(
		handle, C.rnet_session_t(session), C.uint32_t(messageType),
		bytePointer(payload), C.size_t(len(payload)), C.uint64_t(options.CorrelationID),
	))
	runtime.KeepAlive(payload)
	return err
}

func (r *Runtime) SendLegacy(session Session, messageType, streamID uint32, requestID uint64, payload []byte) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	err = statusError(C.rnet_go_send_legacy(
		handle, C.rnet_session_t(session), C.uint32_t(messageType), C.uint32_t(streamID),
		bytePointer(payload), C.size_t(len(payload)), C.uint64_t(requestID),
	))
	runtime.KeepAlive(payload)
	return err
}

func (r *Runtime) CloseSession(session Session, reason int32) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_session_close(handle, C.rnet_session_t(session), C.int32_t(reason)))
}
