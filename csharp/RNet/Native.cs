using System.Runtime.InteropServices;

namespace RNet;

internal static unsafe partial class Native
{
    private const string Library = "rnet";
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern uint rnet_abi_version();
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern nint rnet_last_error_message();
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_keypair_generate(Key* key);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_keypair_from_private(Slice key, Key* output);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_config_v5_init(NetworkConfig* config);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_config_v2_init(GameConfig* config);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_runtime_create_v2(GameConfig* config, Security* security, out ulong runtime);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_runtime_create_logged_v2(GameConfig* config, Security* security, Logger* logger, out ulong runtime);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_server_listen(ulong runtime, Server* config, out ulong endpoint);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_client_connect(ulong runtime, Client* config, out ulong endpoint);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_server_listen_range(ulong runtime, RangeServer* config, out ulong endpoint);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_client_connect_range(ulong runtime, RangeClient* config, out ulong endpoint);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_client_resume_connect(ulong runtime, Client* config, ulong oldSession, Slice ticket, out ulong endpoint);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_client_resume_connect_range(ulong runtime, RangeClient* config, ulong oldSession, Slice ticket, out ulong endpoint);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_selected_protocol_version(ulong runtime, ulong session, out uint version);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_issue_resume_ticket(ulong runtime, ulong session, Slice identity);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_endpoint_local_port(ulong runtime, ulong endpoint, out ushort port);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_auth_decide(ulong runtime, ulong session, uint accept);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_send(ulong runtime, ulong session, Slice payload);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_send_ex(ulong runtime, ulong session, Slice payload, SendOptions* options);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_send_latest(ulong runtime, ulong session, ulong key, Slice payload);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_session_close(ulong runtime, ulong session);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_endpoint_close(ulong runtime, ulong endpoint);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_rekey(ulong runtime, ulong session);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_security_set(ulong runtime, ulong session, uint encrypted);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_poll_events_v2(ulong runtime, EventV2* events, nuint capacity, uint timeoutMs, out nuint count);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_buffer_release(ulong runtime, ulong token);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_runtime_stop(ulong runtime, uint drainMs);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_runtime_destroy(ulong runtime);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_prometheus_snapshot(ulong runtime, Buffer* buffer);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_metrics_log_interval_set(ulong runtime, ulong intervalMs);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_network_quality(ulong runtime, ulong session, Quality* quality);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_clock_sync_snapshot(ulong runtime, ulong session, Clock* clock);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern int rnet_game_clock_micros(ulong runtime, out ulong value);

    internal static void Check(int status) {
        // Capture native TLS text before making any other RNet call on this OS thread.
        if (status != 0) throw new RNetException(status, Marshal.PtrToStringUTF8(rnet_last_error_message()) ?? $"RNet error {status}");
    }
    internal static byte[] Copy(byte* ptr, nuint length) => new ReadOnlySpan<byte>(ptr, checked((int)length)).ToArray();
    internal static string Text(byte* ptr, nuint length) => System.Text.Encoding.UTF8.GetString(new ReadOnlySpan<byte>(ptr, checked((int)length)));
}
