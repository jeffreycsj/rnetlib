using RNet;
using System.Reflection;
using System.Runtime.InteropServices;

// Exercise the public SDK against real native lifetimes; no shutdown or logger mocks.
internal static class LifecycleTests {
    private static void Require(bool condition, string message) {
        if (!condition) throw new Exception(message);
    }

    internal static void Run() {
        CallbackShutdownIsRejected();
        DisposeWaitsForBlockedLogger();
        StopRetainsFinalEventsAndRejectsNewEndpoints();
    }

    private static void CallbackShutdownIsRejected() {
        GameRuntime? current = null;
        int stopRejected = 0, disposeRejected = 0, queried = 0;
        using var runtime = new GameRuntime(new GameOptions { Logger = record => {
            var active = Volatile.Read(ref current);
            if (active == null) return;
            try { active.Stop(); }
            catch (InvalidOperationException) { Interlocked.Increment(ref stopRejected); }
            try { active.Dispose(); }
            catch (InvalidOperationException) { Interlocked.Increment(ref disposeRejected); }
            _ = active.MetricsSnapshot();
            Interlocked.Increment(ref queried);
        } });
        Volatile.Write(ref current, runtime);
        runtime.Poll();
        Require(SpinWait.SpinUntil(() => Volatile.Read(ref queried) > 0, 5000), "logger query did not finish");
        Require(Volatile.Read(ref stopRejected) > 0 && Volatile.Read(ref disposeRejected) > 0,
            "callback Stop and Dispose must both be rejected");
        Require(runtime.LoggerExceptions == 0, "callback shutdown/query escaped its expected error path");
        runtime.Stop();
        _ = runtime.MetricsSnapshot();
        Console.WriteLine("PASS lifecycle: callback Stop/Dispose rejected, metrics and later Stop remain usable");
    }

    private static void DisposeWaitsForBlockedLogger() {
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        using var exited = new ManualResetEventSlim();
        int armed = 0;
        using var runtime = new GameRuntime(new GameOptions { Logger = record => {
            if (Interlocked.CompareExchange(ref armed, 2, 1) != 1) return;
            entered.Set();
            try {
                if (!release.Wait(10000)) throw new TimeoutException("test logger release deadline");
            } finally { exited.Set(); }
        } });
        // Observe SafeHandle closure without acquiring a new native lease. A metrics probe could
        // otherwise become the final lease holder and run cleanup on the test thread itself.
        var owner = typeof(GameRuntime).GetField("owner", BindingFlags.Instance | BindingFlags.NonPublic)
            ?.GetValue(runtime) as SafeHandle ?? throw new Exception("runtime SafeHandle unavailable");
        Task? disposal = null;
        try {
            Volatile.Write(ref armed, 1);
            runtime.Poll();
            Require(entered.Wait(5000), "logger did not enter blocking section");
            disposal = Task.Run(runtime.Dispose);
            Require(SpinWait.SpinUntil(() => owner.IsClosed, 5000), "Dispose did not close the managed owner");
            try {
                _ = runtime.MetricsSnapshot();
                throw new Exception("disposed runtime accepted a new native lease");
            } catch (ObjectDisposedException) { }
            Require(!disposal.IsCompleted && !exited.IsSet,
                "native cleanup completed while its logger callback was still blocked");
            // Repeated disposal must not double-destroy or free logger user data.
            var repeated = Task.Run(runtime.Dispose);
            Require(repeated.Wait(5000), "overlapping Dispose blocked on duplicate cleanup");
            release.Set();
            Require(disposal.Wait(5000) && exited.IsSet, "Dispose did not complete after logger returned");
            Require(runtime.LoggerExceptions == 0, "logger context was lost or callback timed out");
            runtime.Dispose();
        } finally {
            release.Set();
            if (disposal != null) Require(disposal.Wait(5000), "cleanup task remained blocked");
        }
        Console.WriteLine("PASS lifecycle: blocked logger retained until native teardown, overlapping Dispose is safe");
    }

    private static void StopRetainsFinalEventsAndRejectsNewEndpoints() {
        using var key = Keypair.Generate();
        using var runtime = new GameRuntime();
        runtime.Stop();
        runtime.Stop();
        int stopped = runtime.Poll().Count(e => e.Type == GameEventType.RuntimeStopped);
        Require(stopped == 1, "Stop must retain exactly one final event when queue has space");
        _ = runtime.MetricsSnapshot();
        try {
            runtime.Listen(new ServerOptions(key));
            throw new Exception("stopped runtime accepted a new listener");
        } catch (RNetException error) {
            Require(error.Code == -3, $"stopped listen error was {error.Code}, expected InvalidState");
        }
        runtime.Stop();
        Require(runtime.Poll().All(e => e.Type != GameEventType.RuntimeStopped), "repeated Stop duplicated final event");
        runtime.Dispose();
        try {
            runtime.Stop();
            throw new Exception("disposed runtime allowed Stop");
        } catch (ObjectDisposedException) { }
        Console.WriteLine("PASS lifecycle: stop preserves final polling, prevents restart, dispose invalidates calls");
    }
}
