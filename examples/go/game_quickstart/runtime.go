package main

import (
	"bytes"
	"errors"
	"fmt"
	"log"
	"time"

	"rnet.local/netlib/go/rnet"
)

type options struct {
	transport rnet.Transport // Immutable for each endpoint.
	deny      bool           // Demonstrate a rejected login, not a production authorization policy.
	oneSlot   bool           // Set a real native scheduler admission limit of one message.
}

type report struct {
	received int // Byte-verified echoes, not successful local Send calls.
	blocked  int // Flush attempts deferred by native WouldBlock across both runtimes.
}

// run owns both runtimes; success includes explicit shutdown. No background
// application goroutine may outlive it. logger must be non-nil and remain alive
// through Close; log.Logger serializes callbacks from both native log workers.
func run(opts options, logger *log.Logger) (result report, err error) {
	serverKey, err := rnet.GenerateKeypair()
	if err != nil {
		return result, err
	}
	defer clear(serverKey.Private[:])
	clientKey, err := rnet.GenerateKeypair()
	if err != nil {
		return result, err
	}
	defer clear(clientKey.Private[:])
	config := rnet.GameConfig{MinLogLevel: rnet.LogInfo, Logger: func(record rnet.LogRecord) {
		// stderr is a demonstration sink; production should use a fast bounded sink.
		// Never stop/close from this callback or log keys, tickets, or payloads.
		logger.Printf("runtime=%d endpoint=%d session=%d code=%d %s %s",
			record.Runtime, record.Endpoint, record.Session, record.ErrorCode, record.EventName, record.Message)
	}}
	if opts.oneSlot {
		config.ScheduledQueue.MaxQueuedMessages = 1
	}
	server, err := rnet.NewGameRuntime(nil, config)
	if err != nil {
		return result, err
	}
	var client *rnet.GameRuntime
	defer func() {
		err = finish(err, func() error { return closeRuntime("client", client) },
			func() error { return closeRuntime("server", server) })
	}()
	// Real deployments load persistent server identity and distribute its pin via
	// a trusted channel. A key received over the connection is not a trust anchor.
	security := rnet.ClientSecurity{LocalKey: clientKey, ExpectedServerPublicKey: serverKey.Public[:]}
	defer clear(security.LocalKey.Private[:])
	client, err = rnet.NewGameRuntime(&security, config)
	clear(security.LocalKey.Private[:])
	clear(clientKey.Private[:])
	if err != nil {
		return result, err
	}
	protocol := rnet.GameProtocol{ID: 42, MinVersion: 1, MaxVersion: 1}
	listenConfig := rnet.GameServerConfig{
		Transport: opts.transport, Host: "127.0.0.1", Keypair: serverKey, Protocol: protocol,
	}
	defer clear(listenConfig.Keypair.Private[:])
	listener, err := server.Listen(listenConfig)
	clear(listenConfig.Keypair.Private[:])
	clear(serverKey.Private[:])
	if err != nil {
		return result, err
	}
	port, err := server.LocalPort(listener)
	if err != nil {
		return result, err
	}
	connection := rnet.GameClientConfig{
		Transport: opts.transport, Host: "127.0.0.1", Port: port,
		Protocol: protocol, JoinTicket: []byte("demo"),
	}
	endpoint, err := client.Connect(connection)
	clear(connection.JoinTicket)
	if err != nil {
		return result, err
	}
	return exchange(server, client, listener, endpoint, opts)
}

// exchange uses the same opaque Send/Poll API for each fixed transport. These
// strict two-message checks do not add delivery/order guarantees to raw UDP.
func exchange(server, client *rnet.GameRuntime, listener, endpoint rnet.Endpoint, opts options) (report, error) {
	deadline := time.Now().Add(10 * time.Second)
	expected := [][]byte{{'g', 0, 255, 'm'}, {}}
	var serverSession, clientSession rnet.Session
	var serverPending, clientPending pendingSends
	var result report
	for result.received < len(expected) && time.Now().Before(deadline) {
		events, err := server.Poll(64, time.Millisecond)
		if err != nil {
			return result, err
		}
		for _, event := range events {
			if err := eventFailure(event); err != nil {
				return result, err
			}
			switch event.Type {
			case rnet.GameAuthRequest:
				// Demo only: production verifies expiry/audience/signature and binds
				// authorization to the authenticated event.ClientPublicKey.
				accept := !opts.deny && bytes.Equal(event.Data, []byte("demo"))
				clear(event.Data)
				if err := server.AuthDecide(event.Session, accept); err != nil {
					return result, err
				}
			case rnet.GameSessionReady:
				if event.Endpoint != listener || serverSession != 0 {
					return result, errors.New("unexpected server readiness")
				}
				serverSession = event.Session
			case rnet.GameMessage:
				if serverSession == 0 || event.Session != serverSession {
					return result, errors.New("server message before readiness")
				}
				if err := serverPending.stage(event.Data); err != nil {
					return result, err
				}
			}
		}
		events, err = client.Poll(64, time.Millisecond)
		if err != nil {
			return result, err
		}
		for _, event := range events {
			if err := eventFailure(event); err != nil {
				return result, err
			}
			switch event.Type {
			case rnet.GameSessionReady:
				if event.Endpoint != endpoint || clientSession != 0 {
					return result, errors.New("unexpected client readiness")
				}
				clientSession = event.Session
				for _, payload := range expected {
					if err := clientPending.stage(payload); err != nil {
						return result, err
					}
				}
			case rnet.GameMessage:
				if clientSession == 0 || event.Session != clientSession || result.received >= len(expected) ||
					!bytes.Equal(event.Data, expected[result.received]) {
					return result, errors.New("binary/empty echo mismatch")
				}
				result.received++
			}
		}
		// Native admission only stages locally. Echo receipt above is the application
		// acknowledgement; poll must continue even when no Writable event arrives.
		for _, peer := range []struct {
			runtime *rnet.GameRuntime // Owner of the session; handles are not global.
			session rnet.Session      // Zero until that peer receives game readiness.
			pending *pendingSends     // Application copies awaiting local admission.
		}{{server, serverSession, &serverPending}, {client, clientSession, &clientPending}} {
			if peer.session == 0 {
				continue
			}
			blocked, err := peer.pending.flush(deadline, func(payload []byte) error { return peer.runtime.Send(peer.session, payload) })
			if err != nil {
				return result, err
			}
			if blocked {
				result.blocked++
			}
		}
	}
	if result.received != len(expected) {
		return result, errors.New("echo deadline exceeded; UDP is not reliable")
	}
	return result, finish(nil, func() error { return client.CloseSession(clientSession) },
		func() error { return server.CloseEndpoint(listener) })
}

// eventFailure deliberately treats endpoint errors as fatal for this demo, even
// when production could inspect endpoint validity and recover. No peer payload is
// used as diagnostic text; the structured logger carries detailed native causes.
func eventFailure(event rnet.GameEvent) error {
	code := event.Status
	switch event.Type {
	case rnet.GameJoinFailed, rnet.GameProtocolRejected, rnet.GameSessionClosed, rnet.GameEndpointError:
		if code == 0 {
			code = -3 // RNET_E_INVALID_STATE: premature lifecycle completion.
		}
	case rnet.GameProtocolViolation:
		code = -8 // ProtocolViolation can have status zero; it is never success.
	case rnet.GameRuntimeStopped:
		code = -3
	default:
		return nil
	}
	return rnet.StatusError{Code: code, Message: fmt.Sprintf("event=%d endpoint=%d session=%d code=%d; inspect lifecycle logs",
		event.Type, event.Endpoint, event.Session, code)}
}

// closeRuntime retries only backpressure, draining copied final events. The
// deadline bounds attempts, not a blocked native call or user logger. Stop all
// other callers before entering; never invoke this from a logger callback.
func closeRuntime(name string, runtime *rnet.GameRuntime) error {
	if runtime == nil {
		return nil
	}
	deadline := time.Now().Add(5 * time.Second)
	for {
		err := runtime.Close()
		if err == nil {
			return nil
		}
		if !isWouldBlock(err) || !time.Now().Before(deadline) {
			return fmt.Errorf("%s cleanup: %w", name, err)
		}
		if _, pollErr := runtime.Poll(64, time.Millisecond); pollErr != nil {
			return fmt.Errorf("%s cleanup: %w", name, errors.Join(err, pollErr))
		}
		time.Sleep(time.Millisecond)
	}
}
