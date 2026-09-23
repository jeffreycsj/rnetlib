using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text;
using RNet;

static void Require(bool value, string message) { if (!value) throw new Exception(message); }
static void Pump(GameRuntime server, GameRuntime client, Func<bool> done,
    Action<GameEvent>? onServer = null, Action<GameEvent>? onClient = null) {
    var deadline = Stopwatch.StartNew();
    while (!done() && deadline.Elapsed < TimeSpan.FromSeconds(10)) {
        foreach (var e in server.Poll(64, 1)) onServer?.Invoke(e);
        foreach (var e in client.Poll(64, 1)) onClient?.Invoke(e);
    }
    Require(done(), "event deadline exceeded");
}

foreach (var transport in new[] { Transport.Tcp, Transport.Udp, Transport.Kcp }) {
    using var serverKey = Keypair.Generate();
    using var clientKey = Keypair.Generate();
    var records = new ConcurrentQueue<LogRecord>();
    GameRuntime? queryRuntime = null;
    using var server = new GameRuntime(new GameOptions {
        AllowPlaintextBusinessData = true, HeartbeatIntervalMs = 20, HeartbeatTimeoutMs = 2000,
        Logger = record => { records.Enqueue(record); if (queryRuntime != null) _ = queryRuntime.PrometheusSnapshot(); }
    });
    queryRuntime = server;
    using var client = new GameRuntime(new GameOptions {
        AllowPlaintextBusinessData = true, HeartbeatIntervalMs = 20, HeartbeatTimeoutMs = 2000
    }, clientKey, serverKey.PublicKey);
    server.SetMetricsLogInterval(1);
    var listener = server.Listen(new ServerOptions(serverKey) { Transport = transport, ProtocolId = 42, Version = 3 });
    var connector = client.Connect(new ClientOptions { Transport = transport, Host = "localhost",
        Port = server.LocalPort(listener), ProtocolId = 42, Version = 3, JoinTicket = Encoding.UTF8.GetBytes("secret-ticket") });
    ulong ss = 0, cs = 0;
    Pump(server, client, () => ss != 0 && cs != 0, e => {
        if (e.Type == GameEventType.AuthRequest) {
            Require(Encoding.UTF8.GetString(e.Data) == "secret-ticket", "authorization bytes");
            Require(!e.ToString()!.Contains("secret-ticket"), "credential redaction");
            server.AuthDecide(e.Session, true);
        }
        if (e.Type == GameEventType.SessionReady) ss = e.Session;
    }, e => { if (e.Type == GameEventType.SessionReady) cs = e.Session; });
    Require(connector != 0, "client endpoint");
    GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();
    foreach (bool encrypted in new[] { false, true, false }) {
        bool serverChanged = false, clientChanged = false;
        server.SetEncryption(ss, encrypted);
        Pump(server, client, () => serverChanged && clientChanged,
            e => { if (e.Type == GameEventType.SecurityChanged) serverChanged = e.Encrypted == encrypted; },
            e => { if (e.Type == GameEventType.SecurityChanged) clientChanged = e.Encrypted == encrypted; });
        var payload = new byte[] { 0, 255, 9, 0, 128 };
        client.Send(cs, payload);
        bool received = false;
        Pump(server, client, () => received,
            e => { if (e.Type == GameEventType.Message) server.Send(e.Session, e.Data); },
            e => { if (e.Type == GameEventType.Message) { Require(e.Data.SequenceEqual(payload), "opaque echo"); received = true; } });
    }
    server.SendLatest(ss, 7, new byte[] { 1 });
    server.SendLatest(ss, 7, new byte[] { 2 });
    bool latest = false;
    Pump(server, client, () => latest, null, e => {
        if (e.Type == GameEventType.Message) { Require(e.Data.SequenceEqual(new byte[] { 2 }), "latest coalescing"); latest = true; }
    });
    try { client.SetEncryption(cs, true); throw new Exception("client changed server policy"); }
    catch (RNetException error) { Require(error.Code == -3, "client must not control encryption"); }
    bool rekeyed = false;
    server.Rekey(ss);
    Pump(server, client, () => rekeyed, null, e => { if (e.Type == GameEventType.SecurityChanged && e.SecurityOperation == 2) rekeyed = true; });
    bool advanced = false;
    client.Send(cs, Array.Empty<byte>(), new GameSendOptions { Tick = uint.MaxValue, Sequence = 17, CorrelationId = 991, Priority = GamePriority.High });
    Pump(server, client, () => advanced, e => { if (e.Type == GameEventType.Message) {
        Require(e.Data.Length == 0 && e.Tick == uint.MaxValue && e.Sequence == 17 && e.CorrelationId == 991, "advanced metadata ABI"); advanced = true;
    } });
    Require(server.MetricsSnapshot().LoggerAvailable, "typed metrics ABI");
    Require(client.ClockMicros() > 0, "runtime clock ABI");
    Pump(server, client, () => client.ClockSyncSnapshot(cs) != null && client.QualitySnapshot(cs) != null);
    Require(client.ClockSyncSnapshot(cs)!.Samples > 0 && client.QualitySnapshot(cs)!.Samples > 0, "typed sampled telemetry ABI");
    Require(server.ClockSyncSnapshot(ss) == null, "server clock has no client-side sample");
    byte[]? ticket = null;
    server.IssueResumeTicket(ss, Encoding.UTF8.GetBytes("secret-identity"));
    Pump(server, client, () => ticket != null, null, e => { if (e.Type == GameEventType.ResumeTicket) ticket = e.Data; });
    var oldServer = ss; var oldClient = cs;
    client.ConnectResume(new ClientOptions { Transport = transport, Port = server.LocalPort(listener), ProtocolId = 42, Version = 3 }, cs, ticket!);
    Pump(server, client, () => ss != oldServer && cs != oldClient, e => {
        if (e.Type == GameEventType.ResumeRequest) {
            Require(e.RelatedSession == oldServer && Encoding.UTF8.GetString(e.Data) == "secret-identity", "resume identity/mapping");
            server.AuthDecide(e.Session, true);
        }
        if (e.Type == GameEventType.SessionResumed) ss = e.Session;
    }, e => { if (e.Type == GameEventType.SessionResumed) { Require(e.RelatedSession == oldClient, "client mapping"); cs = e.Session; } });
    var rangeListener = server.ListenRange(new ServerOptions(serverKey) { Transport = transport, ProtocolId = 84 }, 1, 5);
    client.ConnectRange(new ClientOptions { Transport = transport, Port = server.LocalPort(rangeListener), ProtocolId = 84 }, 2, 4);
    bool rangeReady = false;
    Pump(server, client, () => rangeReady, e => {
        if (e.Type == GameEventType.AuthRequest) { Require(server.SelectedProtocolVersion(e.Session) == 4, "server selected range"); server.AuthDecide(e.Session, true); }
    }, e => { if (e.Type == GameEventType.SessionReady) { Require(client.SelectedProtocolVersion(e.Session) == 4, "client selected range"); rangeReady = true; } });
    server.Poll(0, 0); // maintenance without borrowing event buffers
    Require(server.PrometheusSnapshot().Contains("rnet_game_logger_enabled 1"), "metrics");
    Require(SpinWait.SpinUntil(() => records.Any(r => r.EventName == "game_latency_summary" && r.Message.Contains("p99_us=")), 2000), "custom logger summary");
    Require(records.All(r => !r.Message.Contains("secret-ticket")), "log credential redaction");
    try { client.Send(0, new byte[] { 1 }); throw new Exception("invalid handle accepted"); }
    catch (RNetException error) { Require(error.Code != 0 && !string.IsNullOrWhiteSpace(error.Message), "detailed native error"); }
    queryRuntime = null;
    server.CloseSession(ss);
    server.CloseEndpoint(listener);
    client.Dispose(); client.Dispose();
    try { client.Poll(); throw new Exception("disposed runtime accepted"); } catch (ObjectDisposedException) { }
    Console.WriteLine($"PASS {transport}: auth, echo, dynamic encryption/rekey, latest, metadata, resume/range, metrics, GC/logger, errors, dispose");
}

using (var runtime = new GameRuntime(new GameOptions { Logger = _ => throw new Exception("sink") })) {
    runtime.Poll();
    Require(SpinWait.SpinUntil(() => runtime.LoggerExceptions > 0, 2000), "callback exceptions contained");
}
Console.WriteLine("PASS logger exception containment");

GameRuntime? callbackRuntime = null;
int rejectedShutdown = 0;
using (var runtime = new GameRuntime(new GameOptions { Logger = record => {
    try { callbackRuntime!.Dispose(); } catch (InvalidOperationException) { Interlocked.Increment(ref rejectedShutdown); }
    _ = callbackRuntime!.MetricsSnapshot();
} })) {
    callbackRuntime = runtime;
    runtime.Poll();
    Require(SpinWait.SpinUntil(() => Volatile.Read(ref rejectedShutdown) != 0, 2000), "callback dispose rejected without deadlock");
}
Console.WriteLine("PASS callback shutdown/reentrant query");

foreach (var transport in new[] { Transport.Tcp, Transport.Udp, Transport.Kcp }) {
    using var serverKey = Keypair.Generate();
    using var wrongKey = Keypair.Generate();
    using var clientKey = Keypair.Generate();
    using var server = new GameRuntime();
    using var client = new GameRuntime(null, clientKey, wrongKey.PublicKey);
    var endpoint = server.Listen(new ServerOptions(serverKey) { Transport = transport });
    client.Connect(new ClientOptions { Transport = transport, Port = server.LocalPort(endpoint) });
    bool failed = false;
    Pump(server, client, () => failed, e => {
        Require(e.Type != GameEventType.AuthRequest && e.Type != GameEventType.SessionReady, "wrong pin reached authorization");
    }, e => {
        Require(e.Type != GameEventType.SessionReady, "wrong pin became ready");
        if (e.Type == GameEventType.JoinFailed) { Require(e.Status != 0, "join failure has reason"); failed = true; }
    });
    Console.WriteLine($"PASS {transport}: server pin mismatch fails before business authorization");
}

try {
    using var invalid = new GameRuntime(new GameOptions { EventQueueCapacity = 0 });
    throw new Exception("zero event capacity accepted");
} catch (RNetException error) { Require(error.Code == -1, "zero capacity validation"); }
using (var runtime = new GameRuntime()) {
    using var entered = new ManualResetEventSlim();
    var poll = Task.Run(() => {
        entered.Set();
        try { runtime.Poll(64, 30); } catch (ObjectDisposedException) { }
    });
    entered.Wait();
    runtime.Dispose();
    Require(poll.Wait(2000), "in-flight poll must release its SafeHandle lease");
}
Console.WriteLine("PASS invalid config and concurrent disposal");
