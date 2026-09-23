# C# 游戏 SDK

`csharp/RNet` 直接调用同一份 C 游戏 ABI，不另写握手、心跳或加密协议。当前验收范围是 **Linux x86_64、.NET 8+**；不声称已支持 Unity/IL2CPP、NativeAOT、Windows、macOS 或移动平台。SDK 拒绝 32 位进程。

## 构建与运行

```sh
cargo build -p rnet-ffi
LD_LIBRARY_PATH="$PWD/target/debug" dotnet run --project csharp/RNet.Smoke -c Release
# 或者
make csharp-test
# 校验打包后的托管 DLL 与 native 库，不重新编译 SDK 源码：
LD_LIBRARY_PATH="$PWD/dist/linux-x86_64/lib" dotnet run \
  --project dist/linux-x86_64/csharp/RNet.Smoke -c Release \
  -p:RNetDll="$PWD/dist/linux-x86_64/lib/RNet.dll"
```

应用项目引用 `csharp/RNet/RNet.csproj`。部署时使用同一发布包中的 `RNet.dll` 和 `librnet.so`，把 native 库放在应用控制的、不可被普通用户写入的加载目录；不要混用旧版 `.so`。新增入口不改变 C ABI 版本号，因此 ABI=1 不代表旧库具备全部新符号。发布包提供 SDK 源码、托管 DLL 和可执行测试的源码。

以下是同进程的完整收发示例；独立服务器和客户端使用相同 API，仅把各自的 runtime 和 poll 循环分到两端。切换传输只改 `transport`。线上服务器私钥应从安全存储加载，客户端通过可信渠道取得服务器公钥，不能从未认证连接获取公钥后直接信任。

```csharp
using System.Text;
using RNet;

using var serverKey = Keypair.Generate(); // 演示；生产应使用持久化身份。
using var clientKey = Keypair.Generate();
using var server = new GameRuntime(new GameOptions {
    Logger = log => Console.Error.WriteLine(
        $"{log.EventName} session={log.Session} code={log.ErrorCode} {log.Message}")
});
using var client = new GameRuntime(null, clientKey, serverKey.PublicKey);
server.SetMetricsLogInterval(30_000);
var transport = Transport.Kcp;
var listener = server.Listen(new ServerOptions(serverKey) {
    Transport = transport, Host = "127.0.0.1", Port = 0, ProtocolId = 42, Version = 1
});
client.Connect(new ClientOptions {
    Transport = transport, Host = "localhost", Port = server.LocalPort(listener),
    ProtocolId = 42, Version = 1, JoinTicket = Encoding.UTF8.GetBytes("demo-only")
});

bool received = false;
var deadline = DateTime.UtcNow.AddSeconds(10);
while (!received && DateTime.UtcNow < deadline) {
    foreach (var e in server.Poll(64, 1)) {
        switch (e.Type) {
            case GameEventType.AuthRequest:
                // 仅为演示。生产必须校验票据签名、过期时间、用途和身份绑定。
                server.AuthDecide(e.Session, Encoding.UTF8.GetString(e.Data) == "demo-only");
                break;
            case GameEventType.Message:
                server.Send(e.Session, e.Data);
                break;
            case GameEventType.JoinFailed:
            case GameEventType.EndpointError:
                throw new Exception($"server event={e.Type} code={e.Status}");
        }
    }
    foreach (var e in client.Poll(64, 1)) {
        switch (e.Type) {
            case GameEventType.SessionReady:
                client.Send(e.Session, Encoding.UTF8.GetBytes("hello"));
                break;
            case GameEventType.Message:
                Console.WriteLine(Encoding.UTF8.GetString(e.Data));
                received = true;
                break;
            case GameEventType.JoinFailed:
            case GameEventType.EndpointError:
                throw new Exception($"client event={e.Type} code={e.Status}");
        }
    }
}
if (!received) throw new TimeoutException("echo did not complete");
```

## 必须遵守的调用契约

- `Send(session, payload)` 不传传输类型、业务类型或 stream ID；收包是 `GameEvent.Data`，其字节由调用方协议解释。只在 `SessionReady` / `SessionResumed` 后发送。
- `Poll` 持续驱动心跳、协商、时钟采样、发送队列和日志摘要。`Poll(0)` 做非阻塞维护；它不能替代正常取出应用事件，否则队列最终会背压。
- `Send` 成功仅表示本地入队，**不表示送达**。`RNetException.Code == -4` 表示背压，保留数据并继续 poll 后重试；不能无限忙重试。UDP 仍可能丢包乱序，可靠消息选 TCP/KCP。
- 所有接收数据已复制成托管数组，SDK 在 `finally` 中释放原生 token。不要把 `Data`、`AuxiliaryData` 或票据写入通用日志。用完敏感数组后主动清零。
- 创建时通过 `GameOptions` 设置线程数、帧大小、队列字节/条数、连接和握手上限、超时、TCP buffer/NoDelay、rekey 阈值等。nullable 网络选项的 `null` 使用 native 默认；显式非法的零值由 native 校验。游戏队列字段的零值使用默认预算，不能用零取消所有限制。创建参数不支持在活跃会话上随意修改。
- 服务端 `SetEncryption(session, bool)` / `Rekey(session)` 是异步操作，等待 `SecurityChanged`；客户端没有加密选择项。默认拒绝明文业务。确有需求时两端创建配置均需 `AllowPlaintextBusinessData=true`，服务端才能关闭业务加密；控制消息仍受保护。明文业务**没有机密性或完整性**，不适合公网敏感数据。
- `SendLatest` 按 session/key 尽力替换尚未交给 I/O worker 的快照。进入 socket 或 KCP 缓存后不可撤回。带 `GameSendOptions` 的发送可设置 priority、expiry、tick、sequence、correlation；expiry 也只作用于本地暂存。
- `ListenRange` / `ConnectRange` 显式启用 wire v4；普通接口仍用精确版本 wire v3。`SelectedProtocolVersion` 查询服务端选定版本。
- `IssueResumeTicket` → `ResumeTicket` → `ConnectResume` / `ConnectRangeResume` → 服务端 `ResumeRequest` 再授权 → `SessionResumed`。恢复分配新句柄，通过 `RelatedSession` 映射旧句柄；失败不代表旧会话已失效。票据一次性且限同一服务端 runtime，不能跨进程恢复。
- `CloseSession` 踢会话，`CloseEndpoint` 关闭端点。`Stop` 的 `-4` 需 poll 后重试；`Dispose` 最终停止并丢弃未交付事件。务必 `using` / 显式 Dispose；不要依赖 GC，特别是 logger 闭包反向引用 runtime 时。与 Dispose 重叠的调用受 SafeHandle lease 保护；Dispose 不保证业务送达。

## 接入自己的日志与监控

把日志组件适配为 `Action<LogRecord>`，通过 `GameOptions.Logger` 注入。回调在 native 有界日志线程上执行，应快速返回；可查询指标，禁止回调内 Stop/Dispose（SDK 抛 `InvalidOperationException` 防自等待）。异常不会越过 native 边界，`LoggerExceptions` 和 `rnet_game_csharp_logger_exceptions_total` 记录次数。队列拥塞丢日志而不阻塞网络；务必监控丢弃数。

`SetMetricsLogInterval(ms)` 默认 30 秒，0 关闭，由 poll 触发 `game_latency_summary`：累计样本 count、P90/P95/P99/P99.9/max（微秒）、发送队列和 logger 丢弃/异常。不是每包日志，也不是滑动窗口 SLO；无样本的零不能解释为零延迟。

`MetricsSnapshot()` 返回类型化游戏指标，`PrometheusSnapshot()` 覆盖传输/排队/恢复/协商等完整文本指标。`QualitySnapshot(session)` 和 `ClockSyncSnapshot(session)` 未采样返回 null。时钟偏移是含排队的近似传输测量，不是可信反外挂时钟。日志以 runtime/endpoint/session/correlation 关联事件；聚合指标不包含玩家标签。

完整多传输、动态加密、重连、版本范围、GC 与回调测试见 [RNet.Smoke](../csharp/RNet.Smoke/Program.cs)。
