package main

import (
	"bytes"
	"errors"
	"strings"
	"testing"

	"rnet.local/netlib/go/rnet"
)

func TestCommandSuccessAndFailure(t *testing.T) {
	for _, test := range []struct {
		args   []string
		code   int
		output string
	}{
		{nil, 0, "game echo complete: received=2"},
		{[]string{"tcp", "--one-slot"}, 0, "both runtimes closed"},
		{[]string{"kcp", "--deny"}, 1, "code=-13"},
		{[]string{"unknown"}, 2, "usage:"},
		{[]string{"--one-slot"}, 2, "usage:"},
		{[]string{"tcp", "--bad"}, 2, "usage:"},
		{[]string{"udp", "--deny", "--deny"}, 2, "usage:"},
		{[]string{"kcp", "--one-slot", "--one-slot"}, 2, "usage:"},
	} {
		t.Run(strings.Join(test.args, " "), func(t *testing.T) {
			var out, diagnostic bytes.Buffer
			code := execute(test.args, &out, &diagnostic)
			if code != test.code {
				t.Fatalf("exit=%d want=%d stderr=%s", code, test.code, diagnostic.String())
			}
			text := out.String()
			if code != 0 {
				if out.Len() != 0 {
					t.Fatalf("failure printed success: %s", out.String())
				}
				text = diagnostic.String()
			}
			if !strings.Contains(text, test.output) {
				t.Fatalf("missing %q in %s", test.output, text)
			}
		})
	}
}

func TestFailureEventsPreserveCategories(t *testing.T) {
	for _, test := range []struct {
		kind         rnet.GameEventType
		status, want int32
	}{
		{rnet.GameJoinFailed, -13, -13}, {rnet.GameProtocolRejected, -8, -8},
		{rnet.GameEndpointError, -7, -7}, {rnet.GameSessionClosed, 0, -3},
		{rnet.GameProtocolViolation, 0, -8}, {rnet.GameRuntimeStopped, 0, -3},
	} {
		err := eventFailure(rnet.GameEvent{Type: test.kind, Status: test.status, Endpoint: 8, Session: 9})
		var status rnet.StatusError
		if !errors.As(err, &status) || status.Code != test.want || !strings.Contains(err.Error(), "session=9") {
			t.Fatalf("event=%d: %v", test.kind, err)
		}
	}
	if err := eventFailure(rnet.GameEvent{Type: rnet.GameWritable}); err != nil {
		t.Fatal(err)
	}
}
