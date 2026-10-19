namespace RNet;

/// <summary>Game event discriminant; session readiness and transport opening are different states.</summary>
public enum GameEventType : uint {
    /// <summary>Runtime started; no peer is implied.</summary>
    RuntimeStarted = 1,
    /// <summary>Endpoint socket opened, not game readiness.</summary>
    EndpointOpened,
    /// <summary>Endpoint-related failure; not always terminal, query endpoint validity.</summary>
    EndpointError,
    /// <summary>Server authorization request with login ticket and authenticated client metadata.</summary>
    AuthRequest,
    /// <summary>Server reauthorization with identity, fresh login ticket and old-session mapping.</summary>
    ResumeRequest,
    /// <summary>Business protocol/range rejected before application authorization.</summary>
    ProtocolRejected,
    /// <summary>Initial game negotiation ready; application sends can be attempted.</summary>
    SessionReady,
    /// <summary>Resume committed; Session is new, RelatedSession is the invalidated old handle.</summary>
    SessionResumed,
    /// <summary>Sensitive, single-use, same-server-runtime resume credential.</summary>
    ResumeTicket,
    /// <summary>Peer session closed; inspect Status.</summary>
    SessionClosed,
    /// <summary>Opaque business bytes and optional envelope metadata.</summary>
    Message,
    /// <summary>Advisory capacity recovery; a subsequent send can still be rejected.</summary>
    Writable,
    /// <summary>Outbound join or negotiation failed.</summary>
    JoinFailed,
    /// <summary>Local security barrier committed; inspect mode, epoch and operation.</summary>
    SecurityChanged,
    /// <summary>Quality classification changed; inspect signal basis and measurements.</summary>
    QualityChanged,
    /// <summary>Invalid game/control protocol; detect by Type even though Status is zero.</summary>
    ProtocolViolation,
    /// <summary>Stop completed; dispose runtime resources after consuming desired final events.</summary>
    RuntimeStopped
}

/// <summary>Owned managed bytes. Clear credential copies when no longer needed; ToString redacts them.</summary>
public sealed class GameEvent {
    /// <summary>Event discriminant; only fields documented for this kind are meaningful.</summary>
    public GameEventType Type { get; internal init; }
    /// <summary>Local owning endpoint; zero for runtime-wide events.</summary>
    public ulong Endpoint { get; internal init; }
    /// <summary>Current/pending local session; the new handle on SessionResumed. Zero when not applicable.</summary>
    public ulong Session { get; internal init; }
    /// <summary>Old local handle on ResumeRequest/SessionResumed; zero otherwise.</summary>
    public ulong RelatedSession { get; internal init; }
    /// <summary>Local error/rejection/close category. Other kinds, including ProtocolViolation, use zero: always
    /// inspect Type.</summary>
    public int Status { get; internal init; }
    /// <summary>Owned login ticket on AuthRequest, identity on ResumeRequest, credential on ResumeTicket, or business
    /// bytes on Message.</summary>
    public byte[] Data { get; internal init; } = Array.Empty<byte>();
    /// <summary>Owned fresh login ticket on ResumeRequest, separate from identity in Data; empty otherwise.</summary>
    public byte[] AuxiliaryData { get; internal init; } = Array.Empty<byte>();
    /// <summary>Authenticated 32-byte client key on AuthRequest/ResumeRequest; other event kinds carry 32 zero-valued
    /// bytes.</summary>
    public byte[] ClientPublicKey { get; internal init; } = Array.Empty<byte>();
    /// <summary>Client build metadata on AuthRequest/ResumeRequest; not authorization.</summary>
    public ulong BuildId { get; internal init; }
    /// <summary>Client capability metadata on AuthRequest/ResumeRequest; application interprets it.</summary>
    public ulong Capabilities { get; internal init; }
    /// <summary>Resulting business mode on SecurityChanged; controls stay authenticated even when false.</summary>
    public bool Encrypted { get; internal init; }
    /// <summary>Session-local key/mode generation on SecurityChanged, not a time value.</summary>
    public ulong SecurityEpoch { get; internal init; }
    /// <summary>SecurityChanged operation: 1 mode switch, 2 rekey.</summary>
    public uint SecurityOperation { get; internal init; }
    /// <summary>QualityChanged grade: 0 unknown, 1 excellent, 2 good, 3 fair, 4 poor.</summary>
    public uint QualityGrade { get; internal init; }
    /// <summary>QualityChanged eligible signals: 1 latency, 2 latency plus UDP gaps, 3 latency plus KCP
    /// retransmissions.</summary>
    public uint QualityBasis { get; internal init; }
    /// <summary>Latest authenticated heartbeat RTT on QualityChanged, in microseconds including queueing.</summary>
    public ulong LastRttUs { get; internal init; }
    /// <summary>Smoothed absolute RTT variation on QualityChanged, in microseconds.</summary>
    public ulong JitterUs { get; internal init; }
    /// <summary>Accepted heartbeat sample count on QualityChanged.</summary>
    public ulong QualitySamples { get; internal init; }
    /// <summary>Sender sequence on Message; null means absent. Not the internal UDP loss sequence.</summary>
    public uint? Sequence { get; internal init; }
    /// <summary>Sender simulation tick on Message; null means absent. Not trusted time.</summary>
    public uint? Tick { get; internal init; }
    /// <summary>Optional peer-visible request/trace metadata on Message; zero absent.</summary>
    public ulong CorrelationId { get; internal init; }
    /// <summary>Diagnostic representation that omits credential/payload contents; does not erase the stored
    /// bytes.</summary>
    public override string ToString() => $"GameEvent({Type}, endpoint={Endpoint}, session={Session}, status={Status}, data=<redacted>)";
}

public sealed unsafe partial class GameRuntime {
    /// <summary>Copies opaque payload to the Normal game queue; success is admission, not delivery.</summary>
    /// <param name="session">Ready local game session.</param>
    /// <param name="payload">Business bytes; empty is valid. No message-type or transport selector is required.</param>
    /// <remarks>Continue polling to forward queued messages. Code -4 is backpressure, -11 is not ready; oversize/stale
    /// handles fail. Do not mutate input concurrently with this call.</remarks>
    public void Send(ulong session, ReadOnlySpan<byte> payload) {
        using var h = Lease(); fixed (byte* ptr = payload)
            Native.Check(Native.rnet_game_send(h.Value, session, new Native.Slice(ptr, payload.Length)));
    }
    /// <summary>Copies business bytes with optional envelope metadata and local scheduling.</summary>
    /// <param name="session">Ready local game session.</param>
    /// <param name="payload">Opaque bytes, possibly empty; copied before return.</param>
    /// <param name="options">Non-null sequence/tick/correlation/priority/expiry settings.</param>
    /// <remarks>Success only admits staged work. Expiry cannot retract bytes already admitted to transport.</remarks>
    public void Send(ulong session, ReadOnlySpan<byte> payload, GameSendOptions options) {
        ArgumentNullException.ThrowIfNull(options);
        using var h = Lease();
        var raw = new Native.SendOptions { Size = (uint)sizeof(Native.SendOptions), Abi = Native.AbiVersion,
            HasSequence = options.Sequence.HasValue ? 1u : 0u, Sequence = options.Sequence ?? 0,
            HasTick = options.Tick.HasValue ? 1u : 0u, Tick = options.Tick ?? 0,
            CorrelationId = options.CorrelationId, Priority = (uint)options.Priority, ExpiryMs = options.ExpiryMs };
        fixed (byte* ptr = payload) Native.Check(Native.rnet_game_send_ex(h.Value, session, new Native.Slice(ptr, payload.Length), &raw));
    }
    /// <summary>Copies a best-effort replaceable snapshot for a ready session and key.</summary>
    /// <param name="session">Ready local game session.</param>
    /// <param name="key">Session-local replacement key; zero is valid.</param>
    /// <param name="payload">Snapshot bytes, copied before return.</param>
    /// <remarks>Replacement covers game staging, plus TCP/KCP pending slots until worker pickup. UDP replacement stops
    /// at game staging; socket writes and KCP send-cache contents cannot be recalled. Poll to forward.</remarks>
    public void SendLatest(ulong session, ulong key, ReadOnlySpan<byte> payload) {
        using var h = Lease(); fixed (byte* ptr = payload)
            Native.Check(Native.rnet_game_send_latest(h.Value, session, key, new Native.Slice(ptr, payload.Length)));
    }
    /// <summary>Drives game state and returns owned event copies, releasing native tokens in finally.</summary>
    /// <param name="capacity">Maximum returned events, 0..4096; default 64. Zero still drives nonblocking
    /// maintenance.</param>
    /// <param name="timeoutMs">Idle event-wait budget in milliseconds; zero nonblocking, ignored for zero capacity.
    /// Lock/work time is additional.</param>
    /// <returns>Owned events, possibly empty; an empty result is not proof of disconnection.</returns>
    /// <remarks>Poll both peers even when idle. Copies survive later polls/disposal; caller erases copied credentials.
    /// Staged count/byte limits pause transport draining, not timers. Use positive capacity regularly;
    /// zero-only polling can block queued ACKs and cause heartbeat timeouts.
    /// Conversion failure can consume exported events without returning them.</remarks>
    /// <exception cref="ArgumentOutOfRangeException">Capacity is outside 0..4096.</exception>
    public GameEvent[] Poll(int capacity = 64, uint timeoutMs = 0) {
        if (capacity < 0 || capacity > 4096) throw new ArgumentOutOfRangeException(nameof(capacity));
        using var h = Lease();
        var raw = new Native.Event[capacity];
        fixed (Native.Event* events = raw) {
            nuint count = 0;
            try {
                Native.Check(Native.rnet_game_poll_events(h.Value, events, (nuint)capacity, timeoutMs, out count));
                var result = new GameEvent[checked((int)count)];
                for (int i = 0; i < result.Length; i++) {
                    var e = events[i];
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
