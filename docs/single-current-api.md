# 单一现行 SDK / wire 迁移

本轮是破坏性迁移，**不支持旧 SDK**。业务协议的版本范围协商保留；它不等于维护多套网络库 ABI 或 wire。整体迁移尚未完成，请勿据此发布生产包。

## 已落地的接口

- 一套完整 `rnet_config_t`、`rnet_config_init`、`rnet_runtime_create(config, optional_client_security, out)`。
- 一套完整 `rnet_game_config_t`、`rnet_game_config_init`、`rnet_game_runtime_create(config, optional_client_security, out)`。实时/优先级队列预算及可选 `logger` 均在配置中。
- 一套结构化 `rnet_logger_t`、完整 `rnet_metrics_t`、含 P99.9 的 `rnet_latency_metric_t`。延迟查询最后一个参数为 `drain_window`：0 查询累计，1 提取并轮转窗口；容量查询不消费窗口。
- `rnet_server_listen` / `rnet_client_connect` 的配置以 transport 决定 TCP/UDP/KCP。
- `rnet_poll_events` 和 `rnet_game_poll_events` 均返回状态码，通过 `out_count` 返回数量；零事件不再掩盖无效句柄或输出错误。
- 单一游戏事件布局直接提供 `correlation_id`，没有嵌套旧事件前缀。C ABI 底层事件也只保留 payload 和 correlation 元数据，不公开业务类型或流编号；Rust 原始事件字段尚待 wire 阶段清理。
- 底层发送为 `rnet_session_send(runtime, session, payload)`；带 correlation 使用 `rnet_session_send_ex(..., options)`。C++11 / Go 同步移除发送消息类型参数及 Legacy 发送方法。
- Rust 底层同样只使用 `send(session, payload)` / `send_with_options(session, payload, options)`；删除 typed send、`send_legacy`、`send_payload` / `send_payload_with_options` 别名。传输类型在建连时固定，业务类型放入 payload，成功仅表示本地队列接受，不保证对端收到。
- Rust `RuntimeConfig::default()` 与 `production()` 使用同一安全策略：不允许旧无认证端点、不允许本地服务器选择业务明文，自动轮换密钥开启。删除 `SecurityPolicy::compatibility()` 与 `GameRuntimeConfig::compatibility()`；业务明文通过明确的服务器策略开启，客户端仍自动跟随已认证服务器。
- 旧 `secure_tcp` / `secure_datagram` / `secure_kcp` 执行模块及 Rust `secure_*` 构造器已删除；C `rnet_listener_open` / `rnet_client_join` 和配置类型、C++ `open_secure_*` / `join_secure*`、Go `OpenSecure*` / `JoinSecure*` 与 cgo 适配同步删除，不保留转发别名。共享 Noise、cookie、KCP 及现行服务器动态切换路径保留。

- 旧无认证 TCP/UDP 执行模块、Rust 公开 `EndpointConfig` / `open_endpoint`、C `rnet_endpoint_open` 和配置/模式类型、C++/Go 包装及 `allow_legacy_unauthenticated_endpoints` 开关均已删除。内部角色不再可空；业务明文仍需服务器明确允许，且始终经过握手和授权。

## 接入要求

当前 `RNET_ABI_VERSION` 判别值为 3，只有一套实现，值 1 和 2 均不被接受。删除 legacy 配置字段后，即使平台填充导致尺寸不变，也不能与旧 SDK 混用。C 程序必须先检查 `rnet_abi_version() == RNET_ABI_VERSION`，再调用任何初始化/输出函数；C++11、Go、C# 的创建/密钥入口自动检查。ABI 判别不能替代调用方的指针有效性义务。

配置的 `struct_size` 必须与当前头文件精确一致。FFI 先读取公共头，再读取通过校验的完整布局；短旧配置不会被提前当成大结构体复制。空 logger 表示禁用日志；非空 logger 必须有有效回调。logger 结构体只在创建时借用，回调及 user_data 必须有效且线程安全，直到 destroy 返回。不要在日志回调里 stop/destroy。

头文件、native 库、C++/Go/C# SDK 必须一起重新编译部署。不要复用旧发布目录里的二进制；本阶段不生成最终发布包。示例见 [getting-started.md](getting-started.md)、[game-networking.md](game-networking.md) 和 [C# 指南](csharp.md)。

## 尚未完成

1. 合并游戏 exact/range 配置及 wire 状态机：统一走业务版本范围协商，精确业务版本表示为 min=max。当前仍有 wire-v3/v4 双路径，不能称为“wire 已统一”。
2. 最终 SDK/动态库/发布包全套验收、全范围对抗性复查。长稳压测按约定最后执行；跨实例恢复不在范围内。

## 阶段对抗性审查

| 严重度 | 位置 | 问题及处理 |
| --- | --- | --- |
| 高，已修复 | FFI 配置读取 | 原先完整解引用后才检查大小；现先读公共头并严格验证布局，以 Linux 保护页测试短配置边界。 |
| 中，已修复 | 延迟查询 | 合并时较新的传输查询会漏掉日志回调样本；累计与窗口各自记录，窗口轮转不清空累计，增加回归。 |
| 中，已修复 | C++ SDK | ABI 检查仅放在初始创建/密钥入口，不在 noexcept 移动构造中调用可能抛异常的检查。 |
| 未完成的需求 | 旧端点及游戏 wire | 旧端点已删除，仍有 exact/range wire 双路径；须统一游戏建连状态机，不能宣称全量去兼容已完成。Rust 旧发送入口已移除，但底层帧和 Rust 原始事件仍有历史字段，待 wire 收敛时一并清理。 |

测试盲区：保护页场景目前只在 Linux 执行；Windows/移动平台、ASan fuzz、独立安全审计和目标环境长稳尚未验收。整体评价：接口布局这一阶段已形成单一现行布局，但游戏 wire 清理与最终发布资格未完成。历史 ABI/多布局兼容承诺不再适用于本分支。

以下验证记录按阶段保留历史事实；当前剩余项以上方“尚未完成”和最新阶段记录为准。

## 本阶段验证记录

以下在 Linux x86_64 串行执行，构建/测试进程由 15 GiB 可用内存保护脚本监控：

- `cargo test --offline --workspace -- --test-threads=1`：通过，包括新增 ABI 拒绝/保护页测试、游戏恢复/弱网回归及原有 TCP 端点生命周期修复。
- `cargo clippy --offline --workspace --all-targets -- -D warnings`、格式、结构检查及 `git diff --check`：通过。
- `make -j1 cpp-test go-test csharp-test abi-check`：通过；C++11 严格警告、Go native 强制重建、C# 三传输联调、release 头文件/真实动态库导出一致。
- `go test -a -race -p 1 -count=1 ./go/rnet`：通过。
- `cargo check --offline --locked --manifest-path fuzz/Cargo.toml --bin ffi_config`：通过；只验证 fuzz 入口可构建，未运行 ASan fuzz。

没有生成本轮最终发布包，没有执行目标环境长稳。本记录不是生产认证或第三方安全审计证明。

## 后续阶段：Rust 发送入口收敛

旧调用形式的 5 个编译拒绝测试先在旧实现上失败，再在清理后通过；既有 TCP/UDP/KCP 收发、握手前禁止发送、超限、背压、安全切换测试迁移到现行发送接口，不通过删除失败路径覆盖实现迁移。

对抗性复查发现原有 correlation 断言主要集中在旧端点路径，因此补充现行加密 TCP/UDP/KCP 的空消息、含零字节/非 UTF-8 消息、`u64::MAX` correlation、默认 correlation=0，以及关闭会话/停止 runtime 后的错误回归。本次不改变排队、控制加密和 wire 编码语义。

本阶段重新通过 `cargo test --offline --workspace -- --test-threads=1`、严格 Clippy、格式/结构检查及 C++11 / Go（强制重建 native 链接）/ C# 联调。测试仍在 Linux 串行、15 GiB 可用内存保护下执行；没有执行目标环境长稳，也未重新运行 ASan fuzz 或独立安全审计。

## 后续阶段：安全默认值与客户端明文处理

本阶段先收敛安全默认值和兼容预设，**尚未删除旧端点导出、显式 legacy 开关或执行模块**。尚待迁移的旧端点测试在测试内显式授权，不再依赖宽松默认配置；现行业务明文切换测试只开启业务明文权限，不开启旧无认证端点。

默认策略的 3 个拒绝/资源回收回归及两个兼容构造器的编译拒绝测试完成 RED → GREEN；另外验证 TCP/UDP/KCP 默认服务器拒绝降级后连接仍可收发，以及独立默认客户端跟随服务器动态切换和 rekey。

### 对抗性审查发现与修复

| 严重度 | 位置 | 问题与处理 |
| --- | --- | --- |
| 高，已修复 | 游戏 poll 的错误分类 | 接收错误曾同时依赖本地 `allow_plaintext_business_data`，导致默认客户端虽已跟随服务器使用业务明文，仍会把未校验的非法 envelope 上抛为违规并关闭。现在按事件的实际完整性与业务/控制类型分类，删除 runtime 中多余的策略副本；不使用可能已切换的当前安全模式判断排队中的旧消息。 |

新增三个真实 TCP/UDP/KCP v4 回归先复现默认客户端违规断线，再验证非法明文仅计数丢弃、后续合法消息仍可收取、切回加密后相同非法内容仍关闭会话。认证控制错误仍沿用关闭路径。这里只消除游戏 envelope 层的错误分类问题，不承诺明文业务的机密性、完整性或整体抗断线能力；外层 framing 和安全记录错误仍可能终止连接。

最终验证：本轮 8 个新增行为测试与 2 个编译拒绝测试通过；串行 workspace 全量测试、严格 Clippy、格式/结构检查、C++11 四个示例、Go 强制重建 native 链接及 C# 三传输联调通过。构建测试继续由 15 GiB 可用内存保护器监控。未重打最终发布包，未执行新的 Go race、ASan/Miri/Loom 专项或目标环境长稳；旧端点、wire 和历史帧字段清理仍需后续完成。

## 审查修正：明文分类与迁移前测试（2026-10-14）

上一阶段的错误分类范围过宽：`poll` 把未校验业务事件的所有转换错误都当作明文坏包，连合法早期消息耗尽本地缓冲也被静默吞掉。本阶段将处理拆到独立的 `message_runtime` 模块：仅 envelope 解码、帧形态及传输序号形态错误可按未校验明文计数丢弃；合法消息随后才进入协商缓冲。

预算拒绝沿用既有 `session_admission_rejected` / `runtime_admission_rejected` 独立计数。明文和加密均上抛 `ProtocolViolation`、通过配置 logger 保留具体预算原因，并以 `ProtocolError` 关闭会话；不新增 ABI，也不把资源不足说成对端恶意的证据。

- TDD：合法明文耗尽预算的回归先在旧实现上因缺少违规事件失败，再修复；消息数、字节数、单会话限额和空消息占槽均有分类测试。
- 真实 TCP/UDP/KCP、两种业务加密模式验证：预就绪预算耗尽可见、路由句柄失效、关闭释放已占预算、不污染明文坏包计数。
- 三传输跨切换回归：客户端暂停游戏 poll，等待旧明文实际接收，再等待服务器拿到客户端切换 ACK；随后 poll 仍丢弃旧坏包并交付旧合法消息。切换后新收到的认证坏包仍严格关闭。
- `secure_loopback` 的六项测试迁到现行 `listen` / `connect`：保留授权前禁止发送、错误公钥拒绝、cookie 先于 Noise、超大 join 拒绝、三传输消息与相关延迟覆盖；仅替换旧路径专属的“不支持控制通道”断言为现行受保护控制交付检查。
- 部署与资格文档明确单 runtime / 原监听器、一次性票据、持续 poll 的硬限制；删除已失效的 v3 metrics / v2 logger 接入名。打包清单包含 README 链接到的迁移说明，但本阶段不生成发布包。

### 对抗性复查

| 发现 | 处理 |
| --- | --- |
| 只按错误码分类不能区分畸形输入和本地预算失败 | 按处理边界分类；预算失败不进入坏包软丢弃分支。 |
| 纯状态测试不足以证明会话真正关闭并释放资源 | 增加真实连接上的六种 transport/security 组合回归。 |
| 仅在双方已轮询到 SecurityChanged 后发包，覆盖不到排队旧消息 | 暂停客户端游戏 poll 跨越实际加密 barrier，再验证旧事件完整性。 |

当前仍未解决的发布阻塞：旧端点、显式 legacy 开关与旧 secure 模块删除；exact/range 配置和 wire 状态机统一。六项测试迁移不等于已删除旧执行路径。最终必须在完成迁移后的干净提交上重跑发布门禁；历史 ASan/Miri/Go race 与短回环证据不外推，跨实例恢复不实现，目标环境长稳最后执行。

本阶段验证（基于 `30d5304` 后的本地修改树，不是干净发布提交）：串行 `cargo test --offline --workspace -- --test-threads=1` 通过，游戏库单元测试 91 项；严格 workspace/all-targets Clippy、格式、结构、shell 语法和 diff 检查通过。Loom 首次离线运行缺依赖，按锁文件补齐后 `cargo test --locked -p rnet-game --features loom admission::tests -- --test-threads=1` 的两个并发模型通过，未改动锁文件。所有构建/测试由至少保留 15 GiB 可用内存的保护器监控。没有重新执行 C++/Go/C# 独立联调、Go race、ASan/Miri、独立审计或目标环境长稳，也没有重打最终发布包；workspace 内的数秒 soak smoke 仅算回归，不算长稳资格。

## 后续阶段：删除旧 secure 执行路径（2026-10-14）

删除三套旧 secure 模块、运行时分派、安全角色变体及 transport-specific secure 构造器。C 的旧 listener/join 导出和配置类型、C++11/Go 包装及 cgo 适配一并删除，不保留兼容别名；源文件可从 Git 历史恢复。现行 `listen` / `connect` 的 Noise、cookie、KCP、动态加密切换及 rekey 共用实现未删除。

TDD 顺序：先运行原有 C ABI 契约测试；新增旧接口消失门禁并确认它在旧 C 头文件上失败；将 C 的 TCP/UDP/KCP 认证建连测试迁到现行入口并运行通过；最后删除旧模块与 SDK 入口。此前已迁移的六项 Rust secure 回归继续验证现行路径，包括授权前禁止发送、错误公钥拒绝、cookie 先于 Noise 分配和超大 join 拒绝。

### 对抗性复查

| 发现 | 处理 |
| --- | --- |
| 原结构检查要求旧 secure 文件存在，与删除目标冲突 | 改为检查现行 adaptive 模块；新增旧模块、构造器及各语言旧 API 不存在的门禁。 |
| 只检查头文件无法证明二进制不再导出旧 API | 重新构建 release 动态库，实际 `nm` 导出与头文件、预期导出清单逐项核对。 |
| 安全文档仍指向旧版本化入口并描述固定 KCP conv | 更新为现行入口及 cookie 预检中的随机 conv；明确仍是一 peer 一活动会话，未承诺多路复用。 |

本阶段验证基于 `1dd489c` 加本地修改，不是干净发布提交：工作区串行测试、严格 workspace/all-targets Clippy、格式/结构及 diff 检查通过；`make -j1 cpp-test go-test csharp-test abi-check` 通过，包括 C++11 四个示例、Go 强制重建 native 链接、C# 三传输联调和实际 release 导出检查。构建测试持续受 15 GiB 可用内存保护。

未重新执行 Go race、ASan fuzz、Miri、Loom、独立审计或目标环境长稳，未生成最终发布包。仍须删除无认证 TCP/UDP 入口及 legacy 开关，再统一 exact/range 游戏 wire；上述阶段结果不代表这些剩余项已完成，也不将历史安全专项结果外推到当前代码。长稳仍在功能迁移与发布门禁完成后执行。

## 后续阶段：删除无认证端点（2026-10-14）

旧 `tcp.rs` / `udp.rs` 执行模块、无认证构造器、公开 `open_endpoint`、C `rnet_endpoint_open` / endpoint 配置与模式类型、C++/Go 包装及全部 SDK 的 legacy 开关已删除。内部 `EndpointConfig` / `EndpointSecurity` 不再公开，角色不可空；仅供单元测试构造路由的辅助方法受 `cfg(test)` 限制，不进入 release 库。源文件可从 Git 历史恢复，不提供旧 SDK 转发层。

唯一 ABI 判别值更新为 3，旧值 1/2 均拒绝。删除字段后，64 位结构体填充可能让总尺寸保持不变，因此不能只检查 `struct_size`。C# 同步删除字段，所有显式 ABI 输入改用集中常量；其他 SDK 使用头文件常量。头文件和 native/SDK 必须一起重建。

### 测试迁移与对抗性复查

| 检查点 | 处理与证据 |
| --- | --- |
| 删除旧路径会失去预算、关闭及事件顺序保护 | 先运行原 20 项 loopback 与 5 项安全默认值基线；迁移后 17 项 loopback 覆盖现行三传输收发、相关 ID、IPv6、超限、预算、准入拒绝、端口冲突、握手 EOF、监听器可继续使用和生命周期事件不被业务包抢占。 |
| 旧 UDP 测试假定业务首包自动建会话 | 改为验证关闭后旧业务包不能复活路由，显式重新握手授权后使用新句柄收发。原 TCP/UDP 正常收发合并为三传输测试；两个只针对已删除入口的拒绝测试由源码/符号消失门禁替代，不保留旧语义。 |
| C ABI 夹具依赖无认证连接 | 夹具改用固定测试密钥、现行 listen/connect 和授权；20 项契约测试继续覆盖日志、延迟、关闭、token 一次释放、错误和并发销毁。原旧 KCP 入口拒绝项由旧 API 消失门禁替代。 |
| ABI 字段删除可能被平台填充掩盖 | 旧 ABI 2 拒绝测试先失败再通过；保护页短配置测试继续通过。C++/Go/C# 与实际 native 联调验证现行契约。 |
| 内部类型私有化后易误删凭证脱敏覆盖 | 将 endpoint 调试输出测试移到传输库内部，同时验证 client ticket 与 server 私钥不进入 Debug 输出。 |
| 业务明文可能被误解为免认证 | 新测试验证开启业务明文仍拒绝无握手 TCP 记录；C++ 明文示例也经过握手授权。现有三传输服务器动态切换/rekey 回归继续通过。 |

旧 API/开关消失测试和 ABI 2 拒绝测试均完成 RED → GREEN。SDK 检查覆盖头文件、预期导出、cgo、C++/Go 包装与 C# 配置；release 实际导出另通过 `nm` 核对，不只依靠源码字符串判断。

本阶段在 `1dd489c` 加当前本地修改树上串行验证通过：`cargo test --offline --workspace -- --test-threads=1`、严格 workspace/all-targets Clippy、格式/结构及 diff 检查、`make -j1 cpp-test go-test csharp-test abi-check`。C++11 四个示例、Go 强制 native 重建、C# 三传输认证/收发/动态切换/恢复/日志/错误路径和 release 导出一致性均通过；所有构建/测试持续保留至少 15 GiB 可用内存。

整体评价：旧端点 API 与执行模块清理已完成，游戏双 wire 尚未统一，仍不是可认证的最终生产发布。未重跑 Go race、ASan fuzz、Miri、Loom 或独立审计，未生成最终发布包；短 soak smoke 只算功能回归。后续先统一 exact/range 配置、协商/恢复状态机与历史帧字段，再在干净发布提交上重跑资格验证，最后执行目标环境长稳。
