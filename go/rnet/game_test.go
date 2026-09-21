package rnet

import (
	"bytes"
	"fmt"
	"strings"
	"testing"
	"time"
)

func TestGameDebugFormattingRedactsCredentials(t *testing.T) {
	server := GameServerConfig{Keypair: Keypair{Private: [32]byte{1, 2, 3}}}
	client := GameClientConfig{JoinTicket: []byte("private-ticket")}
	event := GameEvent{Type: GameResumeRequest, Data: []byte("private-identity"), AuxData: []byte("private-ticket")}
	for _, formatted := range []string{
		fmt.Sprintf("%+v", server), fmt.Sprintf("%#v", server),
		fmt.Sprintf("%+v", client), fmt.Sprintf("%#v", client),
		fmt.Sprintf("%+v", event), fmt.Sprintf("%#v", event),
	} {
		if strings.Contains(formatted, "Private:") || strings.Contains(formatted, "Data:[") ||
			strings.Contains(formatted, "JoinTicket:[") || strings.Contains(formatted, "private-") {
			t.Fatalf("sensitive game debug output: %s", formatted)
		}
	}
}

func TestGameFacadeJoinsAndExchangesOpaquePayload(t *testing.T) {
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
			runtime, err := NewGameRuntime(&ClientSecurity{
				LocalKey: clientKey, ExpectedServerPublicKey: serverKey.Public[:],
			}, GameConfig{HeartbeatInterval: 20 * time.Millisecond, HeartbeatTimeout: time.Second})
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				if err := runtime.Close(); err != nil {
					t.Error(err)
				}
			}()
			protocol := GameProtocol{ID: 91, Version: 1}
			listener, err := runtime.Listen(GameServerConfig{
				Transport: transport, Host: "127.0.0.1", Keypair: serverKey,
				Protocol: protocol,
			})
			if err != nil {
				t.Fatal(err)
			}
			port, err := runtime.LocalPort(listener)
			if err != nil {
				t.Fatal(err)
			}
			ticket := []byte("go-game-ticket")
			clientConfig := GameClientConfig{
				Transport: transport, Host: "localhost", Port: port,
				JoinTicket: ticket, Protocol: protocol,
			}
			client, err := runtime.Connect(clientConfig)
			if err != nil {
				t.Fatal(err)
			}
			deadline := time.Now().Add(3 * time.Second)
			var serverSession, clientSession Session
			for time.Now().Before(deadline) && (serverSession == 0 || clientSession == 0) {
				events, err := runtime.Poll(16, 10*time.Millisecond)
				if err != nil {
					t.Fatal(err)
				}
				for _, event := range events {
					switch event.Type {
					case GameAuthRequest:
						if !bytes.Equal(event.Data, ticket) || event.Endpoint != listener {
							t.Fatalf("unexpected authorization event endpoint=%d ticket_len=%d", event.Endpoint, len(event.Data))
						}
						if err := runtime.AuthDecide(event.Session, true); err != nil {
							t.Fatal(err)
						}
					case GameSessionReady:
						if event.Endpoint == listener {
							serverSession = event.Session
						} else if event.Endpoint == client {
							clientSession = event.Session
						}
					}
				}
			}
			if serverSession == 0 || clientSession == 0 {
				t.Fatal("game sessions did not become ready")
			}
			payload := []byte("go-opaque-payload")
			if err := runtime.Send(clientSession, payload); err != nil {
				t.Fatal(err)
			}
			received := false
			for time.Now().Before(deadline) && !received {
				events, err := runtime.Poll(16, 10*time.Millisecond)
				if err != nil {
					t.Fatal(err)
				}
				for _, event := range events {
					if event.Type == GameMessage && event.Session == serverSession {
						if !bytes.Equal(event.Data, payload) {
							t.Fatalf("payload = %q", event.Data)
						}
						received = true
					}
				}
			}
			if !received {
				t.Fatal("game message did not arrive")
			}
			if err := runtime.SendLatest(clientSession, 7, []byte("go-latest")); err != nil {
				t.Fatal(err)
			}
			queue, err := runtime.RealtimeQueueSnapshot()
			if err != nil || queue.QueuedMessages != 1 || queue.QueuedBytes == 0 {
				t.Fatalf("snapshot not staged: %+v, %v", queue, err)
			}
			deadline = time.Now().Add(3 * time.Second)
			latestReceived := false
			for time.Now().Before(deadline) && !latestReceived {
				events, err := runtime.Poll(16, 10*time.Millisecond)
				if err != nil {
					t.Fatal(err)
				}
				for _, event := range events {
					if event.Type == GameMessage && event.Session == serverSession {
						if !bytes.Equal(event.Data, []byte("go-latest")) {
							t.Fatalf("latest payload = %q", event.Data)
						}
						latestReceived = true
					}
				}
			}
			if !latestReceived {
				t.Fatal("game latest message did not arrive")
			}
			queue, err = runtime.RealtimeQueueSnapshot()
			if err != nil || queue.QueuedMessages != 0 || queue.Forwarded == 0 {
				t.Fatalf("snapshot not forwarded: %+v, %v", queue, err)
			}
			deadline = time.Now().Add(3 * time.Second)
			var quality GameQuality
			for time.Now().Before(deadline) && !quality.Available {
				quality, err = runtime.NetworkQuality(serverSession)
				if err != nil {
					t.Fatal(err)
				}
				if !quality.Available {
					if _, err := runtime.Poll(16, 10*time.Millisecond); err != nil {
						t.Fatal(err)
					}
				}
			}
			if !quality.Available || quality.Samples == 0 ||
				quality.HasUDPLoss != (transport == TransportUDP) ||
				quality.HasKCPRetransmissions != (transport == TransportKCP) {
				t.Fatalf("invalid quality availability: %+v", quality)
			}
			if micros, err := runtime.ClockMicros(); err != nil || micros == 0 {
				t.Fatalf("game clock unavailable: %d, %v", micros, err)
			}
			deadline = time.Now().Add(3 * time.Second)
			var clock GameClockSync
			for time.Now().Before(deadline) && !clock.Available {
				clock, err = runtime.ClockSyncSnapshot(clientSession)
				if err != nil {
					t.Fatal(err)
				}
				if !clock.Available {
					if _, err := runtime.Poll(16, 10*time.Millisecond); err != nil {
						t.Fatal(err)
					}
				}
			}
			if !clock.Available || clock.Samples == 0 {
				t.Fatalf("clock sync unavailable: %+v", clock)
			}
			gameMetrics, err := runtime.MetricsSnapshot()
			if err != nil || gameMetrics.Heartbeat.RTTSamples == 0 || gameMetrics.Clock.Samples == 0 || gameMetrics.LoggerAvailable {
				t.Fatalf("invalid game metrics: %+v, %v", gameMetrics, err)
			}
			metrics, err := runtime.PrometheusSnapshot()
			if err != nil || !strings.Contains(metrics, "rnet_game_heartbeat_") {
				t.Fatalf("missing game metrics: %v", err)
			}
			if err := runtime.IssueResumeTicket(serverSession, []byte("player-91")); err != nil {
				t.Fatal(err)
			}
			deadline = time.Now().Add(3 * time.Second)
			var resumeTicket []byte
			for time.Now().Before(deadline) && resumeTicket == nil {
				events, err := runtime.Poll(16, 10*time.Millisecond)
				if err != nil {
					t.Fatal(err)
				}
				for _, event := range events {
					if event.Type == GameResumeTicket && event.Endpoint == client {
						resumeTicket = event.Data
					}
				}
			}
			if resumeTicket == nil {
				t.Fatal("resume ticket did not arrive")
			}
			if err := runtime.CloseSession(clientSession); err != nil {
				t.Fatal(err)
			}
			resumedClient, err := runtime.ConnectResume(clientConfig, clientSession, resumeTicket)
			if err != nil {
				t.Fatal(err)
			}
			deadline = time.Now().Add(3 * time.Second)
			var serverMapped, clientMapped bool
			for time.Now().Before(deadline) && (!serverMapped || !clientMapped) {
				events, err := runtime.Poll(16, 10*time.Millisecond)
				if err != nil {
					t.Fatal(err)
				}
				for _, event := range events {
					switch event.Type {
					case GameResumeRequest:
						if event.RelatedSession != serverSession ||
							!bytes.Equal(event.Data, []byte("player-91")) ||
							!bytes.Equal(event.AuxData, ticket) {
							t.Fatal("invalid resume request")
						}
						if err := runtime.AuthDecide(event.Session, true); err != nil {
							t.Fatal(err)
						}
					case GameSessionResumed:
						if event.Endpoint == listener {
							serverMapped = event.RelatedSession == serverSession && event.Session != serverSession
						} else if event.Endpoint == resumedClient {
							clientMapped = event.RelatedSession == clientSession && event.Session != clientSession
						}
					}
				}
			}
			if !serverMapped || !clientMapped {
				t.Fatal("resume mappings did not arrive")
			}
		})
	}
}

func TestGameLoggerAndMetricsAreAvailableWithoutTransportRuntime(t *testing.T) {
	records := make(chan LogRecord, 8)
	runtime, err := NewGameRuntime(nil, GameConfig{Logger: func(record LogRecord) { records <- record }})
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	metrics, err := runtime.MetricsSnapshot()
	if err != nil || !metrics.LoggerAvailable {
		t.Fatalf("logger metrics unavailable: %+v, %v", metrics, err)
	}
	deadline := time.Now().Add(time.Second)
	for time.Now().Before(deadline) {
		if _, err := runtime.Poll(4, 5*time.Millisecond); err != nil {
			t.Fatal(err)
		}
		select {
		case record := <-records:
			if record.EventName != "game_runtime_started" {
				t.Fatalf("unexpected record: %+v", record)
			}
			return
		default:
		}
	}
	t.Fatal("game logger callback not invoked")
}

func TestGameLoggerPanicIsCountedWithoutKillingRuntime(t *testing.T) {
	runtime, err := NewGameRuntime(nil, GameConfig{Logger: func(LogRecord) { panic("sink failure") }})
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	deadline := time.Now().Add(time.Second)
	for time.Now().Before(deadline) {
		if _, err := runtime.Poll(4, 5*time.Millisecond); err != nil {
			t.Fatal(err)
		}
		metrics, err := runtime.MetricsSnapshot()
		if err != nil {
			t.Fatal(err)
		}
		if metrics.Logger.SinkPanics >= 1 {
			prometheus, err := runtime.PrometheusSnapshot()
			if err != nil || !strings.Contains(prometheus, "rnet_game_go_logger_panics_total 1") {
				t.Fatalf("Go logger panic metric unavailable: %v, %v", prometheus, err)
			}
			return
		}
	}
	t.Fatal("game logger panic was not counted")
}

func TestGamePlaintextRequiresExplicitServerAndRuntimeOptIn(t *testing.T) {
	key, err := GenerateKeypair()
	if err != nil {
		t.Fatal(err)
	}
	server := GameServerConfig{
		Transport: TransportTCP, Host: "127.0.0.1", Keypair: key,
		InitialSecurity: SecurityPlaintext, Protocol: GameProtocol{ID: 92, Version: 1},
	}
	secureRuntime, err := NewGameRuntime(nil)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := secureRuntime.Listen(server); err == nil {
		t.Fatal("plaintext listener bypassed production policy")
	}
	if err := secureRuntime.Close(); err != nil {
		t.Fatal(err)
	}
	network := DefaultConfig()
	network.MaxEndpoints = 1
	compatibleRuntime, err := NewGameRuntime(nil, GameConfig{
		AllowPlaintextBusinessData: true, Network: &network,
	})
	if err != nil {
		t.Fatal(err)
	}
	defer compatibleRuntime.Close()
	if _, err := compatibleRuntime.Listen(server); err != nil {
		t.Fatal(err)
	}
	if _, err := compatibleRuntime.Listen(server); err == nil {
		t.Fatal("game network endpoint limit was not applied")
	}
}

func TestGameNetworkConfigRejectsTransportLogger(t *testing.T) {
	network := DefaultConfig()
	network.Logger = func(LogRecord) {}
	if _, err := NewGameRuntime(nil, GameConfig{Network: &network}); err == nil {
		t.Fatal("transport logger was silently ignored by game runtime")
	}
}
