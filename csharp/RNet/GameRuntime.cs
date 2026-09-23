using System.Runtime.InteropServices;
using System.Security.Cryptography;

namespace RNet;

/// <summary>One runtime can listen and connect using TCP, UDP and KCP. Poll continuously to drive
/// game controls and queues. Safe to send/query concurrently; polls are serialized natively.
/// Dispose releases native resources after in-flight calls return; logger sinks must return promptly.</summary>
public sealed unsafe partial class GameRuntime : IDisposable {
    private readonly RuntimeOwner owner;
    private readonly LogState? logState;
    public long LoggerExceptions => logState == null ? 0 : Interlocked.Read(ref logState.Exceptions);

    public GameRuntime(GameOptions? options = null, Keypair? clientKey = null, byte[]? expectedServerKey = null) {
        if (IntPtr.Size != 8) throw new PlatformNotSupportedException("This SDK currently validates the 64-bit native ABI only.");
        if (Native.rnet_abi_version() != 1) throw new NotSupportedException("Unsupported RNet ABI version");
        if ((clientKey == null) != (expectedServerKey == null)) throw new ArgumentException("Client identity and server pin must be supplied together.");
        options ??= new GameOptions();
        if (options.MinimumLogLevel > LogLevel.Error) throw new ArgumentOutOfRangeException(nameof(options));
        Native.GameConfig game = default;
        Native.NetworkConfig network = default;
        Native.Check(Native.rnet_game_config_v2_init(&game));
        Native.Check(Native.rnet_config_v5_init(&network));
        Configure(options, ref game, ref network);
        game.Network = &network;
        byte[] privateKey = clientKey?.ExportPrivateKey() ?? Array.Empty<byte>();
        nint context = 0;
        try {
            fixed (byte* key = privateKey, expected = expectedServerKey) {
                var security = new Native.Security { Size = (uint)sizeof(Native.Security), Abi = 1,
                    PrivateKey = new Native.Slice(key, privateKey.Length),
                    ExpectedKey = new Native.Slice(expected, expectedServerKey?.Length ?? 0) };
                ulong runtime;
                if (options.Logger != null) {
                    logState = new LogState(options.Logger);
                    context = GCHandle.ToIntPtr(GCHandle.Alloc(logState));
                    var logger = new Native.Logger { Size = (uint)sizeof(Native.Logger), Abi = 1,
                        Callback = Marshal.GetFunctionPointerForDelegate(LogBridge.Function),
                        UserData = context, MinLevel = (uint)options.MinimumLogLevel };
                    Native.Check(Native.rnet_game_runtime_create_logged_v2(&game, clientKey == null ? null : &security, &logger, out runtime));
                } else Native.Check(Native.rnet_game_runtime_create_v2(&game, clientKey == null ? null : &security, out runtime));
                owner = new RuntimeOwner(runtime, context);
                context = 0; // only native destroy may now release callback user data
            }
        } finally {
            CryptographicOperations.ZeroMemory(privateKey);
            if (context != 0) GCHandle.FromIntPtr(context).Free();
        }
    }

    private static void Configure(GameOptions o, ref Native.GameConfig g, ref Native.NetworkConfig n) {
        g.HeartbeatIntervalMs = o.HeartbeatIntervalMs; g.HeartbeatTimeoutMs = o.HeartbeatTimeoutMs;
        g.AllowPlaintext = o.AllowPlaintextBusinessData ? 1u : 0u;
        g.RealtimeBytes = o.RealtimeMaxBytes; g.RealtimeSessionBytes = o.RealtimeSessionMaxBytes;
        g.RealtimeKeys = o.RealtimeMaxKeys; g.RealtimeBatch = o.RealtimeFlushBatch;
        g.ScheduledBytes = o.ScheduledMaxBytes; g.ScheduledSessionBytes = o.ScheduledSessionMaxBytes;
        g.ScheduledMessages = o.ScheduledMaxMessages; g.ScheduledBatch = o.ScheduledFlushBatch;
        n.Workers = o.WorkerThreads ?? n.Workers; n.EventCapacity = o.EventQueueCapacity ?? n.EventCapacity;
        n.WriteCapacity = o.WriteQueueCapacity ?? n.WriteCapacity; n.MaxBody = o.MaxBodyBytes ?? n.MaxBody;
        n.MaxDatagram = o.MaxDatagramBytes ?? n.MaxDatagram; n.MaxEventBytes = o.MaxEventBytes ?? n.MaxEventBytes;
        n.MaxRuntimeBytes = o.MaxQueuedBytes ?? n.MaxRuntimeBytes; n.MaxSessionBytes = o.MaxSessionQueuedBytes ?? n.MaxSessionBytes;
        n.MaxEndpoints = o.MaxEndpoints ?? n.MaxEndpoints; n.MaxPendingHandshakes = o.MaxPendingHandshakes ?? n.MaxPendingHandshakes;
        n.MaxSessions = o.MaxSessionsPerEndpoint ?? n.MaxSessions; n.MaxSessionsPerIp = o.MaxSessionsPerIp ?? n.MaxSessionsPerIp;
        n.ConnectTimeoutMs = o.ConnectTimeoutMs ?? n.ConnectTimeoutMs; n.HandshakeTimeoutMs = o.HandshakeTimeoutMs ?? n.HandshakeTimeoutMs;
        n.DnsTimeoutMs = o.DnsTimeoutMs ?? n.DnsTimeoutMs; n.IdleTimeoutMs = o.DatagramIdleTimeoutMs ?? n.IdleTimeoutMs;
        n.RekeyAfterMs = o.RekeyAfterMs ?? n.RekeyAfterMs; n.RekeyAfterBytes = o.RekeyAfterBytes ?? n.RekeyAfterBytes;
        n.TcpNoDelay = o.TcpNoDelay ? 1u : 0u; n.TcpSendBuffer = o.TcpSendBufferBytes; n.TcpReceiveBuffer = o.TcpReceiveBufferBytes;
    }

    private RuntimeLease Lease() => new(owner);
    public void AuthDecide(ulong session, bool accept) { using var h = Lease(); Native.Check(Native.rnet_game_auth_decide(h.Value, session, accept ? 1u : 0u)); }
    public void CloseSession(ulong session) { using var h = Lease(); Native.Check(Native.rnet_game_session_close(h.Value, session)); }
    public void CloseEndpoint(ulong endpoint) { using var h = Lease(); Native.Check(Native.rnet_game_endpoint_close(h.Value, endpoint)); }
    /// <summary>Server-only; wait for SecurityChanged. Controls stay authenticated in plaintext mode.</summary>
    public void SetEncryption(ulong session, bool encrypted) { using var h = Lease(); Native.Check(Native.rnet_game_security_set(h.Value, session, encrypted ? 1u : 0u)); }
    public void Rekey(ulong session) { using var h = Lease(); Native.Check(Native.rnet_game_rekey(h.Value, session)); }
    public ushort LocalPort(ulong endpoint) { using var h = Lease(); Native.Check(Native.rnet_game_endpoint_local_port(h.Value, endpoint, out var port)); return port; }
    public uint SelectedProtocolVersion(ulong session) { using var h = Lease(); Native.Check(Native.rnet_game_selected_protocol_version(h.Value, session, out var version)); return version; }
    public void SetMetricsLogInterval(ulong intervalMs) { using var h = Lease(); Native.Check(Native.rnet_game_metrics_log_interval_set(h.Value, intervalMs)); }
    /// <summary>Stops admission and drains within the deadline. Code -4 requires polling and retry.
    /// Dispose always performs final zero-timeout stop and drops remaining undelivered events.</summary>
    public void Stop(uint drainTimeoutMs = 0) {
        RejectCallbackShutdown(); using var h = Lease(); Native.Check(Native.rnet_game_runtime_stop(h.Value, drainTimeoutMs));
    }
    public void Dispose() { RejectCallbackShutdown(); owner.Dispose(); }
    private static void RejectCallbackShutdown() {
        if (LogBridge.InCallback) throw new InvalidOperationException("Stop/Dispose cannot run in a native logger callback.");
    }
}
