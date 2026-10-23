using System.Runtime.InteropServices;

namespace RNet;

/// <summary>Cumulative runtime-wide metrics; times are microseconds, zero samples means unavailable.
/// Managed callback exceptions are separately exposed as GameRuntime.LoggerExceptions.</summary>
[StructLayout(LayoutKind.Sequential)]
public struct GameMetrics {
    internal uint Size, Abi, HasLogger, Reserved;
    /// <summary>Cumulative protected probes admitted locally, not confirmed delivered.</summary>
    public ulong HeartbeatProbesSent;
    /// <summary>Cumulative failed local probe sends.</summary>
    public ulong HeartbeatProbeSendFailures;
    /// <summary>Cumulative protected replies admitted locally.</summary>
    public ulong HeartbeatRepliesSent;
    /// <summary>Cumulative failed local reply sends.</summary>
    public ulong HeartbeatReplySendFailures;
    /// <summary>Cumulative timely replies matching outstanding local challenges.</summary>
    public ulong HeartbeatRepliesMatched;
    /// <summary>Cumulative invalid/late challenge replies; do not refresh liveness.</summary>
    public ulong HeartbeatRepliesRejected;
    /// <summary>Cumulative inbound probes suppressed by response rate limits.</summary>
    public ulong HeartbeatProbesRateLimited;
    /// <summary>Cumulative heartbeat deadlines causing session-close requests.</summary>
    public ulong HeartbeatTimeouts;
    /// <summary>Cumulative accepted RTT samples; zero means unavailable.</summary>
    public ulong RttSamples;
    /// <summary>Cumulative estimated heartbeat RTT median in microseconds.</summary>
    public ulong RttP50Us;
    /// <summary>Cumulative estimated heartbeat RTT P90 in microseconds.</summary>
    public ulong RttP90Us;
    /// <summary>Cumulative estimated heartbeat RTT P95 in microseconds.</summary>
    public ulong RttP95Us;
    /// <summary>Cumulative estimated heartbeat RTT P99 in microseconds.</summary>
    public ulong RttP99Us;
    /// <summary>Cumulative estimated heartbeat RTT P99.9 in microseconds.</summary>
    public ulong RttP999Us;
    /// <summary>Maximum sampled heartbeat RTT in microseconds.</summary>
    public ulong RttMaxUs;
    /// <summary>Cumulative tickets issued and admitted to protected send, not confirmed delivered.</summary>
    public ulong ResumeTicketsIssued;
    /// <summary>Cumulative decoded resume joins reaching ticket validation.</summary>
    public ulong ResumeRequestsReceived;
    /// <summary>Cumulative invalid, expired, replayed or incompatible ticket claims.</summary>
    public ulong ResumeTicketsRejected;
    /// <summary>Cumulative valid resume claims denied by business authorization.</summary>
    public ulong ResumeAuthorizationDenied;
    /// <summary>Cumulative pending/in-flight resume claims revoked by explicit cleanup.</summary>
    public ulong ResumePendingRevoked;
    /// <summary>Cumulative completed resume publications on this runtime.</summary>
    public ulong ResumeSessionsResumed;
    /// <summary>Current retained registry entries; expiry cleanup is opportunistic.</summary>
    public ulong ResumeOutstandingTickets;
    /// <summary>Cumulative locally admitted client clock probes.</summary>
    public ulong ClockProbesSent;
    /// <summary>Cumulative locally admitted server clock replies.</summary>
    public ulong ClockRepliesSent;
    /// <summary>Cumulative accepted client clock samples.</summary>
    public ulong ClockSamples;
    /// <summary>Cumulative rate-limited probes or tracker-rejected replies, not every parser error.</summary>
    public ulong ClockRejected;
    /// <summary>Cumulative probe generation/admission and reply send-admission failures.</summary>
    public ulong ClockSendFailures;
    /// <summary>Cumulative records rejected by bounded logger queues.</summary>
    public ulong LoggerDropped;
    /// <summary>Cumulative caught native sink panics; managed exceptions are counted by
    /// GameRuntime.LoggerExceptions.</summary>
    public ulong LoggerSinkPanics;
    /// <summary>Completed callback samples, including caught panics.</summary>
    public ulong LoggerCallbackSamples;
    /// <summary>Cumulative estimated callback execution P99 in microseconds; excludes queue wait.</summary>
    public ulong LoggerCallbackP99Us;
    /// <summary>Largest measured callback execution time in microseconds.</summary>
    public ulong LoggerCallbackMaxUs;
    /// <summary>Whether native logging is installed; distinguishes disabled logging from zero drops.</summary>
    public bool LoggerAvailable => HasLogger != 0;
}

/// <summary>UDP loss is a sequence-gap estimate; KCP is a retransmission ratio, not IP loss.
/// Null is returned before the first authenticated heartbeat sample.</summary>
/// <param name="Grade">0 unknown, 1 excellent, 2 good, 3 fair, 4 poor; too few samples can keep it unknown.</param>
/// <param name="Basis">Eligible classification signals: 1 latency, 2 latency+UDP gaps, 3 latency+KCP
/// retransmissions.</param>
/// <param name="RttUs">Latest authenticated heartbeat RTT in microseconds, including queueing.</param>
/// <param name="SmoothedRttUs">Smoothed heartbeat RTT in microseconds; not a percentile.</param>
/// <param name="JitterUs">Smoothed absolute RTT variation in microseconds.</param>
/// <param name="Samples">Accepted heartbeat sample count.</param>
/// <param name="UdpLossPerMille">UDP sequence-gap estimate per 1000 positions; null when unavailable, not measured
/// zero.</param>
/// <param name="KcpRetransmissionsPerMille">KCP retransmission ratio per 1000 PUSH transmissions; null when
/// unavailable, not raw IP loss.</param>
/// <param name="UdpExpected">Cumulative expected UDP sequence positions; meaningful only with UDP evidence.</param>
/// <param name="UdpMissing">Expected minus received UDP positions; late arrivals can reduce it.</param>
/// <param name="KcpSent">Cumulative KCP PUSH transmissions, including retransmissions, excluding ACKs.</param>
/// <param name="KcpRetransmitted">Cumulative repeated KCP PUSH transmissions.</param>
public sealed record NetworkQuality(uint Grade, uint Basis, ulong RttUs, ulong SmoothedRttUs,
    ulong JitterUs, ulong Samples, uint? UdpLossPerMille, uint? KcpRetransmissionsPerMille,
    ulong UdpExpected, ulong UdpMissing, ulong KcpSent, ulong KcpRetransmitted);
/// <summary>Approximate runtime-local clock offset, never UTC or an anti-cheat clock.</summary>
/// <param name="ServerMinusClientUs">Signed microseconds added to the client runtime clock to approximate the server
/// runtime clock.</param>
/// <param name="RttUs">Round-trip microseconds after subtracting reported server turnaround.</param>
/// <param name="Samples">Accepted clock samples, not heartbeat samples.</param>
public sealed record ClockSample(long ServerMinusClientUs, ulong RttUs, ulong Samples);

internal static unsafe partial class Native {
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rnet_game_metrics_snapshot(ulong runtime, GameMetrics* metrics);
}

public sealed unsafe partial class GameRuntime {
    /// <summary>Copies runtime-wide cumulative counters and live gauges without resetting them.</summary>
    /// <returns>A non-atomic cross-field observation; sample counts distinguish unavailable percentiles.</returns>
    public GameMetrics MetricsSnapshot() {
        using var h = Lease(); GameMetrics metrics = default;
        Native.Check(Native.rnet_game_metrics_snapshot(h.Value, &metrics)); return metrics;
    }
    /// <summary>Copies low-cardinality Prometheus text and appends managed logger-exception counts.</summary>
    /// <returns>Owned text without per-player labels; native export memory is released even on conversion
    /// failure.</returns>
    public string PrometheusSnapshot() {
        using var h = Lease(); Native.Buffer buffer = default;
        try {
            Native.Check(Native.rnet_game_prometheus_snapshot(h.Value, &buffer));
            return Native.Text(buffer.Data, buffer.Length) +
                $"# TYPE rnet_game_csharp_logger_exceptions_total counter\nrnet_game_csharp_logger_exceptions_total {LoggerExceptions}\n";
        } finally { if (buffer.Token != 0) Native.rnet_game_buffer_release(h.Value, buffer.Token); }
    }
    /// <summary>Queries authenticated heartbeat quality and transport-specific evidence.</summary>
    /// <param name="session">Local tracked session; stale/untracked handles throw RNetException.</param>
    /// <returns>Null before the first heartbeat sample; TCP loss is unavailable rather than measured zero.</returns>
    public NetworkQuality? QualitySnapshot(ulong session) {
        using var h = Lease(); Native.Quality q = default;
        Native.Check(Native.rnet_game_network_quality(h.Value, session, &q));
        return q.Available == 0 ? null : new NetworkQuality(q.Grade, q.Basis, q.RttUs, q.SmoothedRttUs,
            q.JitterUs, q.Samples, q.HasUdpLoss == 0 ? null : q.UdpLossPerMille,
            q.HasKcpRetransmissions == 0 ? null : q.KcpRetransmissionsPerMille,
            q.UdpExpected, q.UdpMissing, q.KcpSent, q.KcpRetransmitted);
    }
    /// <summary>Queries the latest approximate client-side four-timestamp clock sample.</summary>
    /// <param name="session">Local game-ready session; pending/stale handles throw rather than return null.</param>
    /// <returns>Null on server sessions or before a sample; offset is monotonic-runtime time, not UTC/anti-cheat
    /// authority.</returns>
    public ClockSample? ClockSyncSnapshot(ulong session) {
        using var h = Lease(); Native.Clock c = default;
        Native.Check(Native.rnet_game_clock_sync_snapshot(h.Value, session, &c));
        return c.Available == 0 ? null : new ClockSample(c.OffsetUs, c.RttUs, c.Samples);
    }
    /// <summary>Queries this runtime's monotonic clock.</summary>
    /// <returns>Microseconds since its local origin; not comparable across runtimes without an offset sample.</returns>
    public ulong ClockMicros() { using var h = Lease(); Native.Check(Native.rnet_game_clock_micros(h.Value, out var value)); return value; }
}
