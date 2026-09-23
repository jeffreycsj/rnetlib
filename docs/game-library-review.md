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

具体接入见 [游戏指南](game-networking.md) 与 [C# 指南](csharp.md)。验证命令：`make check`、`go test -a -p 1 -count=1 -race ./go/rnet`、`make package`，目标长稳最后单独执行。外部 native archive 不纳入 Go build cache 依赖，native 更新后的验收必须强制重建，而不只清测试结果缓存。

## 本轮验证结果

- `cargo fmt --check`、结构门禁、`cargo clippy --workspace --all-targets -- -D warnings` 通过。
- `cargo test --workspace -- --test-threads=1` 全部通过，包含新销毁/日志回归及既有恢复、心跳、队列、弱网控制重传测试。
- 四个 C++11 最终链接示例、Go 全套与 race、两个 Loom 发送准入模型、release ABI 导出比对通过。
- C# 三传输收发、动态加密/rekey、LatestOnly、空包与 metadata、恢复、版本协商、真实质量/时钟采样、公钥 pin 拒绝、GC、日志异常/查询重入、非法配置和并发 Dispose 通过。发布包中的 `RNet.dll` 与 `librnet.so` 也完成同一测试；managed 构建零 warning。
- CI 已加入 C#、Go race、ABI 和 SDK 打包验收，但这里报告的是本机执行结果，**未声称远端 CI 已运行成功**。本轮没有重新执行历史 ASan/Miri campaign，也没有运行目标环境长稳。

## 后续增量：建立后的 TCP 关闭诊断

- 统一 TCP 已建立会话不再只保留错误码：read/EOF、framing/decode、受保护处理、write、安全控制写入和切换超时保留阶段及本地原因；既有 status 和游戏事件 ABI 不变。
- 关闭清理拆入 `session_close.rs`，诊断最多 1024 字节且不会切断 UTF-8 字符，截断后释放多余分配容量。并发失败与重复关闭只能发布一次事件、计数一次。
- 对抗性复查发现诊断数据会挤占事件字节预算。现对 `SessionClosed` / `JoinFailed` 优先丢可选详情，保留状态/句柄；不剥离认证语义数据，不为了详情额外淘汰业务事件。条目数量本身耗尽时仍按原有策略拒绝并计数，不能保证任意满队列下无损。
- 新增真实 TCP 代理用例覆盖远端 EOF、非法长度前缀和屏蔽 rekey ACK 后的超时；同时检查凭据/payload 不进入日志。队列测试覆盖有余量、字节满、认证数据不可剥离，关闭单元测试覆盖 UTF-8 和并发赢家。
- 覆盖边界：这不是全部平台的 OS 故障注入；主动关闭等只有 reason code 的情形没有伪造补全。后续 UDP/KCP 补齐见下节，目标长稳仍留到最后。
- 增量验证：最终全库串行测试与严格 Clippy 通过；C++11/C# 回归通过；Go 普通/race 均使用 `-a -p 1 -count=1` 实际重建重跑通过；新增三个纯事件队列测试通过固定 nightly 下的 Miri。这不是重新执行全部历史 fuzz/Miri campaign。

## 后续增量：UDP/KCP 关闭诊断

- 自适应 UDP/KCP 的空闲、安全切换、握手/业务认证超时、控制重试耗尽和编码/发送失败保留本地阶段及原因；状态码、wire 和公开 ABI 不变，复用关闭赢家、UTF-8 长度与事件预算保护。
- 真实回环代理覆盖两种传输的空闲与安全切换黑洞。坏包测试确认协议错误计数增长，但已建立会话仍可传递业务、不上抛违规事件、不逐包灌日志；不把未经认证的接收错误升级成断线。
- 延迟发送测试使用真实 Noise 状态和 UDP socket：编码超限、IPv4 socket 向 IPv6 发送失败均保留详情并清理队列；KCP 预算满则保留原消息和会话，不发布关闭事件。
- 对抗性检查范围是本轮关闭诊断及其测试，不代表全库安全证明。仍未模拟所有平台的内核错误；KCP 内部异步 flush/retry 的可恢复发送失败仍依赖重传及超时，不能声称每次 OS 失败都有单独日志。日志详情可因字节预算被舍弃，稳定错误码仍是业务判断依据。

### 本次对抗性复查

| 严重度 | 位置 | 发现与处理 |
| --- | --- | --- |
| 中（测试盲区） | `game_datagram_diagnostics.rs` | 最初双向黑洞不能区分提案丢失和回包丢失。已改为正常转发服务端提案、只丢客户端回包，并断言确有回包被屏蔽。 |
| 低（测试盲区） | `game_datagram_diagnostics.rs` | 仅检查业务仍收到，不能证明非法包触发过解析。已增加协议错误计数增长断言，并在 logger 退出后检查全部日志数量，避免异步回调尚未完成造成假通过。 |

复核错误分支保持原关闭条件与状态码，未把 `WouldBlock` 变成断线，未为每个非法数据报分配诊断。新增发送失败测试检查会话移除、清理通知、队列归零和单次关闭计数；公共诊断队列的字节预算与认证数据保护继续由全库回归覆盖。生产判断仍需要独立安全评估和最终目标环境验收。

本次最终验证：格式、结构门禁、严格 Clippy、全工作区串行测试、四个 C++11 示例、Go 普通/race 强制重建、C# 三传输与异常/生命周期回归、release ABI 导出校验全部通过。新增 6 个测试函数分别覆盖两种数据报传输及延迟发送的异常分支。本次没有重新打包 `dist/`，也没有执行新的 ASan/Miri campaign 或目标环境长稳。

## 后续增量：KCP 未认证预检的发送错误隔离

`adaptive_kcp_preflight.rs` 的服务端 cookie challenge 发送错误原先向上传播，导致共享数据报循环直接退出。现仅丢弃该次预检，计入已有 `Unauthenticated` 准入拒绝计数，不分配会话/KCP 引擎、不逐包产生事件或日志。客户端发送失败仍返回具体 `IoError`，没有全局吞掉错误；wire、ABI 与已有认证过程不变。

新增回归通过 IPv4 socket 向 IPv6 地址发送，制造真实 OS 错误：先确认旧实现失败，再验证服务端隔离、客户端失败保留、拒绝计数和未分配状态。这是处理器边界的故障注入，不声称已经在所有平台复现远端攻击。

对抗性复查未发现本次改动引入的新缺陷；仍需单独处理 `adaptive_datagram.rs` 中 `recv_from` 自身失败后直接退出、未统一报告端点故障和回收句柄的路径。本次预检修复不能代替那条退出路径的修复与回归，长稳仍留到功能收口之后。

本次验证：新增回归完成 RED/GREEN，13 项自适应安全测试、全工作区串行测试（含 C ABI）、格式/结构检查与严格 Clippy 全部通过。未重跑 C++/Go/C# 独立 SDK 示例、重新打包或执行长稳。
