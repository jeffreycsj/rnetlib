using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace RNet;

internal sealed class LogState {
    internal readonly Action<LogRecord> Sink;
    internal long Exceptions;
    internal LogState(Action<LogRecord> sink) { Sink = sink; }
}

internal static unsafe class LogBridge {
    [ThreadStatic] internal static bool InCallback;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    internal delegate void Callback(nint user, ulong timestamp, uint level, byte* name, nuint nameLen,
        ulong runtime, ulong endpoint, ulong session, uint transport, int error, ulong correlation,
        byte* message, nuint messageLen);
    // The native library keeps this pointer after create returns; root it for process lifetime.
    internal static readonly Callback Function = Invoke;
    private static void Invoke(nint user, ulong timestamp, uint level, byte* name, nuint nameLen,
        ulong runtime, ulong endpoint, ulong session, uint transport, int error, ulong correlation,
        byte* message, nuint messageLen) {
        var state = (LogState)GCHandle.FromIntPtr(user).Target!;
        bool previous = InCallback;
        InCallback = true;
        try { state.Sink(new LogRecord(timestamp, (LogLevel)level, Native.Text(name, nameLen),
            runtime, endpoint, session, (Transport)transport, error, correlation, Native.Text(message, messageLen))); }
        catch (Exception) { Interlocked.Increment(ref state.Exceptions); }
        finally { InCallback = previous; }
    }
}

/// <summary>SafeHandle leases cover native calls AND copying/releasing returned buffers.</summary>
internal sealed unsafe class RuntimeOwner : SafeHandleZeroOrMinusOneIsInvalid {
    private readonly nint logContext;
    internal RuntimeOwner(ulong runtime, nint context) : base(true) {
        SetHandle(unchecked((nint)runtime)); logContext = context;
    }
    private readonly object gate = new();
    private int leases;
    private bool closing, disposed, checkedClosed;

    internal ulong AcquireLease() {
        lock (gate) {
            if (closing) throw new RNetException(-4, "game runtime is closing");
            ObjectDisposedException.ThrowIf(disposed || IsClosed, this);
            bool acquired = false;
            DangerousAddRef(ref acquired);
            leases++;
            return unchecked((ulong)handle);
        }
    }
    internal void ReleaseLease() {
        // DangerousRelease can run native cleanup and join the logger. Never hold gate here.
        try { DangerousRelease(); }
        finally { lock (gate) { leases--; } }
    }
    internal void DisposeRuntime() {
        lock (gate) {
            if (closing) throw new InvalidOperationException("Dispose cannot overlap checked Close.");
            if (disposed || checkedClosed) return;
            disposed = true;
        }
        Dispose();
    }
    internal void Close(uint drainTimeoutMs) {
        lock (gate) {
            if (checkedClosed) return;
            ObjectDisposedException.ThrowIf(disposed || IsClosed, this);
            if (closing) throw new RNetException(-4, "game runtime is closing");
            if (leases != 0) throw new RNetException(-4, "game runtime has active managed calls");
            closing = true;
        }
        try {
            // No new managed leases or Dispose can enter. Do not keep gate across native teardown:
            // the logger may query this wrapper and must receive an error without waiting for us.
            ulong runtime = unchecked((ulong)handle);
            Native.Check(Native.rnet_game_runtime_stop(runtime, drainTimeoutMs));
            Native.Check(Native.rnet_game_runtime_destroy(runtime));
            // Destroy succeeded and joined native callbacks. Transfer ownership before freeing their root.
            SetHandleAsInvalid();
            if (logContext != 0) GCHandle.FromIntPtr(logContext).Free();
            lock (gate) { checkedClosed = true; }
        } finally {
            lock (gate) { closing = false; }
            // Direct P/Invoke uses the numeric handle, so retain its owner through the entire call.
            GC.KeepAlive(this);
        }
    }
    protected override bool ReleaseHandle() {
        ulong runtime = unchecked((ulong)handle);
        // A concurrent Dispose may release the last lease inside a logger's metrics query.
        // Native shutdown joins that thread, so hand cleanup to another thread in this case.
        if (LogBridge.InCallback) {
            ThreadPool.QueueUserWorkItem(_ => Cleanup(runtime, logContext));
            return true;
        }
        return Cleanup(runtime, logContext);
    }
    private static bool Cleanup(ulong runtime, nint context) {
        Native.Event* events = stackalloc Native.Event[64];
        // No caller can enter once SafeHandle is closed. Queue capacity, rather than time, bounds
        // the remaining work. WOULD_BLOCK means the final stopped event needs queue space.
        int status;
        while ((status = Native.rnet_game_runtime_stop(runtime, 0)) == -4) {
            int polled = Native.rnet_game_poll_events(runtime, events, 64, 0, out var count);
            ReleaseBuffers(runtime, events, count);
            if (polled != 0) return false; // retain callback root if native teardown did not complete
        }
        if (status != 0 || Native.rnet_game_runtime_destroy(runtime) != 0) return false;
        if (context != 0) GCHandle.FromIntPtr(context).Free();
        return true;
    }
    internal static void ReleaseBuffers(ulong runtime, Native.Event* events, nuint count) {
        for (nuint i = 0; i < count; i++) {
            if (events[i].Token != 0) Native.rnet_game_buffer_release(runtime, events[i].Token);
            if (events[i].AuxToken != 0) Native.rnet_game_buffer_release(runtime, events[i].AuxToken);
        }
    }
}

internal readonly struct RuntimeLease : IDisposable {
    private readonly RuntimeOwner owner;
    internal readonly ulong Value;
    internal RuntimeLease(RuntimeOwner owner) {
        Value = owner.AcquireLease();
        this.owner = owner;
    }
    public void Dispose() => owner.ReleaseLease();
}
