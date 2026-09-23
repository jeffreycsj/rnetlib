# 七项要求审查与交付边界

本轮以现有规格、各层实现、跨语言 ABI 和实际测试为证据，检查传输、安全状态机、游戏接口、内存/队列边界、回调生命周期及可观测性。它不是逐行安全证明，也不替代第三方密码学审计或目标环境验收。

## 要求与证据

| 要求 | 当前实现 | 主要证据 |
| --- | --- | --- |
| TCP / UDP / KCP | 建立端点时选类型，send 无 transport/msg_type/stream_id | `rnet-game/tests/game_runtime.rs`、`rnet-transport/tests` |
| 客户端与服务端同库 | 同一 GameRuntime 监听、连接、认证、收发、踢人、关闭；配置有界预算、握手/连接上限、超时等 | `runtime.rs`、`range_api.rs`、`game_quickstart.rs` |
| Rust / C / C++ / Go / C# | 统一 C 游戏 ABI，C++11/Go 和新增 .NET 8 门面；C# 首先验收 Linux x64 | `game_ffi.rs`、C++ game examples、Go game tests、`csharp/RNet.Smoke` |
| 加密与服务端中途切换 | Noise 握手与服务器公钥 pin；认证控制驱动切换/rekey，客户端自动跟随 | `security_switch`、game runtime/FFI/SDK 三传输测试 |
| 自有日志组件 | 可选 bounded callback；不记录业务包和凭据；隔离回调 panic/exception | `diagnostics.rs`、`rnet-observe`、各语言 callback 测试 |
| 错误、P95/P99、可追溯 | 结构化句柄/原因/correlation、Prometheus、固定内存直方图；新增默认 30 秒游戏层累计摘要 | `diagnostics_summary.rs`、`game_diagnostics.rs`、`game_log_interval.rs` |
| 生产游戏库 | 具备有界资源、背压、安全默认、协商、心跳、恢复、实时快照、队列公平调度和 CI 基础 | **生产候选，不是已完成目标环境验收的生产定版** |

## 本轮确认并修复的问题

| 严重度 | 位置 | 缺陷与修复 |
| --- | --- | --- |
| 高 | `rnet-ffi/src/game_registry.rs` | 持全局注册表锁销毁 runtime，logger 回调查询指标时可与 join 死锁。先移除句柄并释放锁再 drop；新增可控回调阻塞的 RED/GREEN 测试。传输 destroy 同步采用该释放顺序。 |
| 高 | `go/rnet/game.go` | Close 持 Go wrapper 锁调用 native destroy，正在查询指标的 logger 无法退出。先标记关闭，再无锁销毁；失败恢复句柄，覆盖并发回调回归。 |
| 中 | `go/rnet/native.go` 及调用点 | native 错误文本为 OS-thread-local；Go 两次 cgo 间换线程会读错原因。入口固定 OS 线程直到复制错误文本；并发不同失败原因测试。 |
| 中 | `rnet-game/src/poll_runtime.rs`、transport `state.rs` | typed 事件转换和失败路径丢弃具体原因。保留本地握手/端点诊断、协议校验和排队发送失败原因，禁止把凭据或业务 payload 当错误详情。 |
| 功能缺口 | 游戏日志、`csharp/` | 游戏层缺周期分位数日志；C# 无门面。已补跨语言间隔配置、分模块 C# SDK、资源管理和三传输测试。 |

对抗性复查特别检查了 C# native token 的 finally 释放、delegate/GCHandle 存活、SafeHandle 与并发 Dispose、回调查询重入、异常隔离、错误 pin 和参数零值。跨语言 tests 调用真实 native 库和 socket，不以 mock 代替协议。

## 仍需明确的限制与发布门禁

1. **不宣称性能已经达标。** 功能收口后，再按 `game-production-qualification.md` 执行目标硬件、并发数、包长、频率、丢包/乱序/延迟矩阵与 24–72 小时长稳，保存 CPU/RSS/队列/丢包/P95/P99/P99.9 原始结果。没有目标 SLO 与实测结果就没有容量保证。
2. **独立安全评估未完成。** 有单元/集成、短 ASan fuzz、Miri/Loom 门禁不等于已完成长期 fuzz 或 `snow`/密码学第三方审计。
3. **平台边界。** 当前 SDK 交付以 Linux x86_64 为验证基线；C# Windows/Unity/IL2CPP/AOT 与其他平台需各自 ABI、库加载、线程及生命周期验证，不应直接列作生产支持。
4. **明文是明确降级。** 允许明文时业务数据没有完整性和机密性；控制仍认证，坏明文 envelope 静默计数，但不能阻止对合法明文业务的篡改。公网默认保持加密。
5. **日志不是无限审计仓库。** 回调必须迅速返回；满队列允许丢弃并计数。摘要是累计直方图，不是滑动窗口，不记录每个包；TCP/KCP 本地 pickup 不是送达。已建立连接的部分关闭事件只有稳定 reason code，不能从它恢复不存在的 OS 细节。
6. **恢复和撤回边界不扩大。** 按用户要求不做跨实例恢复；已进 socket/KCP 缓存的数据无法撤回。玩家身份/权限/重放策略仍属业务层，指标不自动按玩家高基数切分。

具体接入见 [游戏指南](game-networking.md) 与 [C# 指南](csharp.md)。验证命令：`make check`、`go test -race ./go/rnet`、`make package`，目标长稳最后单独执行。

## 本轮验证结果

- `cargo fmt --check`、结构门禁、`cargo clippy --workspace --all-targets -- -D warnings` 通过。
- `cargo test --workspace -- --test-threads=1` 全部通过，包含新销毁/日志回归及既有恢复、心跳、队列、弱网控制重传测试。
- 四个 C++11 最终链接示例、Go 全套与 race、两个 Loom 发送准入模型、release ABI 导出比对通过。
- C# 三传输收发、动态加密/rekey、LatestOnly、空包与 metadata、恢复、版本协商、真实质量/时钟采样、公钥 pin 拒绝、GC、日志异常/查询重入、非法配置和并发 Dispose 通过。发布包中的 `RNet.dll` 与 `librnet.so` 也完成同一测试；managed 构建零 warning。
- CI 已加入 C#、Go race、ABI 和 SDK 打包验收，但这里报告的是本机执行结果，**未声称远端 CI 已运行成功**。本轮没有重新执行历史 ASan/Miri campaign，也没有运行目标环境长稳。
