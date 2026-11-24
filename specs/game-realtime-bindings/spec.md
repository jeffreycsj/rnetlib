# 实时快照高级接口跨语言一致性

用户已同意补齐跨语言接口；本轮将现有 Rust 行为透传到 C/C++11/Go/C#，不改 wire、ABI 结构或队列算法。

## 行为契约
- 保留无 tick 的 send_latest；新增 send_latest_with_tick，tick=0 和 UINT32_MAX 都是有效值；同键替换同时替换 payload 和 tick。
- C 新函数接收 uint32_t tick（显式存在）；省略 tick 使用现有 send_latest，不增加配置结构或 presence 参数。
- flush_realtime(capacity) 最多尝试 capacity 个待发快照，返回被传输层接受的数量，不代表送达；0 不发包，仍检查 runtime 句柄。
- C 返回状态码，必填 size_t* out_forwarded 在失败时清零；空输出指针返回 InvalidArgument。Go 返回 (int,error)，拒绝负 capacity；C++ 使用 size_t；C# 使用 nuint。
- 独立 flush 只推进实时快照，不取业务事件、不推进心跳/时钟/协商，不替代持续正容量 poll。可能等待共享 poll 锁，背压时保留快照。
- 复用现有借用 payload、lease、错误和停止语义；未就绪/失效 session 拒绝发送，已停止 runtime 的空 flush 与 Rust 一致返回 0。

## 实现与验收
- [x] 基线：现有 FFI 游戏、跨语言用例。
- [x] RED：ABI 缺失符号、tick 替换/零值/最大值、独立 flush/零容量/错误输出。
- [x] C ABI 与自动生成头文件、导出白名单。
- [x] C++11、Go、C# 门面及真实收发回归。
- [x] 文档使用示例、边界说明、接口审查与对抗性审查。
- [x] 相关 Rust/C/C++/Go/C#、文档与 ABI 检查。

发布审批归档、runner 配置、干净提交完整资格和最终目标环境长稳为后续阶段，不在本轮冒充已完成。

## 验证记录（2026-11-24）

基于 `d58dbe8` 的本地未提交修改树；这是本轮功能回归，不是干净发布提交资格。

- 基线：FFI 游戏 9 项，以及 C++11、Go、C# 现有接入测试通过。
- RED：新 FFI 回归仅因缺少 `rnet_game_send_latest_with_tick` 和 `rnet_game_flush_realtime` 导出失败。
- GREEN：FFI 游戏 10 项通过；包含空输出、错误/销毁句柄、零容量、超大容量空队列、停止后 flush、正容量批量上限、空快照和 tick 省略/0/UINT32_MAX。
- `cargo test --workspace`：汇总 413 项通过，0 ignored，未发现资源跳过记录；其中短 soak smoke 不是长稳压测。
- `cargo clippy --workspace --all-targets -- -D warnings`、格式和结构检查通过。
- `make doc-check`：头文件一致性、75 项脚本回归、原生注释、Rust 文档与 doctest 通过。
- `make c-test cpp-test go-test csharp-test DOTNET=/data/tmp/rnet-dotnet/dotnet` 通过。C 有实际编译/链接和状态输出检查；C++11 有真实 TCP 快照收发；Go/C# 的新快照行为覆盖 TCP/UDP/KCP。
- 保留各语言无 tick 回归后，重新运行 C++11/Go/C# 全部对应 Make 目标，通过。
- `make abi-check`：release 库实际导出、生成头文件与导出清单一致。
- `go test -a -p 1 -count=1 -race -run 'TestGameFacadeJoinsAndExchangesOpaquePayload|TestGameRealtimeRejectsInvalidCapacityAndClosedRuntime' ./go/rnet` 在最终用例上通过。
- 构建和测试串行，由保护进程监测至少 15 GiB MemAvailable；未改变系统工具链或依赖锁文件。

本地日志保存在 `/tmp/netlib-bindings-*.log`，是临时开发记录，不是长期发布归档。没有生成新发布包、提交、推送、部署或运行目标环境长稳；未重跑 ASan/Miri/Loom 专项。

## API 与对抗性复查

审查范围为本次接口、绑定、生成头、测试及文档。按本地 SDK 的命名、所有权、错误及兼容契约审查，不引入 REST 资源或 Protobuf 字段。

| 复查项 | 结论 |
| --- | --- |
| 新用例替换了旧的无 tick 收发覆盖 | 已保留两条路径，验证带 tick 的待发快照被无 tick 快照替换后不残留标记；Go/C# 另验证最大值。 |
| 独立 flush 是否实际发送、是否遵守批量上限 | FFI 在调用 poll 前查询队列；两个 key、capacity=1 留下一条，零容量不消耗；三语言门面验证显式 flush 后接收。 |
| 句柄、指针、整数和生命周期 | C 输出先置零，拒绝空输出；复用 game lease，Go 保持 native 线程及 payload 存活，C# 复用 SafeHandle lease；count 不超过输入 capacity。 |
| 可用性和性能 | 薄包装复用 Rust 算法，不增队列或 wire 字段；文档明确共享 poll 锁等待、运输层接受不等于送达、持续正容量 poll。 |
| 部署一致性 | 布局 ABI 未变但新增符号；SDK 与 native 必须一起重建发布，不能用布局版本相等推断旧库具备新函数。 |

没有发现本轮尚未解决的高置信实现缺陷。平台验证仍仅 Linux x86_64/.NET 8；Go race 不能替代 Rust 并发内存模型验证。本轮未新增并发 flush/poll/stop 的专用长期压力测试，也不构成独立密码学审计。
