namespace RNet;

public enum GameEventType : uint {
    RuntimeStarted = 1, EndpointOpened, EndpointError, AuthRequest, ResumeRequest,
    ProtocolRejected, SessionReady, SessionResumed, ResumeTicket, SessionClosed,
    Message, Writable, JoinFailed, SecurityChanged, QualityChanged, ProtocolViolation, RuntimeStopped
}

/// <summary>Owned managed bytes. Clear credential copies when no longer needed; ToString redacts them.</summary>
public sealed class GameEvent {
    public GameEventType Type { get; internal init; }
    public ulong Endpoint { get; internal init; }
    public ulong Session { get; internal init; }
    public ulong RelatedSession { get; internal init; }
    public int Status { get; internal init; }
    public byte[] Data { get; internal init; } = Array.Empty<byte>();
    public byte[] AuxiliaryData { get; internal init; } = Array.Empty<byte>();
    public byte[] ClientPublicKey { get; internal init; } = Array.Empty<byte>();
    public ulong BuildId { get; internal init; }
    public ulong Capabilities { get; internal init; }
    public bool Encrypted { get; internal init; }
    public ulong SecurityEpoch { get; internal init; }
    public uint SecurityOperation { get; internal init; }
    public uint QualityGrade { get; internal init; }
    public uint QualityBasis { get; internal init; }
    public ulong LastRttUs { get; internal init; }
    public ulong JitterUs { get; internal init; }
    public ulong QualitySamples { get; internal init; }
    public uint? Sequence { get; internal init; }
    public uint? Tick { get; internal init; }
    public ulong CorrelationId { get; internal init; }
    public override string ToString() => $"GameEvent({Type}, endpoint={Endpoint}, session={Session}, status={Status}, data=<redacted>)";
}

public sealed unsafe partial class GameRuntime {
    /// <summary>Only session + opaque payload are required. Queue admission is not delivery.
    /// Code -4 means backpressure; code -11 means game readiness has not been observed yet.</summary>
    public void Send(ulong session, ReadOnlySpan<byte> payload) {
        using var h = Lease(); fixed (byte* ptr = payload)
            Native.Check(Native.rnet_game_send(h.Value, session, new Native.Slice(ptr, payload.Length)));
    }
    public void Send(ulong session, ReadOnlySpan<byte> payload, GameSendOptions options) {
        ArgumentNullException.ThrowIfNull(options);
        using var h = Lease();
        var raw = new Native.SendOptions { Size = (uint)sizeof(Native.SendOptions), Abi = 1,
            HasSequence = options.Sequence.HasValue ? 1u : 0u, Sequence = options.Sequence ?? 0,
            HasTick = options.Tick.HasValue ? 1u : 0u, Tick = options.Tick ?? 0,
            CorrelationId = options.CorrelationId, Priority = (uint)options.Priority, ExpiryMs = options.ExpiryMs };
        fixed (byte* ptr = payload) Native.Check(Native.rnet_game_send_ex(h.Value, session, new Native.Slice(ptr, payload.Length), &raw));
    }
    /// <summary>Replaces pending snapshots by session/key, before native I/O worker pickup.</summary>
    public void SendLatest(ulong session, ulong key, ReadOnlySpan<byte> payload) {
        using var h = Lease(); fixed (byte* ptr = payload)
            Native.Check(Native.rnet_game_send_latest(h.Value, session, key, new Native.Slice(ptr, payload.Length)));
    }
    /// <summary>Copies all bytes before releasing native buffers, including on conversion failure.
    /// Zero capacity performs a nonblocking maintenance tick. Maximum batch size is 4096.</summary>
    public GameEvent[] Poll(int capacity = 64, uint timeoutMs = 0) {
        if (capacity < 0 || capacity > 4096) throw new ArgumentOutOfRangeException(nameof(capacity));
        using var h = Lease();
        var raw = new Native.EventV2[capacity];
        fixed (Native.EventV2* events = raw) {
            nuint count = 0;
            try {
                Native.Check(Native.rnet_game_poll_events_v2(h.Value, events, (nuint)capacity, timeoutMs, out count));
                var result = new GameEvent[checked((int)count)];
                for (int i = 0; i < result.Length; i++) {
                    var e = events[i].Base;
                    result[i] = new GameEvent {
                        Type = (GameEventType)e.Type, Endpoint = e.Endpoint, Session = e.Session,
                        RelatedSession = e.RelatedSession, Status = e.Status,
                        Data = Native.Copy(e.Data, e.Length), AuxiliaryData = Native.Copy(e.Aux, e.AuxLength),
                        ClientPublicKey = new ReadOnlySpan<byte>(e.PeerKey, 32).ToArray(),
                        BuildId = e.BuildId, Capabilities = e.Capabilities, Encrypted = e.Encrypted != 0,
                        SecurityEpoch = e.SecurityEpoch, SecurityOperation = e.SecurityOperation,
                        QualityGrade = e.QualityGrade, QualityBasis = e.QualityBasis,
                        LastRttUs = e.LastRttUs, JitterUs = e.JitterUs, QualitySamples = e.QualitySamples,
                        Sequence = e.HasSequence == 0 ? null : e.Sequence, Tick = e.HasTick == 0 ? null : e.Tick,
                        CorrelationId = events[i].CorrelationId
                    };
                }
                return result;
            } finally { RuntimeOwner.ReleaseBuffers(h.Value, events, count); }
        }
    }
}
