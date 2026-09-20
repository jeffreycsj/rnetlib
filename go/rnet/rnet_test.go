package rnet

import (
	"bytes"
	"testing"
	"time"
)

func TestTCPEchoCrossesCgoBoundary(t *testing.T) {
	config := DefaultConfig()
	config.AllowLegacyEndpoints = true
	runtime, err := NewRuntime(config)
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()

	listener, err := runtime.OpenTCPListener("127.0.0.1", 0)
	if err != nil {
		t.Fatal(err)
	}
	port, err := runtime.LocalPort(listener)
	if err != nil {
		t.Fatal(err)
	}
	client, err := runtime.OpenTCPClient("127.0.0.1", port)
	if err != nil {
		t.Fatal(err)
	}

	deadline := time.Now().Add(2 * time.Second)
	var clientSession, serverSession Session
	for time.Now().Before(deadline) && (clientSession == 0 || serverSession == 0) {
		events, err := runtime.Poll(16, 50*time.Millisecond)
		if err != nil {
			t.Fatal(err)
		}
		for _, event := range events {
			if event.Type != EventSessionOpened {
				continue
			}
			if event.Endpoint == client {
				clientSession = event.Session
			} else if event.Endpoint == listener {
				serverSession = event.Session
			}
		}
	}
	if clientSession == 0 || serverSession == 0 {
		t.Fatal("sessions did not open")
	}

	payload := []byte("go-echo")
	if err := runtime.Send(clientSession, 19, payload); err != nil {
		t.Fatal(err)
	}
	for time.Now().Before(deadline) {
		events, err := runtime.Poll(16, 50*time.Millisecond)
		if err != nil {
			t.Fatal(err)
		}
		for _, event := range events {
			if event.Type == EventMessage && event.Session == serverSession {
				if !bytes.Equal(event.Data, payload) {
					t.Fatalf("payload = %q", event.Data)
				}
				return
			}
		}
	}
	t.Fatal("message did not arrive")
}

func TestSecureJoinCrossesCgoBoundary(t *testing.T) {
	for name, transport := range map[string]Transport{
		"tcp": TransportTCP, "udp": TransportUDP, "kcp": TransportKCP,
	} {
		t.Run(name, func(t *testing.T) {
			serverKey, err := GenerateKeypair()
			if err != nil {
				t.Fatal(err)
			}
			clientKey, err := GenerateKeypair()
			if err != nil {
				t.Fatal(err)
			}
			config := DefaultConfig()
			config.ClientSecurity = &ClientSecurity{
				LocalKey: clientKey, ExpectedServerPublicKey: serverKey.Public[:],
			}
			runtime, err := NewRuntime(config)
			if err != nil {
				t.Fatal(err)
			}
			defer runtime.Close()
			listener, err := runtime.Listen(ServerConfig{
				Transport: transport, Host: "127.0.0.1", Keypair: serverKey,
				InitialSecurity: SecurityEncrypted,
			})
			if err != nil {
				t.Fatal(err)
			}
			port, err := runtime.LocalPort(listener)
			if err != nil {
				t.Fatal(err)
			}
			client, err := runtime.Connect(ClientConfig{
				Transport: transport, Host: "127.0.0.1", Port: port,
				JoinPayload: []byte("go-ticket"),
			})
			if err != nil {
				t.Fatal(err)
			}

			deadline := time.Now().Add(3 * time.Second)
			var clientSession, serverSession Session
			for time.Now().Before(deadline) && (clientSession == 0 || serverSession == 0) {
				events, pollErr := runtime.Poll(16, 50*time.Millisecond)
				if pollErr != nil {
					t.Fatal(pollErr)
				}
				for _, event := range events {
					switch event.Type {
					case EventAuthRequest:
						peerKey, keyErr := event.ClientPublicKey()
						if keyErr != nil || peerKey != clientKey.Public {
							t.Fatalf("client key = %x, err = %v", peerKey, keyErr)
						}
						joinPayload, payloadErr := event.JoinPayload()
						if payloadErr != nil || !bytes.Equal(joinPayload, []byte("go-ticket")) {
							t.Fatalf("ticket = %q, err = %v", joinPayload, payloadErr)
						}
						if err := runtime.AuthDecide(event.Session, true); err != nil {
							t.Fatal(err)
						}
					case EventSessionOpened:
						if event.Endpoint == client {
							clientSession = event.Session
						} else if event.Endpoint == listener {
							serverSession = event.Session
						}
					}
				}
			}
			if clientSession == 0 || serverSession == 0 {
				t.Fatal("secure sessions did not open")
			}
			if err := runtime.Send(clientSession, 1, []byte("go-secure")); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestDefaultRuntimeKeyRestoreAndMetrics(t *testing.T) {
	runtime, err := NewRuntime()
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()

	generated, err := GenerateKeypair()
	if err != nil {
		t.Fatal(err)
	}
	restored, err := KeypairFromPrivate(generated.Private[:])
	if err != nil {
		t.Fatal(err)
	}
	if restored != generated {
		t.Fatal("restored keypair differs from generated keypair")
	}
	if _, err := runtime.Metrics(); err != nil {
		t.Fatal(err)
	}
	latencies, err := runtime.LatencyMetrics()
	if err != nil {
		t.Fatal(err)
	}
	if len(latencies) != 8 {
		t.Fatalf("latency metric count = %d", len(latencies))
	}
	if err := runtime.SetMetricsLogInterval(0); err != nil {
		t.Fatal(err)
	}
	if _, err := runtime.Poll(-1, 0); err == nil {
		t.Fatal("negative poll capacity was accepted")
	}
	if err := runtime.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := runtime.Poll(0, 0); err == nil {
		t.Fatal("zero-capacity poll hid an invalid runtime handle")
	}
}

func TestStructuredLoggerAndV2Observability(t *testing.T) {
	records := make(chan LogRecord, 8)
	config := DefaultConfig()
	config.Logger = func(record LogRecord) {
		select {
		case records <- record:
		default:
		}
	}
	runtime, err := NewRuntime(config)
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()

	select {
	case record := <-records:
		if record.EventName != "runtime_created" || record.Runtime == 0 || record.TimestampUnixMillis == 0 {
			t.Fatalf("unexpected structured log: %+v", record)
		}
	case <-time.After(time.Second):
		t.Fatal("structured runtime-created log was not delivered")
	}

	metrics, err := runtime.Metrics()
	if err != nil {
		t.Fatal(err)
	}
	if metrics.QueuedSendBytes != 0 || len(metrics.SessionClosedByReason) != 19 {
		t.Fatalf("unexpected v2 metrics: %+v", metrics)
	}
	latencies, err := runtime.DrainLatencyMetrics()
	if err != nil {
		t.Fatal(err)
	}
	for _, latency := range latencies {
		if latency.P99 > latency.P999 {
			t.Fatalf("non-monotonic P99/P99.9: %+v", latency)
		}
	}
}
