namespace RNet;

/// <summary>Endpoint transport selected at Listen/Connect, not on each send.</summary>
public enum Transport : uint {
    /// <summary>Reliable ordered TCP stream with library message framing.</summary>
    Tcp = 1,
    /// <summary>Unreliable UDP messages; no retransmission guarantee.</summary>
    Udp = 2,
    /// <summary>Reliable ordered KCP delivery over UDP.</summary>
    Kcp = 3
}
/// <summary>Minimum-inclusive logger severity.</summary>
public enum LogLevel : uint {
    /// <summary>Most verbose diagnostics.</summary>
    Trace,
    /// <summary>Development diagnostics.</summary>
    Debug,
    /// <summary>Lifecycle and periodic summaries.</summary>
    Info,
    /// <summary>Anomalous or recoverable conditions.</summary>
    Warn,
    /// <summary>Local operation or endpoint failures.</summary>
    Error
}
/// <summary>Weighted local game queue priority; not a remote dispatch guarantee.</summary>
public enum GamePriority : uint {
    /// <summary>Lowest local scheduler share.</summary>
    Low = 1,
    /// <summary>Default ordinary-message priority.</summary>
    Normal,
    /// <summary>Higher scheduler share without a deadline guarantee.</summary>
    High,
    /// <summary>Highest share; lower priority queues still receive service.</summary>
    Critical
}

/// <summary>Native failure with an owned diagnostic message and machine-readable status code.</summary>
public sealed class RNetException : Exception {
    /// <summary>Original negative native status, e.g. -4 backpressure, -11 not ready. Do not parse Message for control
    /// flow.</summary>
    public int Code { get; }
    /// <summary>Creates an exception preserving the native error category and diagnostic.</summary>
    /// <param name="code">Native status, not a peer business error.</param>
    /// <param name="message">Copied diagnostic text.</param>
    public RNetException(int code, string message) : base(message) { Code = code; }
}

/// <summary>Creation-time policies and budgets, copied by GameRuntime. Later edits do not reconfigure it.</summary>
/// <remarks>Nullable fields keep native defaults when null; nonnullable zero fallback is documented per field.
/// Invalid settings throw RNetException. Queue admission failures use code -4.</remarks>
public sealed class GameOptions {
    /// <summary>Permits local server plaintext business mode; false by default. Clients follow server policy
    /// independently. Plaintext has no confidentiality or integrity.</summary>
    public bool AllowPlaintextBusinessData { get; init; }
    /// <summary>Probe interval in milliseconds. Both heartbeat values zero select 5000/15000 ms defaults; otherwise
    /// both must be positive.</summary>
    public uint HeartbeatIntervalMs { get; init; }
    /// <summary>Acknowledgement timeout in milliseconds; explicit value must be at least HeartbeatIntervalMs.</summary>
    public uint HeartbeatTimeoutMs { get; init; }
    /// <summary>Positive I/O worker count; null keeps native defaults.</summary>
    public uint? WorkerThreads { get; init; }
    /// <summary>Positive transport event-count limit including lifecycle events; null keeps native defaults.</summary>
    public uint? EventQueueCapacity { get; init; }
    /// <summary>Positive outbound command-count capacity; null keeps native defaults.</summary>
    public uint? WriteQueueCapacity { get; init; }
    /// <summary>Positive logical body byte limit, including game-envelope overhead; null keeps native
    /// defaults.</summary>
    public uint? MaxBodyBytes { get; init; }
    /// <summary>UDP/KCP datagram byte ceiling including protocol overhead; 20..65507. Null keeps the native 1200-byte
    /// default.</summary>
    public uint? MaxDatagramBytes { get; init; }
    /// <summary>Positive runtime transport-event payload byte budget; null keeps native defaults.</summary>
    public ulong? MaxEventBytes { get; init; }
    /// <summary>Positive total transport-send byte budget, including framing reservations; null keeps native
    /// defaults.</summary>
    public ulong? MaxQueuedBytes { get; init; }
    /// <summary>Positive per-session transport-send byte budget, at most MaxQueuedBytes; null keeps native
    /// defaults.</summary>
    public ulong? MaxSessionQueuedBytes { get; init; }
    /// <summary>Positive runtime cap on listeners plus outbound endpoints; null keeps native defaults.</summary>
    public uint? MaxEndpoints { get; init; }
    /// <summary>Positive runtime-wide pending handshake/authorization cap; null keeps native defaults.</summary>
    public uint? MaxPendingHandshakes { get; init; }
    /// <summary>Positive cap on active and pending peers per endpoint; null keeps native defaults.</summary>
    public uint? MaxSessionsPerEndpoint { get; init; }
    /// <summary>Positive admission cap per IPv4 address or native-configured IPv6 prefix; null keeps native
    /// defaults.</summary>
    public uint? MaxSessionsPerIp { get; init; }
    /// <summary>Positive TCP connect-attempt timeout in milliseconds; null keeps native defaults.</summary>
    public ulong? ConnectTimeoutMs { get; init; }
    /// <summary>Positive cryptographic handshake/application authorization deadline in milliseconds; null keeps native
    /// defaults.</summary>
    public ulong? HandshakeTimeoutMs { get; init; }
    /// <summary>Positive synchronous name-resolution wait limit in milliseconds; null keeps native defaults.</summary>
    public ulong? DnsTimeoutMs { get; init; }
    /// <summary>Positive established UDP/KCP idle timeout in milliseconds; null keeps native defaults.</summary>
    public ulong? DatagramIdleTimeoutMs { get; init; }
    /// <summary>Server automatic key-rotation age in milliseconds; zero disables this trigger, null retains native
    /// defaults.</summary>
    public ulong? RekeyAfterMs { get; init; }
    /// <summary>Server protected-byte rotation threshold; zero disables this trigger, null retains native
    /// defaults.</summary>
    public ulong? RekeyAfterBytes { get; init; }
    /// <summary>Controls TCP_NODELAY for new sockets; true by default.</summary>
    public bool TcpNoDelay { get; init; } = true;
    /// <summary>Requested TCP kernel send-buffer bytes; zero retains OS defaults, actual allocation is
    /// OS-dependent.</summary>
    public ulong TcpSendBufferBytes { get; init; }
    /// <summary>Requested TCP kernel receive-buffer bytes; zero retains OS defaults.</summary>
    public ulong TcpReceiveBufferBytes { get; init; }
    /// <summary>Runtime-wide snapshot staging bytes; zero selects 16 MiB.</summary>
    public ulong RealtimeMaxBytes { get; init; }
    /// <summary>Per-session snapshot staging bytes; zero selects 256 KiB, no greater than the total budget.</summary>
    public ulong RealtimeSessionMaxBytes { get; init; }
    /// <summary>Distinct pending snapshot keys per session; zero selects 64.</summary>
    public ulong RealtimeMaxKeys { get; init; }
    /// <summary>Snapshot forwarding attempts per Poll; zero selects 256.</summary>
    public ulong RealtimeFlushBatch { get; init; }
    /// <summary>Runtime-wide ordinary/priority message staging bytes; zero selects 16 MiB.</summary>
    public ulong ScheduledMaxBytes { get; init; }
    /// <summary>Per-session ordinary/priority staging bytes; zero selects 256 KiB, no greater than the total.</summary>
    public ulong ScheduledSessionMaxBytes { get; init; }
    /// <summary>Ordinary/priority staging message-count cap; zero selects 65536.</summary>
    public ulong ScheduledMaxMessages { get; init; }
    /// <summary>Ordinary/priority forwarding attempts per Poll; zero selects 256.</summary>
    public ulong ScheduledFlushBatch { get; init; }
    /// <summary>Runs on the bounded native logging thread. Return promptly; never call Stop/Dispose.
    /// Exceptions are caught and counted in LoggerExceptions. Metrics queries are allowed.</summary>
    public Action<LogRecord>? Logger { get; init; }
    /// <summary>Minimum inclusive callback severity; defaults to Info. Values outside the enum are rejected.</summary>
    public LogLevel MinimumLogLevel { get; init; } = LogLevel.Info;
}

/// <summary>Authenticated listener policy; transport remains fixed and keys are copied by Listen.</summary>
public sealed class ServerOptions {
    /// <summary>References a caller-owned identity without transferring disposal responsibility.</summary>
    /// <param name="key">Non-null, undisposed server identity; keep stable when clients pin its public key.</param>
    public ServerOptions(Keypair key) { Key = key ?? throw new ArgumentNullException(nameof(key)); }
    /// <summary>Caller-owned server identity; keep it undisposed until Listen returns. The runtime copies the
    /// secret.</summary>
    public Keypair Key { get; }
    /// <summary>Immutable listener transport; defaults to TCP and must match the clients.</summary>
    public Transport Transport { get; init; } = Transport.Tcp;
    /// <summary>Numeric bind IPv4/IPv6 without brackets, URL scheme or port; defaults to loopback. DNS is not
    /// accepted.</summary>
    public string Host { get; init; } = "127.0.0.1";
    /// <summary>Local port; zero requests an OS-assigned port, available from LocalPort.</summary>
    public ushort Port { get; init; }
    /// <summary>Initial business-data mode; true by default. False requires GameOptions plaintext permission.</summary>
    public bool InitialEncryption { get; init; } = true;
    /// <summary>Nonzero business protocol identity; must match clients. Default 1, independent of SDK/wire
    /// version.</summary>
    public ulong ProtocolId { get; init; } = 1;
    /// <summary>Positive inclusive minimum business version; default 1. Equal bounds require an exact
    /// version.</summary>
    public uint MinVersion { get; init; } = 1;
    /// <summary>Inclusive maximum business version, at least MinVersion; default 1. Highest overlap is
    /// selected.</summary>
    public uint MaxVersion { get; init; } = 1;
}

/// <summary>Outbound routing and business-version metadata; identity/trust come from the runtime.</summary>
public sealed class ClientOptions {
    /// <summary>Immutable connection transport, matching the listener; defaults to TCP.</summary>
    public Transport Transport { get; init; } = Transport.Tcp;
    /// <summary>DNS name or numeric IP without URL scheme/path or embedded port; defaults to localhost.</summary>
    public string Host { get; init; } = "localhost";
    /// <summary>Remote listening port; must be nonzero.</summary>
    public ushort Port { get; init; }
    /// <summary>Opaque protected login bytes, copied by Connect; may be empty if server authorization allows. Caller
    /// erases its copy.</summary>
    public byte[] JoinTicket { get; init; } = Array.Empty<byte>();
    /// <summary>Nonzero business protocol identity matching the server; default 1.</summary>
    public ulong ProtocolId { get; init; } = 1;
    /// <summary>Positive inclusive minimum business version; default 1. Equal bounds require an exact
    /// version.</summary>
    public uint MinVersion { get; init; } = 1;
    /// <summary>Inclusive maximum business version, at least MinVersion; default 1.</summary>
    public uint MaxVersion { get; init; } = 1;
    /// <summary>Optional authenticated client build metadata exposed to server authorization; zero is
    /// unspecified.</summary>
    public ulong BuildId { get; init; }
    /// <summary>Application-defined authenticated capability bits; zero advertises none. The library does not negotiate
    /// these bits.</summary>
    public ulong Capabilities { get; init; }
    /// <summary>Diagnostic representation that omits credential/payload contents; does not erase the stored
    /// bytes.</summary>
    public override string ToString() => $"ClientOptions({Transport}, credentials=<redacted>)";
}

/// <summary>Optional peer metadata and local scheduling; business message types belong inside the payload.</summary>
public sealed class GameSendOptions {
    /// <summary>Optional caller sequence metadata; null omits it, zero is valid. Does not enable deduplication or
    /// ordering.</summary>
    public uint? Sequence { get; init; }
    /// <summary>Optional simulation tick metadata; null omits it. Not a trusted clock.</summary>
    public uint? Tick { get; init; }
    /// <summary>Optional request/trace metadata copied to the peer; zero means absent.</summary>
    public ulong CorrelationId { get; init; }
    /// <summary>Local weighted scheduler priority; defaults to Normal. Does not reserve bandwidth or a
    /// deadline.</summary>
    public GamePriority Priority { get; init; } = GamePriority.Normal;
    /// <summary>Relative local staging expiry in milliseconds; zero means no expiry. Cannot retract transport-admitted
    /// bytes.</summary>
    public ulong ExpiryMs { get; init; }
}

/// <summary>Owned structured log record, safe to retain after the callback returns.</summary>
/// <param name="TimestampUnixMs">UTC log timestamp in milliseconds.</param>
/// <param name="Level">Record severity.</param>
/// <param name="EventName">Owned structured event name.</param>
/// <param name="Runtime">Diagnostic native runtime handle; not a managed GameRuntime object.</param>
/// <param name="Endpoint">Local endpoint, zero when unspecified.</param>
/// <param name="Session">Local session, zero when unspecified.</param>
/// <param name="Transport">Source transport; numeric zero means unspecified.</param>
/// <param name="ErrorCode">Local native status, zero when no error.</param>
/// <param name="CorrelationId">Optional trace identifier, zero when absent.</param>
/// <param name="Message">Owned diagnostic text without business payloads or credentials.</param>
public sealed record LogRecord(ulong TimestampUnixMs, LogLevel Level, string EventName,
    ulong Runtime, ulong Endpoint, ulong Session, Transport Transport, int ErrorCode,
    ulong CorrelationId, string Message);
