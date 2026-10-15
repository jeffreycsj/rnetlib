using System.Runtime.InteropServices;
using System.Security.Cryptography;

namespace RNet;

/// <summary>Authenticated game client/server runtime for TCP, UDP and KCP.</summary>
/// <remarks>Both peers must poll continuously for negotiation, heartbeats, clocks and queues. Send/query calls may run
/// concurrently; native polls serialize. Never copy handles between runtimes. Native failures throw RNetException with
/// status/detail; native operations after disposal throw ObjectDisposedException. Dispose prevents new calls but
/// in-flight leases can defer native teardown beyond its return. Logger callbacks must return promptly and never call
/// Stop/Dispose. Only the current SDK/native ABI pair is accepted; this SDK validates the 64-bit layout.</remarks>
public sealed unsafe partial class GameRuntime : IDisposable {
    private readonly RuntimeOwner owner;
    private readonly LogState? logState;
    /// <summary>Cumulative managed logger exceptions caught by the bridge; zero also when no logger is
    /// installed.</summary>
    public long LoggerExceptions => logState == null ? 0 : Interlocked.Read(ref logState.Exceptions);

    /// <summary>Creates a runtime without listening or connecting.</summary>
    /// <param name="options">Creation-time settings; null uses defaults.</param>
    /// <param name="clientKey">Optional client identity copied during construction; null creates server-only
    /// usage.</param>
    /// <param name="expectedServerKey">Exactly 32 pinned server public-key bytes; must be supplied together with
    /// clientKey.</param>
    /// <remarks>Keys are copied; caller owns its originals. Logger delegate/context is retained until native teardown.
    /// No network trust bypass is implied by plaintext permission.</remarks>
    /// <exception cref="ArgumentException">Identity and pin were not supplied together, or configuration arguments are
    /// invalid.</exception>
    /// <exception cref="RNetException">Native configuration, key length or resource creation failed.</exception>
    public GameRuntime(GameOptions? options = null, Keypair? clientKey = null, byte[]? expectedServerKey = null) {
        if (IntPtr.Size != 8) throw new PlatformNotSupportedException("This SDK currently validates the 64-bit native ABI only.");
        Native.CheckAbi();
        if ((clientKey == null) != (expectedServerKey == null)) throw new ArgumentException("Client identity and server pin must be supplied together.");
        options ??= new GameOptions();
        if (options.MinimumLogLevel > LogLevel.Error) throw new ArgumentOutOfRangeException(nameof(options));
        Native.GameConfig game = default;
        Native.NetworkConfig network = default;
        Native.Check(Native.rnet_game_config_init(&game));
        Native.Check(Native.rnet_config_init(&network));
        Configure(options, ref game, ref network);
        game.Network = &network;
        byte[] privateKey = clientKey?.ExportPrivateKey() ?? Array.Empty<byte>();
        nint context = 0;
        try {
            fixed (byte* key = privateKey, expected = expectedServerKey) {
                var security = new Native.Security { Size = (uint)sizeof(Native.Security), Abi = Native.AbiVersion,
                    PrivateKey = new Native.Slice(key, privateKey.Length),
                    ExpectedKey = new Native.Slice(expected, expectedServerKey?.Length ?? 0) };
                ulong runtime;
                if (options.Logger != null) {
                    logState = new LogState(options.Logger);
                    context = GCHandle.ToIntPtr(GCHandle.Alloc(logState));
                    var logger = new Native.Logger { Size = (uint)sizeof(Native.Logger), Abi = Native.AbiVersion,
                        Callback = Marshal.GetFunctionPointerForDelegate(LogBridge.Function),
                        UserData = context, MinLevel = (uint)options.MinimumLogLevel };
                    game.Logger = &logger;
                    Native.Check(Native.rnet_game_runtime_create(&game, clientKey == null ? null : &security, out runtime));
                } else Native.Check(Native.rnet_game_runtime_create(&game, clientKey == null ? null : &security, out runtime));
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
    /// <summary>Queues server authorization for AuthRequest/ResumeRequest; acceptance is not game readiness.</summary>
    /// <param name="session">Pending local server session from the request.</param>
    /// <param name="accept">True authorizes, false rejects.</param>
    /// <remarks>Wait for SessionReady/SessionResumed before sending business data.</remarks>
    public void AuthDecide(ulong session, bool accept) { using var h = Lease(); Native.Check(Native.rnet_game_auth_decide(h.Value, session, accept ? 1u : 0u)); }
    /// <summary>Closes a peer, drops staged work and revokes its resume state; listener remains open.</summary>
    /// <param name="session">Local session owned by this runtime.</param>
    public void CloseSession(ulong session) { using var h = Lease(); Native.Check(Native.rnet_game_session_close(h.Value, session)); }
    /// <summary>Closes an endpoint and its sessions; runtime remains usable.</summary>
    /// <param name="endpoint">Local listener or outbound endpoint, not a session handle.</param>
    public void CloseEndpoint(ulong endpoint) { using var h = Lease(); Native.Check(Native.rnet_game_endpoint_close(h.Value, endpoint)); }
    /// <summary>Queues a server-led business encryption change; clients follow automatically.</summary>
    /// <param name="session">Established local server-side session.</param>
    /// <param name="encrypted">False requires AllowPlaintextBusinessData; plaintext lacks confidentiality and
    /// integrity.</param>
    /// <remarks>Controls stay protected. SecurityChanged reports local commit, not peer completion. Requesting an
    /// already active mode may succeed without another event.</remarks>
    public void SetEncryption(ulong session, bool encrypted) { using var h = Lease(); Native.Check(Native.rnet_game_security_set(h.Value, session, encrypted ? 1u : 0u)); }
    /// <summary>Queues server key rotation without changing business encryption policy.</summary>
    /// <param name="session">Established local server-side session.</param>
    /// <remarks>Wait for SecurityChanged with SecurityOperation=2 for local commit; success only admits the
    /// request.</remarks>
    public void Rekey(ulong session) { using var h = Lease(); Native.Check(Native.rnet_game_rekey(h.Value, session)); }
    /// <summary>Queries a live endpoint's bound port.</summary>
    /// <param name="endpoint">Local endpoint returned by Listen/Connect.</param>
    /// <returns>The actual port, including the OS-selected port for Listen with Port=0.</returns>
    public ushort LocalPort(ulong endpoint) { using var h = Lease(); Native.Check(Native.rnet_game_endpoint_local_port(h.Value, endpoint, out var port)); return port; }
    /// <summary>Queries the selected business version; selection may precede game readiness.</summary>
    /// <param name="session">Local session with a selected version.</param>
    /// <returns>Selected version; native failure is thrown if unavailable.</returns>
    public uint SelectedProtocolVersion(ulong session) { using var h = Lease(); Native.Check(Native.rnet_game_selected_protocol_version(h.Value, session, out var version)); return version; }
    /// <summary>Changes the poll-driven cumulative latency-summary log interval.</summary>
    /// <param name="intervalMs">Milliseconds; default 30000, zero disables summaries.</param>
    public void SetMetricsLogInterval(ulong intervalMs) { using var h = Lease(); Native.Check(Native.rnet_game_metrics_log_interval_set(h.Value, intervalMs)); }
    /// <summary>Stops admission/I/O with a transport send-drain budget; retains handle for final polling.</summary>
    /// <param name="drainTimeoutMs">Milliseconds for send draining; zero skips draining. Not a hard upper bound on
    /// total call duration.</param>
    /// <remarks>Code -4 requires polling pending events then retrying. Do not call from a logger callback. Use Dispose
    /// for final resource cleanup; stopped runtimes cannot restart.</remarks>
    public void Stop(uint drainTimeoutMs = 0) {
        RejectCallbackShutdown(); using var h = Lease(); Native.Check(Native.rnet_game_runtime_stop(h.Value, drainTimeoutMs));
    }
    /// <summary>Closes the managed handle; native cleanup follows once in-flight leases finish.</summary>
    /// <remarks>Idempotent outside callbacks. Remaining events may be discarded. Native cleanup failures are not
    /// surfaced as RNetException by SafeHandle; use Stop to observe stop errors. Concurrent calls may delay native
    /// teardown/callback completion beyond this return.</remarks>
    /// <exception cref="InvalidOperationException">Called from a native logger callback.</exception>
    public void Dispose() { RejectCallbackShutdown(); owner.Dispose(); }
    private static void RejectCallbackShutdown() {
        if (LogBridge.InCallback) throw new InvalidOperationException("Stop/Dispose cannot run in a native logger callback.");
    }
}
