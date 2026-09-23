using System.Security.Cryptography;
using System.Text;

namespace RNet;

public sealed unsafe partial class GameRuntime {
    public ulong Listen(ServerOptions options) => ListenCore(options, null);
    /// <summary>Explicit wire-v4 negotiation; legacy Listen/Connect remain exact-version wire v3.</summary>
    public ulong ListenRange(ServerOptions options, uint minVersion, uint maxVersion) => ListenCore(options, (minVersion, maxVersion));
    private ulong ListenCore(ServerOptions o, (uint Min, uint Max)? range) {
        ArgumentNullException.ThrowIfNull(o);
        using var h = Lease();
        var host = Encoding.UTF8.GetBytes(o.Host);
        var key = o.Key.ExportPrivateKey();
        try {
            fixed (byte* hp = host, kp = key) {
                if (range is { } r) {
                    var raw = new Native.RangeServer { Size = (uint)sizeof(Native.RangeServer), Abi = 1,
                        Transport = (uint)o.Transport, Encryption = o.InitialEncryption ? 1u : 0u,
                        Host = new Native.Slice(hp, host.Length), Port = o.Port, PrivateKey = new Native.Slice(kp, key.Length),
                        ProtocolId = o.ProtocolId, MinVersion = r.Min, MaxVersion = r.Max };
                    Native.Check(Native.rnet_game_server_listen_range(h.Value, &raw, out var endpoint)); return endpoint;
                } else {
                    var raw = new Native.Server { Size = (uint)sizeof(Native.Server), Abi = 1,
                        Transport = (uint)o.Transport, Encryption = o.InitialEncryption ? 1u : 0u,
                        Host = new Native.Slice(hp, host.Length), Port = o.Port, PrivateKey = new Native.Slice(kp, key.Length),
                        ProtocolId = o.ProtocolId, Version = o.Version };
                    Native.Check(Native.rnet_game_server_listen(h.Value, &raw, out var endpoint)); return endpoint;
                }
            }
        } finally { CryptographicOperations.ZeroMemory(key); }
    }
    public ulong Connect(ClientOptions options) => ConnectCore(options, null, 0, default, false);
    public ulong ConnectRange(ClientOptions options, uint minVersion, uint maxVersion) => ConnectCore(options, (minVersion, maxVersion), 0, default, false);
    /// <summary>One-use, same-server-runtime recovery. Both peers receive new session handles after authorization.</summary>
    public ulong ConnectResume(ClientOptions options, ulong oldSession, ReadOnlySpan<byte> ticket) => ConnectCore(options, null, oldSession, ticket, true);
    public ulong ConnectRangeResume(ClientOptions options, uint minVersion, uint maxVersion, ulong oldSession, ReadOnlySpan<byte> ticket) => ConnectCore(options, (minVersion, maxVersion), oldSession, ticket, true);
    private ulong ConnectCore(ClientOptions o, (uint Min, uint Max)? range, ulong oldSession, ReadOnlySpan<byte> resumeTicket, bool resume) {
        ArgumentNullException.ThrowIfNull(o);
        using var h = Lease();
        var host = Encoding.UTF8.GetBytes(o.Host);
        ArgumentNullException.ThrowIfNull(o.JoinTicket);
        fixed (byte* hp = host, tp = o.JoinTicket, rp = resumeTicket) {
            var ticket = new Native.Slice(rp, resumeTicket.Length);
            ulong endpoint;
            if (range is { } r) {
                var raw = new Native.RangeClient { Size = (uint)sizeof(Native.RangeClient), Abi = 1, Transport = (uint)o.Transport,
                    Host = new Native.Slice(hp, host.Length), Port = o.Port, Ticket = new Native.Slice(tp, o.JoinTicket.Length),
                    ProtocolId = o.ProtocolId, MinVersion = r.Min, MaxVersion = r.Max, BuildId = o.BuildId, Capabilities = o.Capabilities };
                Native.Check(resume ? Native.rnet_game_client_resume_connect_range(h.Value, &raw, oldSession, ticket, out endpoint)
                    : Native.rnet_game_client_connect_range(h.Value, &raw, out endpoint));
            } else {
                var raw = new Native.Client { Size = (uint)sizeof(Native.Client), Abi = 1, Transport = (uint)o.Transport,
                    Host = new Native.Slice(hp, host.Length), Port = o.Port, Ticket = new Native.Slice(tp, o.JoinTicket.Length),
                    ProtocolId = o.ProtocolId, Version = o.Version, BuildId = o.BuildId, Capabilities = o.Capabilities };
                Native.Check(resume ? Native.rnet_game_client_resume_connect(h.Value, &raw, oldSession, ticket, out endpoint)
                    : Native.rnet_game_client_connect(h.Value, &raw, out endpoint));
            }
            return endpoint;
        }
    }
    public void IssueResumeTicket(ulong session, ReadOnlySpan<byte> identity) {
        using var h = Lease(); fixed (byte* ptr = identity) Native.Check(Native.rnet_game_issue_resume_ticket(h.Value, session, new Native.Slice(ptr, identity.Length)));
    }
}
