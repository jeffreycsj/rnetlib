using System.Diagnostics;
using System.Runtime.ExceptionServices;

namespace RNet.Quickstart;

// Single-dispatcher application queue, not SDK API. Empty messages occupy a slot;
// each peer holds at most two owned copies of at most 128 bytes each.
internal sealed class PendingSends {
    private readonly Queue<byte[]> items = new(); // FIFO values not yet admitted by native Send.

    internal void Stage(ReadOnlySpan<byte> payload) {
        if (payload.Length > 128 || items.Count >= 2)
            throw new InvalidOperationException("Example pending queue limit exceeded");
        items.Enqueue(payload.ToArray());
    }

    // deadline is an absolute Stopwatch timestamp. True means WouldBlock: resume
    // positive-capacity Poll before retrying, not only when Writable is observed.
    // send borrows the array; it must not mutate/reenter this queue. The deadline
    // bounds new attempts, not callback execution time. Failed admissions stay queued.
    internal bool Flush(long deadline, Action<byte[]> send) {
        while (items.TryPeek(out var payload)) {
            if (Stopwatch.GetTimestamp() >= deadline) throw new TimeoutException("Pending send deadline exceeded");
            try { send(payload); }
            catch (RNetException error) when (error.Code == -4) { return true; }
            items.Dequeue();
        }
        return false;
    }
}

internal static class Cleanup {
    // Attempts every closer even after a primary/cleanup failure. A single error
    // preserves its original type/stack; multiple errors retain primary-first order.
    internal static void Finish(Exception? primary, params Action[] closers) {
        var failures = new List<Exception>();
        if (primary != null) failures.Add(primary);
        foreach (var close in closers) {
            try { close(); } catch (Exception error) { failures.Add(error); }
        }
        if (failures.Count == 1) ExceptionDispatchInfo.Capture(failures[0]).Throw();
        if (failures.Count > 1) throw new AggregateException("Operation/cleanup failures", failures);
    }
}
