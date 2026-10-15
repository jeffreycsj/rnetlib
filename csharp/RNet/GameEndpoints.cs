using System.Security.Cryptography;
using System.Text;

namespace RNet;

public sealed unsafe partial class GameRuntime {
    /// <summary>Binds a listener with fixed transport and inclusive business-version policy.</summary>
    /// <param name="o">Non-null options; numeric bind IP only. Identity is copied and remains caller-owned.</param>
    /// <returns>Local listener endpoint; port zero can be queried through LocalPort.</returns>
    /// <remarks>Poll for AuthRequest/ResumeRequest and decide authorization. Opening the listener is not peer
    /// readiness.</remarks>
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
    /// <summary>Starts an authenticated outbound join, using runtime-scoped identity and server pin.</summary>
    /// <param name="options">Non-null route/login/protocol settings; DNS name or numeric IP, not a URL.</param>
    /// <returns>Outbound endpoint, not a ready session.</returns>
    /// <remarks>DNS resolution can wait up to the configured limit. Continue polling for SessionReady or JoinFailed; do
    /// not send merely because this returned.</remarks>
    public ulong Connect(ClientOptions options) => ConnectCore(options, 0, default, false);
    /// <summary>Starts single-use, same-server-runtime recovery with a new connection.</summary>
    /// <param name="options">Route/version metadata and fresh authorization ticket.</param>
    /// <param name="oldSession">Nonzero old local session handle for the mapping.</param>
    /// <param name="ticket">Sensitive resume credential copied for this call.</param>
    /// <returns>New endpoint, not the resumed session.</returns>
    /// <remarks>Server must authorize ResumeRequest. SessionResumed maps the old handle to the new one after readiness;
    /// a still-live old session is retained if negotiation fails. No recovery across restart, rolling deployment or
    /// another server instance.</remarks>
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
    /// <summary>Server queues a single-use resume ticket for a ready peer.</summary>
    /// <param name="session">Ready server-side session.</param>
    /// <param name="identity">Opaque 1..64-byte business identity, copied; never log it.</param>
    /// <remarks>Success means local admission. Client receives ResumeTicket asynchronously and owns erasure of its
    /// copied credential.</remarks>
    public void IssueResumeTicket(ulong session, ReadOnlySpan<byte> identity) {
        using var h = Lease(); fixed (byte* ptr = identity) Native.Check(Native.rnet_game_issue_resume_ticket(h.Value, session, new Native.Slice(ptr, identity.Length)));
    }
}
