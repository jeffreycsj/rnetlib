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

应用项目引用 `csharp/RNet/RNet.csproj`。部署时使用同一发布包中的 `RNet.dll` 和 `librnet.so`，把 native 库放在应用控制的、不可被普通用户写入的加载目录；不要混用旧版 `.so`。只接受当前 SDK/native ABI，不兼容旧 SDK；ABI 数值相同也不能代替同批发布。发布包提供 SDK 源码、托管 DLL 和可执行测试的源码。

构建自动生成 `RNet.xml`，公开 API 缺少 XML 注释会以 CS1591 构建失败。直接引用 DLL 时将 `RNet.xml` 放在 DLL 同目录，IDE 可展示参数、返回值、单位和生命周期说明；发布脚本会一并收录。文档检查不能代替运行时与平台验证。

## 完整游戏接入示例

[GameExample.cs](../csharp/RNet.Quickstart/GameExample.cs) 使用两个独立 runtime 演示
建立、授权、等待游戏就绪、二进制和空消息回显、日志与关闭；
[PendingSends.cs](../csharp/RNet.Quickstart/PendingSends.cs) 单独实现应用重试及异常汇合，
[Program.cs](../csharp/RNet.Quickstart/Program.cs) 负责参数和退出码。独立部署时，双方使用
相同 API，将各自的 runtime 与 poll 循环放到对应进程即可。

```sh
make csharp-test
LD_LIBRARY_PATH="$PWD/target/debug" dotnet run --project csharp/RNet.Quickstart -c Release -- tcp --one-slot
LD_LIBRARY_PATH="$PWD/target/debug" dotnet run --project csharp/RNet.Quickstart -c Release -- udp
LD_LIBRARY_PATH="$PWD/target/debug" dotnet run --project csharp/RNet.Quickstart -c Release -- kcp --deny
```

无参数默认 KCP；`--deny` 预期以 `AuthRejected`（-13）和退出码 1 失败，非法参数退出 2。
`--one-slot` 使用真实的一条消息 scheduler 预算，三传输测试要求出现背压且最终收到两个字节匹配的回显。
每端应用队列最多两条、每条 128 字节，空消息也占一条；发送失败不出队，WouldBlock 回到正容量 poll，
不会仅依赖 Writable 或无限忙重试。UDP 仍可能丢失/重复/乱序，严格回显失败不代表库承诺可靠 UDP。
线上多人队列需要另外制定每玩家预算、截止与过载策略。

交换的十秒截止使用单调 Stopwatch；清理各有五秒 Stop 背压重试期限。这些限制约束重试次数/时机，
不能抢占原生调用、Dispose 或阻塞的日志组件。成功输出表示收到回显并完成显式 Stop/Dispose 调用序列，
**不表示已检查 SafeHandle 内部的原生销毁结果**。即使第一端清理失败仍尝试第二端；单异常保留原类型和
堆栈，多异常用 AggregateException 按主错误在前的顺序保存，命令行逐项打印 native 错误类别。

示例保持加密。服务器私钥应从安全存储加载并持久化，客户端通过可信渠道获得公钥固定值；
不可把演示票据当作生产认证，必须校验签名/过期/用途并绑定已认证客户端公钥。
Keypair 用完 Dispose，复制出的敏感数组另行清零。日志不打印业务载荷或凭证；Console.Error
只是串行化演示日志接入，生产换成快速的有界日志组件。默认 30 秒摘要含 P95/P99 和样本数，
短回显通常不会等到摘要输出。

发布包包含示例源码及项目文件；从包根目录引用同批 DLL 和 native 库运行，无需重建 Rust/SDK：

```sh
LD_LIBRARY_PATH="$PWD/lib" dotnet run --project csharp/RNet.Quickstart -c Release \
  -p:RNetDll="$PWD/lib/RNet.dll" -- tcp --one-slot
```

`make csharp-test` 保留原有动态加密/换钥、恢复、版本协商、监控和回调回归；另运行无外部 NuGet
测试依赖的队列/清理契约测试及 Python 3 CLI 检查。短回环不替代目标平台或长稳验证。

## 必须遵守的调用契约

- `Send(session, payload)` 不传传输类型、业务类型或 stream ID；收包是 `GameEvent.Data`，其字节由调用方协议解释。只在 `SessionReady` / `SessionResumed` 后发送。
- `Poll` 持续驱动心跳、协商、时钟采样、发送队列和日志摘要。`Poll(0)` 做非阻塞维护；它不能替代正常取出应用事件，否则队列最终会背压。
- `Send` 成功仅表示本地入队，**不表示送达**。`RNetException.Code == -4` 表示背压，保留数据并继续 poll 后重试；不能无限忙重试。UDP 仍可能丢包乱序，可靠消息选 TCP/KCP。
- 所有接收数据已复制成托管数组，SDK 在 `finally` 中释放原生 token。不要把 `Data`、`AuxiliaryData` 或票据写入通用日志。用完敏感数组后主动清零。
- 创建时通过 `GameOptions` 设置线程数、帧大小、队列字节/条数、连接和握手上限、超时、TCP buffer/NoDelay、rekey 阈值等。nullable 网络选项的 `null` 使用 native 默认；显式非法的零值由 native 校验。游戏队列字段的零值使用默认预算，不能用零取消所有限制。创建参数不支持在活跃会话上随意修改。
- 服务端 `SetEncryption(session, bool)` / `Rekey(session)` 是异步操作，以 `SecurityChanged` 确认本地完成；请求已经启用的模式可能不产生新事件。客户端没有加密选择项。默认拒绝服务端明文策略；确有需求时服务端 runtime 设置 `AllowPlaintextBusinessData=true`，客户端自动跟随，不要求设置该标记。控制消息仍受保护。明文业务**没有机密性或完整性**，不适合公网敏感数据。
- `SendLatest` 按 session/key 尽力替换尚未交给 I/O worker 的快照。进入 socket 或 KCP 缓存后不可撤回。带 `GameSendOptions` 的发送可设置 priority、expiry、tick、sequence、correlation；expiry 也只作用于本地暂存。
- `Listen` / `Connect` 的配置统一使用 `MinVersion` / `MaxVersion` 闭区间，两者相等表示精确业务版本（默认均为 1）。双方须持续 poll 至游戏就绪；`SelectedProtocolVersion` 查询服务端选定版本。没有独立 Range 入口；旧 SDK/ABI 不兼容，必须与当前 native 库一起重建。
- `IssueResumeTicket` → `ResumeTicket` → `ConnectResume` → 服务端 `ResumeRequest` 再授权 → `SessionResumed`。恢复分配新句柄，通过 `RelatedSession` 映射旧句柄；失败不代表旧会话已失效。票据一次性且限同一服务端 runtime，不能跨进程恢复；新的版本范围须包含票据绑定的原选定版本。
- `CloseSession` 踢会话，`CloseEndpoint` 关闭端点。`Stop` 的 `-4` 需 poll 后重试；`Dispose` 最终停止并丢弃未交付事件。务必 `using` / 显式 Dispose；不要依赖 GC，特别是 logger 闭包反向引用 runtime 时。与 Dispose 重叠的调用受 SafeHandle lease 保护，最后一个在途调用结束后才销毁 native，因此 Dispose 返回不一定代表回调已退出；先停止业务调用并协调回调引用的外部资源生命周期。Dispose 不保证送达，也不通过 RNetException 报告 SafeHandle 内部清理失败。

## 接入自己的日志与监控

把日志组件适配为 `Action<LogRecord>`，通过 `GameOptions.Logger` 注入。回调在 native 有界日志线程上执行，应快速返回；可查询指标，禁止回调内 Stop/Dispose（SDK 抛 `InvalidOperationException` 防自等待）。异常不会越过 native 边界，`LoggerExceptions` 和 `rnet_game_csharp_logger_exceptions_total` 记录次数。队列拥塞丢日志而不阻塞网络；务必监控丢弃数。

`SetMetricsLogInterval(ms)` 默认 30 秒，0 关闭，由 poll 触发 `game_latency_summary`：累计样本 count、P90/P95/P99/P99.9/max（微秒）、发送队列和 logger 丢弃/异常。不是每包日志，也不是滑动窗口 SLO；无样本的零不能解释为零延迟。

`MetricsSnapshot()` 返回类型化游戏指标，`PrometheusSnapshot()` 覆盖传输/排队/恢复/协商等完整文本指标。`QualitySnapshot(session)` 和 `ClockSyncSnapshot(session)` 未采样返回 null。时钟偏移是含排队的近似传输测量，不是可信反外挂时钟。日志以 runtime/endpoint/session/correlation 关联事件；聚合指标不包含玩家标签。

完整多传输、动态加密、重连、版本范围、GC 与回调测试见 [RNet.Smoke](../csharp/RNet.Smoke/Program.cs)。
