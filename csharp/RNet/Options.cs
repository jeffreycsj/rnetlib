namespace RNet;

public enum Transport : uint { Tcp = 1, Udp = 2, Kcp = 3 }
public enum LogLevel : uint { Trace, Debug, Info, Warn, Error }
public enum GamePriority : uint { Low = 1, Normal, High, Critical }

public sealed class RNetException : Exception {
    public int Code { get; }
    public RNetException(int code, string message) : base(message) { Code = code; }
}

/// <summary>Creation-time budgets. Null uses native production defaults. Queue-full sends throw code -4.</summary>
public sealed class GameOptions {
    public bool AllowPlaintextBusinessData { get; init; }
    public uint HeartbeatIntervalMs { get; init; }
    public uint HeartbeatTimeoutMs { get; init; }
    public uint? WorkerThreads { get; init; }
    public uint? EventQueueCapacity { get; init; }
    public uint? WriteQueueCapacity { get; init; }
    public uint? MaxBodyBytes { get; init; }
    public uint? MaxDatagramBytes { get; init; }
    public ulong? MaxEventBytes { get; init; }
    public ulong? MaxQueuedBytes { get; init; }
    public ulong? MaxSessionQueuedBytes { get; init; }
    public uint? MaxEndpoints { get; init; }
    public uint? MaxPendingHandshakes { get; init; }
    public uint? MaxSessionsPerEndpoint { get; init; }
    public uint? MaxSessionsPerIp { get; init; }
    public ulong? ConnectTimeoutMs { get; init; }
    public ulong? HandshakeTimeoutMs { get; init; }
    public ulong? DnsTimeoutMs { get; init; }
    public ulong? DatagramIdleTimeoutMs { get; init; }
    public ulong? RekeyAfterMs { get; init; }
    public ulong? RekeyAfterBytes { get; init; }
    public bool TcpNoDelay { get; init; } = true;
    public ulong TcpSendBufferBytes { get; init; }
    public ulong TcpReceiveBufferBytes { get; init; }
    public ulong RealtimeMaxBytes { get; init; }
    public ulong RealtimeSessionMaxBytes { get; init; }
    public ulong RealtimeMaxKeys { get; init; }
    public ulong RealtimeFlushBatch { get; init; }
    public ulong ScheduledMaxBytes { get; init; }
    public ulong ScheduledSessionMaxBytes { get; init; }
    public ulong ScheduledMaxMessages { get; init; }
    public ulong ScheduledFlushBatch { get; init; }
    /// <summary>Runs on the bounded native logging thread. Return promptly; never call Stop/Dispose.
    /// Exceptions are caught and counted in LoggerExceptions. Metrics queries are allowed.</summary>
    public Action<LogRecord>? Logger { get; init; }
    public LogLevel MinimumLogLevel { get; init; } = LogLevel.Info;
}

public sealed class ServerOptions {
    public ServerOptions(Keypair key) { Key = key ?? throw new ArgumentNullException(nameof(key)); }
    public Keypair Key { get; }
    public Transport Transport { get; init; } = Transport.Tcp;
    public string Host { get; init; } = "127.0.0.1";
    public ushort Port { get; init; }
    public bool InitialEncryption { get; init; } = true;
    public ulong ProtocolId { get; init; } = 1;
    public uint Version { get; init; } = 1;
}

public sealed class ClientOptions {
    public Transport Transport { get; init; } = Transport.Tcp;
    public string Host { get; init; } = "localhost";
    public ushort Port { get; init; }
    public byte[] JoinTicket { get; init; } = Array.Empty<byte>();
    public ulong ProtocolId { get; init; } = 1;
    public uint Version { get; init; } = 1;
    public ulong BuildId { get; init; }
    public ulong Capabilities { get; init; }
    public override string ToString() => $"ClientOptions({Transport}, credentials=<redacted>)";
}

public sealed class GameSendOptions {
    public uint? Sequence { get; init; }
    public uint? Tick { get; init; }
    public ulong CorrelationId { get; init; }
    public GamePriority Priority { get; init; } = GamePriority.Normal;
    /// <summary>Local staging expiry only; zero means no expiry.</summary>
    public ulong ExpiryMs { get; init; }
}

public sealed record LogRecord(ulong TimestampUnixMs, LogLevel Level, string EventName,
    ulong Runtime, ulong Endpoint, ulong Session, Transport Transport, int ErrorCode,
    ulong CorrelationId, string Message);
