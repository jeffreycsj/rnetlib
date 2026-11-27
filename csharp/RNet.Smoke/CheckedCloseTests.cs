using System.Reflection;
using System.Runtime.InteropServices;
using RNet;

internal static class CheckedCloseTests {
    [StructLayout(LayoutKind.Sequential)]
    private struct Buffer {
        public uint Size, Abi;
        public nint Data;
        public nuint Length;
        public ulong Token;
    }
    [DllImport("rnet", CallingConvention = CallingConvention.Cdecl)]
    private static extern int rnet_game_prometheus_snapshot(ulong runtime, out Buffer buffer);
    [DllImport("rnet", CallingConvention = CallingConvention.Cdecl)]
    private static extern int rnet_game_buffer_release(ulong runtime, ulong token);
    [DllImport("rnet", CallingConvention = CallingConvention.Cdecl)]
    private static extern int rnet_game_runtime_stop(ulong runtime, uint timeout);

    private static void Require(bool condition, string message) {
        if (!condition) throw new Exception(message);
    }
    private static SafeHandle Owner(GameRuntime runtime) =>
        (SafeHandle)typeof(GameRuntime).GetField("owner", BindingFlags.NonPublic | BindingFlags.Instance)!.GetValue(runtime)!;
    private static ulong Handle(GameRuntime runtime) => unchecked((ulong)Owner(runtime).DangerousGetHandle());
    private static IDisposable Lease(GameRuntime runtime) =>
        (IDisposable)typeof(GameRuntime).GetMethod("Lease", BindingFlags.NonPublic | BindingFlags.Instance)!.Invoke(runtime, null)!;
    private static void Close(GameRuntime runtime) => runtime.Close();
    private static void Status(Action action, int code, string detail) {
        try { action(); throw new Exception($"expected native error {code}"); }
        catch (RNetException error) {
            Require(error.Code == code && error.Message.Contains(detail), $"unexpected error: {error}");
        }
    }
    internal static void Run() {
        DestroyFailureCanBeRetried();
        ActiveLeaseDoesNotStartStop();
        ConcurrentCloseAndDispose();
        DisposeStillDefersUntilLeaseEnds();
        CallbackCloseRejected();
        using var disposed = new GameRuntime();
        disposed.Dispose();
        try { Close(disposed); throw new Exception("Dispose cannot imply checked Close succeeded"); }
        catch (ObjectDisposedException) { }
        Console.WriteLine("PASS checked close: dispose-first cannot claim checked cleanup success");
    }

    private static void DestroyFailureCanBeRetried() {
        using var runtime = new GameRuntime(new GameOptions { Logger = record => { } });
        using var key = Keypair.Generate();
        ulong handle = Handle(runtime), token = 0;
        try {
            Require(rnet_game_prometheus_snapshot(handle, out var buffer) == 0 && buffer.Token != 0, "retain real native export");
            token = buffer.Token;
            for (int i = 0; i < 2; i++) {
                Status(() => Close(runtime), -3, "release all game event buffers");
                Require(!Owner(runtime).IsClosed && runtime.MetricsSnapshot().LoggerAvailable, "failed close lost ownership/logger");
                _ = runtime.PrometheusSnapshot();
            }
            Require(runtime.Poll().Count(e => e.Type == GameEventType.RuntimeStopped) == 1, "failed destroy lost/duplicated stop event");
            Status(() => runtime.Listen(new ServerOptions(key)), -3, "");
            Require(rnet_game_buffer_release(handle, token) == 0, "release retained export");
            token = 0;
            Close(runtime);
            Close(runtime);
            runtime.Dispose();
            Require(rnet_game_runtime_stop(handle, 0) == -2, "Close only invalidated the managed wrapper");
            try { _ = runtime.Poll(); throw new Exception("closed runtime accepted Poll"); }
            catch (ObjectDisposedException) { }
        } finally {
            if (token != 0) Require(rnet_game_buffer_release(handle, token) == 0, "failure cleanup token release");
        }
        Console.WriteLine("PASS checked close: real outstanding-buffer failure retains ownership and retries successfully");
    }

    private static void ActiveLeaseDoesNotStartStop() {
        using var runtime = new GameRuntime();
        using var key = Keypair.Generate();
        using (Lease(runtime)) {
            Status(() => Close(runtime), -4, "active");
            ulong endpoint = runtime.Listen(new ServerOptions(key));
            runtime.CloseEndpoint(endpoint);
        }
        Close(runtime);
        Console.WriteLine("PASS checked close: active lease rejects closure before stopping admission");
    }

    private static bool Closing(SafeHandle owner) {
        var flags = BindingFlags.NonPublic | BindingFlags.Instance;
        object gate = owner.GetType().GetField("gate", flags)!.GetValue(owner)!;
        lock (gate) { return (bool)owner.GetType().GetField("closing", flags)!.GetValue(owner)!; }
    }
    private static void ConcurrentCloseAndDispose() {
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        int armed = 0, queryRejected = 0;
        GameRuntime? current = null;
        using var runtime = new GameRuntime(new GameOptions { Logger = record => {
            if (Interlocked.CompareExchange(ref armed, 2, 1) != 1) return;
            entered.Set();
            if (!release.Wait(10000)) throw new TimeoutException("logger gate");
            try { _ = Volatile.Read(ref current)!.MetricsSnapshot(); }
            catch (RNetException error) when (error.Code == -4) { Interlocked.Increment(ref queryRejected); }
        } });
        Task? close = null;
        ulong handle = Handle(runtime);
        Volatile.Write(ref current, runtime);
        try {
            Volatile.Write(ref armed, 1);
            runtime.Poll();
            Require(entered.Wait(5000), "logger did not enter");
            close = Task.Run(() => Close(runtime));
            Require(SpinWait.SpinUntil(() => Closing(Owner(runtime)), 5000), "close not started");
            Status(() => Close(runtime), -4, "closing");
            Status(() => runtime.MetricsSnapshot(), -4, "closing");
            try { runtime.Dispose(); throw new Exception("Dispose stole checked-close ownership"); }
            catch (InvalidOperationException) { }
            Require(!close.IsCompleted, "close returned before logger exited");
            release.Set();
            Require(close.Wait(5000), "close did not finish after logger release");
            Require(runtime.LoggerExceptions == 0 && queryRejected == 1 && rnet_game_runtime_stop(handle, 0) == -2, "cleanup failed");
        } finally {
            release.Set();
            if (close != null) Require(close.Wait(5000), "close task leaked");
        }
        Console.WriteLine("PASS checked close: concurrent close, queries and Dispose rejected without callback deadlock");
    }

    private static void DisposeStillDefersUntilLeaseEnds() {
        using var runtime = new GameRuntime();
        ulong handle = Handle(runtime);
        using (Lease(runtime)) {
            runtime.Dispose();
            // SafeHandle.IsClosed need not become true while a DangerousAddRef is still held.
            // The wrapper must nevertheless reject new operations as soon as Dispose starts.
            try { _ = runtime.MetricsSnapshot(); throw new Exception("Dispose allowed a new managed lease"); }
            catch (ObjectDisposedException) { }
            Require(rnet_game_runtime_stop(handle, 0) == 0, "Dispose destroyed an active lease");
            try { Close(runtime); throw new Exception("Close accepted a disposed owner with active lease"); }
            catch (ObjectDisposedException) { }
        }
        Require(rnet_game_runtime_stop(handle, 0) == -2, "last lease did not perform deferred cleanup");
        Console.WriteLine("PASS checked close: existing Dispose defers native destruction until last lease release");
    }

    private static void CallbackCloseRejected() {
        GameRuntime? current = null;
        int rejected = 0, finished = 0;
        using var runtime = new GameRuntime(new GameOptions { Logger = record => {
            var active = Volatile.Read(ref current);
            if (active == null) return;
            try { Close(active); }
            catch (InvalidOperationException) { Interlocked.Increment(ref rejected); }
            _ = active.MetricsSnapshot();
            Interlocked.Increment(ref finished);
        } });
        Volatile.Write(ref current, runtime);
        runtime.Poll();
        Require(SpinWait.SpinUntil(() => Volatile.Read(ref finished) > 0, 5000), "callback Close not rejected");
        Require(rejected > 0, "callback Close did not throw InvalidOperationException");
        Volatile.Write(ref current, null);
        // Finish callbacks before testing external closure; no caller should race checked Close.
        runtime.Dispose();
        Require(runtime.LoggerExceptions == 0, "callback error escaped");
        Console.WriteLine("PASS checked close: callback shutdown rejected before touching native lifetime");
    }
}
