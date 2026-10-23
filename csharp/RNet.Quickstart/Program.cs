using RNet;
using RNet.Quickstart;

// Reject invalid options before acquiring native resources. No arguments means KCP.
Transport transport = Transport.Kcp;
bool deny = false, oneSlot = false;
try {
    if (args.Length > 0) transport = args[0] switch {
        "tcp" => Transport.Tcp, "udp" => Transport.Udp, "kcp" => Transport.Kcp,
        _ => throw new ArgumentException("Unknown transport")
    };
    foreach (var flag in args.Skip(1)) {
        if (flag == "--deny" && !deny) deny = true;
        else if (flag == "--one-slot" && !oneSlot) oneSlot = true;
        else throw new ArgumentException("Unknown or duplicate option");
    }
} catch (ArgumentException error) {
    Console.Error.WriteLine($"{error.Message}; usage: RNet.Quickstart [tcp|udp|kcp] [--one-slot] [--deny]");
    return 2;
}

try {
    var report = GameExample.Run(transport, deny, oneSlot);
    Console.WriteLine($"game echo complete: received={report.Received} blocked={report.Blocked}; both Stop/Dispose sequences completed");
    return 0;
} catch (Exception error) {
    ReportError(error);
    return 1;
}

// Aggregation must not hide the original status behind a later cleanup failure.
static void ReportError(Exception error) {
    if (error is AggregateException aggregate) {
        foreach (var inner in aggregate.InnerExceptions) ReportError(inner);
        return;
    }
    Console.Error.WriteLine(error is RNetException native
        ? $"example code={native.Code}: {native.Message}" : $"example {error.GetType().Name}: {error.Message}");
    if (error.InnerException != null) ReportError(error.InnerException);
}
