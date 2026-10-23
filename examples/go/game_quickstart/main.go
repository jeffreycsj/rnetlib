// Command game_quickstart demonstrates two independent game runtimes on loopback.
// Build from the repository/package root; CGO links the matching lib/librnet.a.
package main

import (
	"errors"
	"fmt"
	"io"
	"log"
	"os"

	"rnet.local/netlib/go/rnet"
)

func main() { os.Exit(execute(os.Args[1:], os.Stdout, os.Stderr)) }

// execute returns a process exit status only after run has attempted both closes.
// Invalid options are rejected before creating any native resources.
func execute(args []string, stdout, stderr io.Writer) int {
	opts, err := parseOptions(args)
	if err != nil {
		fmt.Fprintf(stderr, "%v; usage: game_quickstart [tcp|udp|kcp] [--one-slot] [--deny]\n", err)
		return 2
	}
	result, err := run(opts, log.New(stderr, "", 0))
	if err != nil {
		var status rnet.StatusError
		if errors.As(err, &status) {
			fmt.Fprintf(stderr, "example code=%d: %v\n", status.Code, err)
		} else {
			fmt.Fprintf(stderr, "example: %v\n", err)
		}
		return 1
	}
	fmt.Fprintf(stdout, "game echo complete: received=%d blocked=%d; both runtimes closed\n", result.received, result.blocked)
	return 0
}

// parseOptions defaults to KCP. If any arguments are supplied, transport comes
// first; unknown and duplicate flags fail instead of silently changing policy.
func parseOptions(args []string) (options, error) {
	opts := options{transport: rnet.TransportKCP}
	if len(args) == 0 {
		return opts, nil
	}
	switch args[0] {
	case "tcp":
		opts.transport = rnet.TransportTCP
	case "udp":
		opts.transport = rnet.TransportUDP
	case "kcp":
		opts.transport = rnet.TransportKCP
	default:
		return opts, errors.New("unknown transport")
	}
	for _, arg := range args[1:] {
		switch {
		case arg == "--deny" && !opts.deny:
			opts.deny = true
		case arg == "--one-slot" && !opts.oneSlot:
			opts.oneSlot = true
		default:
			return opts, errors.New("unknown or duplicate option")
		}
	}
	return opts, nil
}
