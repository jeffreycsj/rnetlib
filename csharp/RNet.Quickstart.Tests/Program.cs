using System.Diagnostics;
using RNet;
using RNet.Quickstart;

static void Require(bool condition, string message) { if (!condition) throw new Exception(message); }
static T Throws<T>(Action action) where T : Exception {
    try { action(); } catch (T error) { return error; }
    throw new Exception($"Expected {typeof(T).Name}");
}
static long Deadline() => Stopwatch.GetTimestamp() + 10 * Stopwatch.Frequency;

var tests = new (string Name, Action Run)[] {
    ("copy, binary, empty, retry", () => {
        var pending = new PendingSends();
        byte[] source = [7, 0, 255];
        pending.Stage(source); pending.Stage([]); source[0] = 9;
        Require(pending.Flush(Deadline(), _ => throw new RNetException(-4, "full")), "WouldBlock must defer");
        var received = new List<byte[]>();
        Require(!pending.Flush(Deadline(), p => received.Add(p.ToArray())), "successful flush blocked");
        Require(received.Count == 2 && received[0].SequenceEqual(new byte[] { 7, 0, 255 }) && received[1].Length == 0, "copy/FIFO lost");
        pending.Flush(Deadline(), _ => throw new Exception("already accepted message repeated"));
    }),
    ("size/count limits and accepted prefix", () => {
        var pending = new PendingSends();
        Throws<InvalidOperationException>(() => pending.Stage(new byte[129]));
        pending.Stage(new byte[128]); pending.Stage([]);
        Throws<InvalidOperationException>(() => pending.Stage([]));
        int calls = 0;
        Require(pending.Flush(Deadline(), _ => { if (++calls == 2) throw new RNetException(-4, "full"); }), "suffix should wait");
        calls = 0;
        pending.Flush(Deadline(), p => { calls++; Require(p.Length == 0, "accepted prefix repeated"); });
        Require(calls == 1, "pending empty message lost");
    }),
    ("expiry and terminal errors retain pending copy", () => {
        var pending = new PendingSends(); pending.Stage([1]);
        Throws<TimeoutException>(() => pending.Flush(Stopwatch.GetTimestamp() - 1, _ => throw new Exception("expired send attempted")));
        var terminal = new RNetException(-3, "session retired");
        Require(ReferenceEquals(terminal, Throws<RNetException>(() => pending.Flush(Deadline(), _ => throw terminal))), "native reason replaced");
        var allocation = new OutOfMemoryException("injected callback failure");
        Require(ReferenceEquals(allocation, Throws<OutOfMemoryException>(() => pending.Flush(Deadline(), _ => throw allocation))), "non-native error replaced");
        int calls = 0;
        pending.Flush(Deadline(), p => { calls++; Require(p.SequenceEqual(new byte[] { 1 }), "pending payload changed"); });
        Require(calls == 1, "failed send removed message");
    }),
    ("cleanup preserves primary and attempts both peers", () => {
        var primary = new RNetException(-13, "auth denied");
        int calls = 0;
        Require(ReferenceEquals(primary, Throws<RNetException>(() => Cleanup.Finish(primary, () => calls++, () => calls++))), "primary replaced");
        Require(calls == 2, "peer cleanup skipped");
        var first = new Exception("first cleanup"); var second = new Exception("second cleanup");
        var joined = Throws<AggregateException>(() => Cleanup.Finish(primary, () => throw first, () => throw second));
        Require(joined.InnerExceptions.SequenceEqual(new[] { primary, first, second }), "failure order/diagnostics lost");
        Require(ReferenceEquals(first, Throws<Exception>(() => Cleanup.Finish(null, () => throw first))), "cleanup failure hidden");
        Cleanup.Finish(null, () => { });
    })
};
int failures = 0;
foreach (var test in tests) {
    try { test.Run(); Console.WriteLine($"PASS {test.Name}"); }
    catch (Exception error) { failures++; Console.Error.WriteLine($"FAIL {test.Name}: {error}"); }
}
return failures == 0 ? 0 : 1;
