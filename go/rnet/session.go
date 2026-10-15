package rnet

/*
#include "native.h"
*/
import "C"

import "runtime"

// SendOptions carries optional transport metadata; business message typing belongs in payload.
type SendOptions struct {
	// CorrelationID optional request/trace ID copied to the peer; zero absent.
	CorrelationID uint64
}

// AuthDecide accepts or rejects a pending server EventAuthRequest by session handle.
// An accepted decision is asynchronous; await EventSessionOpened before sending.
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

// SetSecurity requests a server-led business protection change; clients follow automatically.
// Plaintext requires explicit runtime opt-in and has no confidentiality or integrity.
// Await EventSecurityChanged for completion; an already active mode may be a no-op.
func (r *Runtime) SetSecurity(session Session, mode SecurityMode) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_session_security_set(
		handle, C.rnet_session_t(session), C.uint32_t(mode)))
}

// Rekey requests a server-led key-generation change without changing business protection.
// Successful return queues the request; EventSecurityChanged reports local completion.
func (r *Runtime) Rekey(session Session) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_session_rekey(handle, C.rnet_session_t(session)))
}

// Send copies payload to a ready session's bounded send queue; nil is an empty message.
// Success means local admission, not remote delivery. Limits, backpressure, stale
// handles and sessions awaiting handshake/authorization return errors.
func (r *Runtime) Send(session Session, payload []byte) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	err = statusError(C.rnet_go_send(
		handle,
		C.rnet_session_t(session),
		bytePointer(payload),
		C.size_t(len(payload)),
	))
	runtime.KeepAlive(payload)
	return err
}

// SendWithOptions has Send's admission and ownership contract and includes options
// as peer-visible metadata. Neither payload nor metadata is retained by reference.
func (r *Runtime) SendWithOptions(session Session, payload []byte, options SendOptions) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	err = statusError(C.rnet_go_send_ex(
		handle, C.rnet_session_t(session),
		bytePointer(payload), C.size_t(len(payload)), C.uint64_t(options.CorrelationID),
	))
	runtime.KeepAlive(payload)
	return err
}

// CloseSession closes a local session with reason zero or a native negative status code.
// The peer is not guaranteed to observe the same reason; await local close diagnostics.
func (r *Runtime) CloseSession(session Session, reason int32) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_session_close(handle, C.rnet_session_t(session), C.int32_t(reason)))
}
