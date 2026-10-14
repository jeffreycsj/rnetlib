using System.Security.Cryptography;
using System.Text;

namespace RNet;

public sealed unsafe partial class GameRuntime {
    /// <summary>Accepts the configured inclusive application version range; both peers must poll until ready.</summary>
    public ulong Listen(ServerOptions o) {
        ArgumentNullException.ThrowIfNull(o);
        using var h = Lease();
        var host = Encoding.UTF8.GetBytes(o.Host);
        var key = o.Key.ExportPrivateKey();
        try {
            fixed (byte* hp = host, kp = key) {
                var raw = new Native.Server { Size = (uint)sizeof(Native.Server), Abi = Native.AbiVersion,
                    Transport = (uint)o.Transport, Encryption = o.InitialEncryption ? 1u : 0u,
                    Host = new Native.Slice(hp, host.Length), Port = o.Port, PrivateKey = new Native.Slice(kp, key.Length),
                    ProtocolId = o.ProtocolId, MinVersion = o.MinVersion, MaxVersion = o.MaxVersion };
                Native.Check(Native.rnet_game_server_listen(h.Value, &raw, out var endpoint));
                return endpoint;
            }
        } finally { CryptographicOperations.ZeroMemory(key); }
    }
    public ulong Connect(ClientOptions options) => ConnectCore(options, 0, default, false);
    /// <summary>One-use, same-server-runtime recovery. Both peers receive new session handles after authorization and negotiation.</summary>
    public ulong ConnectResume(ClientOptions options, ulong oldSession, ReadOnlySpan<byte> ticket) => ConnectCore(options, oldSession, ticket, true);
    private ulong ConnectCore(ClientOptions o, ulong oldSession, ReadOnlySpan<byte> resumeTicket, bool resume) {
        ArgumentNullException.ThrowIfNull(o);
        using var h = Lease();
        var host = Encoding.UTF8.GetBytes(o.Host);
        ArgumentNullException.ThrowIfNull(o.JoinTicket);
        fixed (byte* hp = host, tp = o.JoinTicket, rp = resumeTicket) {
            var ticket = new Native.Slice(rp, resumeTicket.Length);
            var raw = new Native.Client { Size = (uint)sizeof(Native.Client), Abi = Native.AbiVersion, Transport = (uint)o.Transport,
                Host = new Native.Slice(hp, host.Length), Port = o.Port, Ticket = new Native.Slice(tp, o.JoinTicket.Length),
                ProtocolId = o.ProtocolId, MinVersion = o.MinVersion, MaxVersion = o.MaxVersion, BuildId = o.BuildId, Capabilities = o.Capabilities };
            ulong endpoint;
            Native.Check(resume ? Native.rnet_game_client_resume_connect(h.Value, &raw, oldSession, ticket, out endpoint)
                : Native.rnet_game_client_connect(h.Value, &raw, out endpoint));
            return endpoint;
        }
    }
    public void IssueResumeTicket(ulong session, ReadOnlySpan<byte> identity) {
        using var h = Lease(); fixed (byte* ptr = identity) Native.Check(Native.rnet_game_issue_resume_ticket(h.Value, session, new Native.Slice(ptr, identity.Length)));
    }
}
