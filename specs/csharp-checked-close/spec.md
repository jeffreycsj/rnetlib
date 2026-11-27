# C# 显式关闭与失败重试

状态：2026-11-27，用户已确认；Close 和并发所有权保护已实现，本地回归及独立 DLL 消费验证通过。延续多语言契约复查中的 C# 清理失败缺口。

## 目标与当前行为

当前 `GameRuntime.Dispose()` 关闭 SafeHandle，`ReleaseHandle` 内执行 stop/destroy。原生清理失败只返回 false，调用方既看不到 RNetException，也不能通过已关闭的托管句柄重试；logger GCHandle 必须继续保留以避免 native 回调访问已释放对象。现有 Stop 可以报告停止失败，但不能证明 destroy 成功。

增加明确的关闭入口，复用当前原生 stop/destroy ABI。成功代表 native runtime 已销毁、日志回调已结束且 logger context 已释放；失败保留所有权供处理原因后重试。不改变传输、协议或 Rust/C/C++/Go API，不移除 Dispose 和 using 支持。

## 公开接口与行为

```csharp
public void Close(uint drainTimeoutMs = 0);
```

- Close 依次执行原生 stop 与 destroy。drainTimeoutMs 是 stop 的发送排空预算，不是整个 Close 的硬超时；零表示不等待发送排空。
- 原生失败立即读取同线程诊断并抛出 RNetException，保留原错误码及原因。不吞掉错误、不在 Close 内无限 poll/retry，不把失败变成成功返回。
- 成功后释放 logger context，禁止重复 native destroy；重复 Close 和后续 Dispose 无害。普通 SDK 操作抛 ObjectDisposedException。
- 失败保留 runtime 和 logger context。若 stop 已成功，仍可 Poll/查询及重试 Close，但不能恢复监听、连接或发送，不承诺业务状态回滚。
- logger 回调内 Close 与现有 Stop/Dispose 一样抛 InvalidOperationException，避免关闭等待自己。
- Close 已在执行时，另一个 Close 或新 SDK 调用返回 RNetException(-4, 明确的 closing 原因)，不等持有全局或托管生命周期锁的线程退出。
- 已有 SDK lease 尚未完成时，Close 在开始 native stop 前返回 WouldBlock，调用方停止并发调用者后重试。Close 不等待 lease，避免与正在回调查询的线程相互等待。
- Close 已进入关闭阶段时，重叠 Dispose 抛 InvalidOperationException，避免 Dispose 抢先关闭 SafeHandle、破坏失败后的重试所有权。先前已有的 Dispose/Dispose 重叠契约保持不变。
- Dispose 已经开始时，Close 抛 ObjectDisposedException；不把 SafeHandle 已关闭误报为 checked native destroy 成功。
- 现有 Dispose 仍是兼容的最终释放路径，不新增“Dispose 一定报告清理成功”的保证。需要重试的调用方应在 using 作用域结束前处理 Close 异常；失败后立即退出 using 会进入旧的 Dispose 兜底路径。

## 所有权与并发实现

- RuntimeOwner 保留 SafeHandle 作为 native 所有权基础；新增请求进入与关闭状态保护，并跟踪托管在途 lease，覆盖 native 调用以及结果复制/token 释放全过程。
- lease 获取、Close 开始、Dispose 开始在同一个短临界区仲裁。Close 确认没有 lease 后拒绝新的 lease；不在持锁状态调用 native stop/destroy、执行用户回调或等待 logger。
- 原生关闭期间保持 RuntimeOwner 的强引用，阻止 GC 提前终结。失败恢复可重试状态，SafeHandle 及 logger context 不失效、不释放。
- 原生 destroy 成功后，标记 SafeHandle 不再需要原生释放，记录 checked-close 成功状态，并且仅释放一次 logger context；后续 Dispose/终结器不重复释放。
- Dispose 的在途 lease 延迟清理以及最后一个 lease 在日志回调内释放时转交线程池的既有保护必须保留。
- managed 生命周期拒绝有自己的明确诊断，不读取无关线程遗留的 native last-error。原生异常信息在其他 FFI 调用前复制，避免覆盖。

## 验证方案

保持既有 Smoke、三传输连接/加密/恢复及上一批生命周期测试。新增测试放独立 C# Smoke 文件，不增加 SDK 公共测试入口。

- 真实 native buffer 所有权失败：测试通过真实 ABI 保留一个 Prometheus token，使 destroy 连续两次明确返回 InvalidState。验证 Close 抛原始错误、失败后可查询/Poll，logger context 仍活着；释放 token 后 Close 成功，保存的原生句柄随后无效。
- stop 成功但 destroy 失败后不能重新监听；最终停止事件不重复产生，失败不等于恢复运行。
- 正常 Close、重复 Close、Close 后 Dispose、Dispose 后 Close、回调内 Close、并发 Close 及 Close/Dispose 重叠均检查具体结果。
- 通过受控的日志回调建立关闭窗口；只观察测试对象已有 SafeHandle/必要 lease 状态，不能把 stop/destroy 实现 mock 成期望结果。借助真实 ABI 查询确认成功后 native handle 失效，不只断言托管字段变化。
- 验证有在途 lease 时 Close 不开始停止、失败后保留可用句柄；确认旧 Dispose 最后一个 lease 的释放行为不被破坏。测试清理在 finally 释放 token/放行回调，所有测试等待设置期限。
- 不声称解决永久阻塞 logger：原生 destroy 等待 logger 返回，Close 不能抢占它。OS OOM、Unity/IL2CPP/AOT、目标环境长稳不是本批证据范围。

## 任务

- [x] 确认公开 Close 接口和重叠 Dispose 的错误语义。
- [x] 串行跑 C# 基线，先为失败保留与重试写 RED 回归；接口缺失的编译失败不作为行为修复证据，需记录功能实现后状态/所有权断言的验证结果。
- [x] 实现 RuntimeOwner 生命周期仲裁、lease 计数、Close 错误传播与成功后一次性释放。
- [x] 补充上述真实 native 错误、并发及日志回调测试，保留全部既有 Dispose 回归。
- [x] 更新 XML 注释、C# 接入文档及 Quickstart：成功输出以 checked Close 完成为条件，失败退出保留原始错误，不能仅完成 Dispose 就输出成功。
- [x] make csharp-test/doc-check/structure-check、独立 RNet.dll 消费构建以及对抗性复查；执行时至少保留 15 GiB 可用内存。

发布门槛保持不变：本批完成后仍需其余跨语言高级接口核对、干净候选提交上的发布资格，最后才是目标环境弱网及 24–72 小时长稳。未经用户要求不提交、推送或发布。


## 实施证据与对抗性复核

- 基线：`make -j1 csharp-test DOTNET=/data/tmp/rnet-dotnet/dotnet` 通过，日志 `/tmp/netlib-checked-close-baseline.log`。
- 首次新增契约测试能编译并运行至真实 native token 留存，但因缺少 Close API 明确失败（`/tmp/netlib-checked-close-red.log`）。这是缺失接口的运行断言，不冒充已有 stop/destroy 算法的行为 RED。接口实现后测试改为编译期直接调用 Close。
- 已通过真实 native outstanding-buffer 的连续两次销毁拒绝、错误详情保留、句柄保留、释放 token 后成功、原生句柄失效以及重复 Close/Dispose。未替换 native stop/destroy 返回码。
- active lease 导致 Close 在停止前返回 -4，随后仍能监听，证明拒绝没有偷偷停止业务；Dispose 则立即拒绝新操作，等既有 lease 结束再销毁。
- 关闭等待 logger 时，另一个 Close/查询返回 -4，Dispose 抛明确异常；放行 logger 后回调可收到查询错误并正常退出，证明清理未持有阻塞回调的托管锁。回调内 Close 在触及 native 所有权之前被拒绝。
- 测试修订：初次扩大回归误以为 Dispose 后 SafeHandle.IsClosed 必立即为 true；实际持有 DangerousAddRef 时并非如此。改查公开接口拒绝新调用及最后一个 lease 释放后 native 句柄失效。RuntimeOwner 的独立 disposed 状态防止依赖这个不成立的假设。该失败不是原生销毁算法的 RED。
- 最终 `make -j1 csharp-test doc-check structure-check DOTNET=/data/tmp/rnet-dotnet/dotnet` 通过（`/tmp/netlib-checked-close-final.log`）：旧 Smoke、六组 checked Close、Quickstart 边界和三传输 CLI、脚本文档门禁、严格 Rust 文档/doctest、结构检查均通过；C# 零警告/零错误。
- 独立临时目录只放 RNet.dll/XML、当前 debug librnet.so 和 Smoke/Quickstart 源码，使用 RNetDll 引用构建，无 SDK 项目源码；完整 Smoke 连续十次通过，独立 Quickstart 的默认/单槽/拒绝/非法参数 CLI 全部通过（`/tmp/netlib-checked-close-consumer.log`）。这不是完整 release 打包资格。
- 对抗性复核未发现本批仍需修改的高置信运行问题；所有 native teardown 均在 gate 外，错误路径不关闭 SafeHandle/释放 logger root，成功后 SetHandleAsInvalid 阻止再次销毁。锁与计数只覆盖请求进入/退出，不跨原生调用等待。
- 边界：没有注入真实 OOM、线程池拒绝或永久阻塞 logger；最后一个 lease 恰在回调线程释放的全部故障矩阵未执行。未重跑 Rust workspace/Clippy、其他语言动态集成、ASan/Miri/Loom、完整 release 包、远端 CI 或目标环境长稳。构建串行并用守护脚本保持至少 15 GiB 可用内存，未提交/推送/发布。
