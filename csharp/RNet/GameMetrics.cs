using System.Runtime.InteropServices;

namespace RNet;

/// <summary>Cumulative runtime-wide metrics; times are microseconds, zero samples means unavailable.
/// Managed callback exceptions are separately exposed as GameRuntime.LoggerExceptions.</summary>
[StructLayout(LayoutKind.Sequential)]
public struct GameMetrics {
    internal uint Size, Abi, HasLogger, Reserved;
    public ulong HeartbeatProbesSent, HeartbeatProbeSendFailures, HeartbeatRepliesSent, HeartbeatReplySendFailures;
    public ulong HeartbeatRepliesMatched, HeartbeatRepliesRejected, HeartbeatProbesRateLimited, HeartbeatTimeouts;
    public ulong RttSamples, RttP50Us, RttP90Us, RttP95Us, RttP99Us, RttP999Us, RttMaxUs;
    public ulong ResumeTicketsIssued, ResumeRequestsReceived, ResumeTicketsRejected, ResumeAuthorizationDenied;
    public ulong ResumePendingRevoked, ResumeSessionsResumed, ResumeOutstandingTickets;
    public ulong ClockProbesSent, ClockRepliesSent, ClockSamples, ClockRejected, ClockSendFailures;
    public ulong LoggerDropped, LoggerSinkPanics, LoggerCallbackSamples, LoggerCallbackP99Us, LoggerCallbackMaxUs;
    public bool LoggerAvailable => HasLogger != 0;
}

/// <summary>UDP loss is a sequence-gap estimate; KCP is a retransmission ratio, not IP loss.
/// Null is returned before the first authenticated heartbeat sample.</summary>
public sealed record NetworkQuality(uint Grade, uint Basis, ulong RttUs, ulong SmoothedRttUs,
    ulong JitterUs, ulong Samples, uint? UdpLossPerMille, uint? KcpRetransmissionsPerMille,
    ulong UdpExpected, ulong UdpMissing, ulong KcpSent, ulong KcpRetransmitted);
/// <summary>Approximate runtime-local clock offset, never UTC or an anti-cheat clock.</summary>
public sealed record ClockSample(long ServerMinusClientUs, ulong RttUs, ulong Samples);

internal static unsafe partial class Native {
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rnet_game_metrics_snapshot(ulong runtime, GameMetrics* metrics);
}

public sealed unsafe partial class GameRuntime {
    public GameMetrics MetricsSnapshot() {
        using var h = Lease(); GameMetrics metrics = default;
        Native.Check(Native.rnet_game_metrics_snapshot(h.Value, &metrics)); return metrics;
    }
    /// <summary>Low-cardinality native counters and percentiles, with managed logger exceptions appended.</summary>
    public string PrometheusSnapshot() {
        using var h = Lease(); Native.Buffer buffer = default;
        try {
            Native.Check(Native.rnet_game_prometheus_snapshot(h.Value, &buffer));
            return Native.Text(buffer.Data, buffer.Length) +
                $"# TYPE rnet_game_csharp_logger_exceptions_total counter\nrnet_game_csharp_logger_exceptions_total {LoggerExceptions}\n";
        } finally { if (buffer.Token != 0) Native.rnet_game_buffer_release(h.Value, buffer.Token); }
    }
    public NetworkQuality? QualitySnapshot(ulong session) {
        using var h = Lease(); Native.Quality q = default;
        Native.Check(Native.rnet_game_network_quality(h.Value, session, &q));
        return q.Available == 0 ? null : new NetworkQuality(q.Grade, q.Basis, q.RttUs, q.SmoothedRttUs,
            q.JitterUs, q.Samples, q.HasUdpLoss == 0 ? null : q.UdpLossPerMille,
            q.HasKcpRetransmissions == 0 ? null : q.KcpRetransmissionsPerMille,
            q.UdpExpected, q.UdpMissing, q.KcpSent, q.KcpRetransmitted);
    }
    public ClockSample? ClockSyncSnapshot(ulong session) {
        using var h = Lease(); Native.Clock c = default;
        Native.Check(Native.rnet_game_clock_sync_snapshot(h.Value, session, &c));
        return c.Available == 0 ? null : new ClockSample(c.OffsetUs, c.RttUs, c.Samples);
    }
    public ulong ClockMicros() { using var h = Lease(); Native.Check(Native.rnet_game_clock_micros(h.Value, out var value)); return value; }
}
