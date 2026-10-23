package main

import (
	"bytes"
	"errors"
	"fmt"
	"log"
	"strings"
	"testing"

	"rnet.local/netlib/go/rnet"
)

func TestGameExchangeAllTransportsAndRealBackpressure(t *testing.T) {
	for name, transport := range map[string]rnet.Transport{
		"tcp": rnet.TransportTCP, "udp": rnet.TransportUDP, "kcp": rnet.TransportKCP,
	} {
		for _, oneSlot := range []bool{false, true} {
			t.Run(fmt.Sprintf("%s/one-slot=%v", name, oneSlot), func(t *testing.T) {
				var logs bytes.Buffer
				result, err := run(options{transport: transport, oneSlot: oneSlot}, log.New(&logs, "", 0))
				if err != nil {
					t.Fatalf("exchange: %v\n%s", err, logs.String())
				}
				if result.received != 2 || (oneSlot && result.blocked == 0) {
					t.Fatalf("missing delivery/backpressure evidence: %+v", result)
				}
				if !strings.Contains(logs.String(), "runtime=") {
					t.Fatal("native logging callback not used")
				}
			})
		}
	}
}

func TestGameAuthorizationFailureIsNotTimeout(t *testing.T) {
	for _, transport := range []rnet.Transport{rnet.TransportTCP, rnet.TransportUDP, rnet.TransportKCP} {
		t.Run(fmt.Sprint(transport), func(t *testing.T) {
			var logs bytes.Buffer
			result, err := run(options{transport: transport, deny: true}, log.New(&logs, "", 0))
			var status rnet.StatusError
			if !errors.As(err, &status) || status.Code != -13 || result.received != 0 {
				t.Fatalf("expected auth rejection before any echo: %+v err=%v\n%s", result, err, logs.String())
			}
			if strings.Contains(err.Error(), "cleanup") {
				t.Fatalf("shutdown failed: %v", err)
			}
		})
	}
}
