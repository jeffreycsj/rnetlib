using System.Diagnostics;
using System.Security.Cryptography;

namespace RNet.Quickstart;

internal sealed class EchoReport {
    internal int Received; // Verified peer echoes, not local admission successes.
    internal int Blocked;  // Flush attempts deferred by native WouldBlock on either peer.
}

internal static class GameExample {
    // transport is fixed at endpoint creation; deny is only a demo login policy.
    // oneSlot requests real scheduler admission pressure, not simulated packet loss.
    // No application workers race Stop/Dispose; only native logging runs concurrently.
    internal static EchoReport Run(Transport transport, bool deny, bool oneSlot) {
        // Production loads persistent identity and distributes the public-key pin
        // through a trusted channel, never from the unauthenticated connection.
        using var serverKey = Keypair.Generate();
        using var clientKey = Keypair.Generate();
        var options = new GameOptions {
            ScheduledMaxMessages = oneSlot ? 1UL : 0,
            Logger = record => {
                // Console.Error serializes writes. Production injects a fast sink,
                // monitors dropped logs, and never Stop/Dispose from this callback.
                // No credentials or business payloads are formatted here.
                Console.Error.WriteLine($"timestamp_ms={record.TimestampUnixMs} level={record.Level} runtime={record.Runtime} endpoint={record.Endpoint} session={record.Session} correlation={record.CorrelationId} code={record.ErrorCode} {record.EventName} {record.Message}");
            }
        };
        GameRuntime? server = null, client = null;
        Exception? primary = null;
        var report = new EchoReport();
        try {
            server = new GameRuntime(options);
            client = new GameRuntime(options, clientKey, serverKey.PublicKey);
            var listener = server.Listen(new ServerOptions(serverKey) {
                Transport = transport, ProtocolId = 42, MinVersion = 1, MaxVersion = 1
            });
            byte[] ticket = "demo"u8.ToArray();
            ulong endpoint;
            try {
                endpoint = client.Connect(new ClientOptions {
                    Transport = transport, Host = "127.0.0.1", Port = server.LocalPort(listener),
                    ProtocolId = 42, MinVersion = 1, MaxVersion = 1, JoinTicket = ticket
                });
            } finally { CryptographicOperations.ZeroMemory(ticket); }
            Exchange(server, client, listener, endpoint, deny, report);
        } catch (Exception error) { primary = error; }
        Cleanup.Finish(primary, () => CloseRuntime("client", client), () => CloseRuntime("server", server));
        return report;
    }

    // Each side owns its session handle. An endpoint/auth acceptance is not game
    // readiness. UDP retains its native loss/duplicate/reordering semantics: this
    // strict two-message echo intentionally fails if those prevent exact delivery.
    private static void Exchange(GameRuntime server, GameRuntime client, ulong listener,
        ulong endpoint, bool deny, EchoReport report) {
        long deadline = Stopwatch.GetTimestamp() + 10 * Stopwatch.Frequency;
        byte[][] expected = [[(byte)'g', 0, 255, (byte)'m'], []];
        ulong serverSession = 0, clientSession = 0;
        var serverPending = new PendingSends(); var clientPending = new PendingSends();
        while (report.Received < expected.Length && Stopwatch.GetTimestamp() < deadline) {
            foreach (var e in server.Poll(64, 1)) {
                CheckEvent(e);
                switch (e.Type) {
                    case GameEventType.AuthRequest:
                        // Demo credential only. Verify signature/expiry/audience and
                        // bind real authorization to e.ClientPublicKey in production.
                        bool accept = !deny && e.Data.AsSpan().SequenceEqual("demo"u8);
                        CryptographicOperations.ZeroMemory(e.Data);
                        server.AuthDecide(e.Session, accept);
                        break;
                    case GameEventType.SessionReady:
                        if (e.Endpoint != listener || serverSession != 0)
                            throw new InvalidOperationException("Unexpected server readiness");
                        serverSession = e.Session;
                        break;
                    case GameEventType.Message:
                        if (serverSession == 0 || e.Session != serverSession)
                            throw new InvalidOperationException("Server message before readiness");
                        serverPending.Stage(e.Data);
                        break;
                }
            }
            foreach (var e in client.Poll(64, 1)) {
                CheckEvent(e);
                switch (e.Type) {
                    case GameEventType.SessionReady:
                        if (e.Endpoint != endpoint || clientSession != 0)
                            throw new InvalidOperationException("Unexpected client readiness");
                        clientSession = e.Session;
                        foreach (var payload in expected) clientPending.Stage(payload);
                        break;
                    case GameEventType.Message:
                        if (clientSession == 0 || e.Session != clientSession || report.Received >= expected.Length ||
                            !e.Data.SequenceEqual(expected[report.Received]))
                            throw new InvalidOperationException("Binary/empty echo mismatch");
                        report.Received++;
                        break;
                }
            }
            // Send only acknowledges local admission. Retry after polls even without
            // Writable: that event alone does not represent game scheduler capacity.
            if (serverSession != 0 && serverPending.Flush(deadline, p => server.Send(serverSession, p))) report.Blocked++;
            if (clientSession != 0 && clientPending.Flush(deadline, p => client.Send(clientSession, p))) report.Blocked++;
        }
        if (report.Received != expected.Length) throw new TimeoutException("Echo deadline exceeded; UDP is not reliable");
        Cleanup.Finish(null, () => client.CloseSession(clientSession), () => server.CloseEndpoint(listener));
    }

    private static void CheckEvent(GameEvent e) {
        int code = e.Status;
        switch (e.Type) {
            // Conservative demo policy: abort on any endpoint error even if an
            // application could inspect endpoint validity and continue/reconnect.
            case GameEventType.JoinFailed:
            case GameEventType.ProtocolRejected:
            case GameEventType.SessionClosed:
            case GameEventType.EndpointError:
                if (code == 0) code = -3;
                break;
            case GameEventType.ProtocolViolation: code = -8; break; // Status may be zero.
            case GameEventType.RuntimeStopped: code = -3; break;
            default: return;
        }
        throw new RNetException(code, $"event={e.Type} endpoint={e.Endpoint} session={e.Session}; inspect lifecycle logs");
    }

    // Stop exposes native errors before SafeHandle disposal. The five-second
    // deadline bounds retry attempts, not a native call or a blocked user logger.
    // Dispose cannot report native teardown failures; do not claim otherwise.
    private static void CloseRuntime(string name, GameRuntime? runtime) {
        if (runtime == null) return;
        Exception? primary = null;
        try {
            long deadline = Stopwatch.GetTimestamp() + 5 * Stopwatch.Frequency;
            while (true) {
                try { runtime.Stop(); break; }
                catch (RNetException error) when (error.Code == -4 && Stopwatch.GetTimestamp() < deadline) {
                    runtime.Poll(64, 1); // Discard owned final events; native tokens are already released.
                    Thread.Sleep(1);
                }
            }
        } catch (Exception error) { primary = error; }
        try { Cleanup.Finish(primary, runtime.Dispose); }
        catch (Exception error) { throw new InvalidOperationException($"{name} cleanup failed", error); }
    }
}
