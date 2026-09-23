using System.Runtime.InteropServices;

namespace RNet;

// ABI-only types. Native bools are uint32; handles are uint64 even on managed runtimes.
internal static unsafe partial class Native
{
    [StructLayout(LayoutKind.Sequential)]
    internal struct Slice { public byte* Data; public nuint Length;
        public Slice(byte* data, int length) { Data = data; Length = checked((nuint)length); } }
    [StructLayout(LayoutKind.Sequential)]
    internal struct GameConfig {
        public uint Size, Abi, HeartbeatIntervalMs, HeartbeatTimeoutMs, AllowPlaintext, Reserved;
        public NetworkConfig* Network;
        public ulong RealtimeBytes, RealtimeSessionBytes, RealtimeKeys, RealtimeBatch;
        public ulong ScheduledBytes, ScheduledSessionBytes, ScheduledMessages, ScheduledBatch;
    }
    [StructLayout(LayoutKind.Sequential)]
    internal struct NetworkConfig {
        public uint Size, Abi, Workers, EventCapacity, WriteCapacity, MaxBody, MaxDatagram;
        public ulong MaxEventBytes, MaxRuntimeBytes, MaxSessionBytes;
        public uint MaxSessions, MaxSessionsPerIp, HandshakeRate, HandshakeBurst, Ipv6Prefix;
        public ulong HandshakeTimeoutMs, ConnectTimeoutMs, DnsTimeoutMs, IdleTimeoutMs;
        public uint AllowPlaintext, AllowLegacy;
        public ulong RekeyAfterMs, RekeyAfterBytes;
        public nint Logger, LoggerV2;
        public uint TcpNoDelay;
        public ulong TcpSendBuffer, TcpReceiveBuffer;
        public uint MaxEndpoints, MaxPendingHandshakes;
    }
    [StructLayout(LayoutKind.Sequential)]
    internal struct Security { public uint Size, Abi; public Slice PrivateKey, ExpectedKey; public nint Verify, UserData; }
    [StructLayout(LayoutKind.Sequential)]
    internal struct Key { public uint Size, Abi; public fixed byte Private[32]; public fixed byte Public[32]; }
    [StructLayout(LayoutKind.Sequential)]
    internal struct Server {
        public uint Size, Abi, Transport, Encryption;
        public Slice Host; public ushort Port, Reserved; public Slice PrivateKey;
        public ulong ProtocolId; public uint Version;
    }
    [StructLayout(LayoutKind.Sequential)]
    internal struct Client {
        public uint Size, Abi, Transport, Reserved;
        public Slice Host; public ushort Port, Reserved2; public Slice Ticket;
        public ulong ProtocolId; public uint Version, Reserved3; public ulong BuildId, Capabilities;
    }
    [StructLayout(LayoutKind.Sequential)]
    internal struct RangeServer {
        public uint Size, Abi, Transport, Encryption;
        public Slice Host; public ushort Port, Reserved; public Slice PrivateKey;
        public ulong ProtocolId; public uint MinVersion, MaxVersion;
    }
    [StructLayout(LayoutKind.Sequential)]
    internal struct RangeClient {
        public uint Size, Abi, Transport, Reserved;
        public Slice Host; public ushort Port, Reserved2; public Slice Ticket;
        public ulong ProtocolId; public uint MinVersion, MaxVersion, Reserved3;
        public ulong BuildId, Capabilities;
    }
    [StructLayout(LayoutKind.Sequential)]
    internal struct Event {
        public uint Size, Type; public ulong Endpoint, Session, RelatedSession; public int Status;
        public byte* Data; public nuint Length; public ulong Token;
        public byte* Aux; public nuint AuxLength; public ulong AuxToken;
        public fixed byte PeerKey[32]; public ulong BuildId, Capabilities;
        public uint Encrypted; public ulong SecurityEpoch;
        public uint SecurityOperation, QualityGrade, QualityBasis;
        public ulong LastRttUs, JitterUs, QualitySamples;
        public uint HasSequence, Sequence, HasTick, Tick;
    }
    [StructLayout(LayoutKind.Sequential)]
    internal struct EventV2 { public Event Base; public ulong CorrelationId; }
    [StructLayout(LayoutKind.Sequential)]
    internal struct Buffer { public uint Size, Abi; public byte* Data; public nuint Length; public ulong Token; }
    [StructLayout(LayoutKind.Sequential)]
    internal struct Logger { public uint Size, Abi; public nint Callback, UserData; public uint MinLevel; }
    [StructLayout(LayoutKind.Sequential)]
    internal struct SendOptions {
        public uint Size, Abi, HasSequence, Sequence, HasTick, Tick;
        public ulong CorrelationId; public uint Priority; public ulong ExpiryMs;
    }
    [StructLayout(LayoutKind.Sequential)]
    internal struct Quality {
        public uint Size, Abi, Available, Grade, Basis, HasUdpLoss, HasKcpRetransmissions, UdpLossPerMille, KcpRetransmissionsPerMille;
        public ulong RttUs, SmoothedRttUs, JitterUs, Samples, UdpExpected, UdpMissing, KcpSent, KcpRetransmitted;
    }
    [StructLayout(LayoutKind.Sequential)]
    internal struct Clock { public uint Size, Abi, Available, Reserved; public long OffsetUs; public ulong RttUs, Samples; }
}
