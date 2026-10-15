// Package rnet provides bounded event-driven client/server transport and game runtimes.
// Use GameRuntime for protected game negotiation, heartbeats, recovery and staging;
// Runtime exposes transport frames without those game-layer services.
//
// Listen and Connect select TCP, UDP or KCP once per endpoint. Sends use a session,
// not an endpoint or transport selector. Server authorization and business-version
// negotiation must complete before game sends: continuously Poll on both peers,
// including while idle. A successful send is local admission, not remote delivery.
//
// Configuration and send bytes are copied across the native boundary; returned
// events own Go copies. Erase caller-owned secrets and tickets when no longer needed.
// Server-led encryption changes are automatic on clients. Plaintext business mode
// provides neither confidentiality nor integrity, even though controls stay protected.
//
// Do not copy runtime values after use. Close explicitly after quiescing callers;
// logger callbacks must return promptly and cannot Stop/Close their own runtime.
// Native failures preserve StatusError.Code and diagnostic; local validation errors
// can be ordinary Go errors. Only the matching current SDK/native ABI is supported.
package rnet
