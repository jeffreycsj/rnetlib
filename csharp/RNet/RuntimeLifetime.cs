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
        Native.EventV2* events = stackalloc Native.EventV2[64];
        // No caller can enter once SafeHandle is closed. Queue capacity, rather than time, bounds
        // the remaining work. WOULD_BLOCK means the final stopped event needs queue space.
        int status;
        while ((status = Native.rnet_game_runtime_stop(runtime, 0)) == -4) {
            int polled = Native.rnet_game_poll_events_v2(runtime, events, 64, 0, out var count);
            ReleaseBuffers(runtime, events, count);
            if (polled != 0) return false; // retain callback root if native teardown did not complete
        }
        if (status != 0 || Native.rnet_game_runtime_destroy(runtime) != 0) return false;
        if (context != 0) GCHandle.FromIntPtr(context).Free();
        return true;
    }
    internal static void ReleaseBuffers(ulong runtime, Native.EventV2* events, nuint count) {
        for (nuint i = 0; i < count; i++) {
            if (events[i].Base.Token != 0) Native.rnet_game_buffer_release(runtime, events[i].Base.Token);
            if (events[i].Base.AuxToken != 0) Native.rnet_game_buffer_release(runtime, events[i].Base.AuxToken);
        }
    }
}

internal readonly struct RuntimeLease : IDisposable {
    private readonly RuntimeOwner owner;
    internal readonly ulong Value;
    internal RuntimeLease(RuntimeOwner owner) {
        bool acquired = false;
        owner.DangerousAddRef(ref acquired);
        this.owner = owner;
        Value = unchecked((ulong)owner.DangerousGetHandle());
    }
    public void Dispose() => owner.DangerousRelease();
}
