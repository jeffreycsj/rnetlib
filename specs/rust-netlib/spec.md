# Rust Netlib Phase 1 规格说明

## 需求概述

在 `/home/jeffsjchen/netlib` 新建一个 Rust workspace，实现设计文档中的 Phase 1：以 Tokio 为异步核心，对 TCP/UDP 提供统一生命周期、有界队列、帧协议、批量事件和可观测性，并通过稳定 C ABI 交付 Rust 静态/动态库、C 头文件、C++ RAII 包装和 Go cgo 包装。KCP 的上层适配边界在本阶段固定，但不在未完成依赖选型、许可审核、fuzz 和弱网测试前声称可生产使用。

## 行为规格

### 正常流程

- 当调用方使用合法配置创建 runtime 时，系统应启动一个可配置 worker 数的 Tokio 多线程 runtime，并返回带 generation 的 opaque handle。
- 当打开 TCP listener/client endpoint 时，系统应完成监听/连接，并通过批量 poll 返回建连、收包、可写和关闭事件；同一 session 的事件保持有序。
- 当打开 UDP endpoint 时，系统应按一数据报一帧收发，保留对端会话映射，且不伪装可靠或有序语义。
- 当收到 TCP 半包或粘包时，系统应按固定网络字节序帧头增量解析，并将完整 body 以受控 buffer token 交给调用方。
- 当调用 `rnet_send` 发送合法消息时，系统应构造统一帧，放入该 session 的单写者有界队列，不在 ABI 调用中无限阻塞。
- 当调用 `rnet_poll_events` 时，系统应在 capacity 和 timeout 范围内批量返回事件；调用方通过 `rnet_buffer_release` 归还数据。
- 当事件消费者落后时，系统应对 TCP 暂停读取，对 UDP 按配置丢弃，并记录队列满/丢弃指标。
- 当调用 `stop` 时，系统应从 RUNNING 进入 DRAINING，拒绝新连接和普通发送，在 deadline 内尽力排空 TCP，随后进入 STOPPED；重复 stop 幂等。
- 当配置了外部 logger 时，系统应通过专用有界队列/线程调用，默认不阻塞 I/O worker。

### 边界和异常

- 当 magic、version、flags、body length 或 UDP 数据报长度不合法时，系统应在分配大块内存前拒绝，上报稳定错误码/协议事件。
- 当写队列满时，TCP 应返回 `RNET_E_WOULD_BLOCK`；UDP 应按丢新/丢旧策略处理；容量为零、恰好满和恢复可写都必须可测。
- 当 handle 失效、generation 过期、runtime 已停止或参数指针/长度组合不合法时，系统应返回稳定错误码，不触发 UB。
- 当 Rust 代码 panic 时，所有 `extern "C"` 入口应捕获 panic 并返回 `RNET_E_INTERNAL_PANIC`，panic 不跨越 ABI。
- 当 buffer 重复释放、runtime 尚有借出 buffer 或从 logger 中重入 destroy 时，系统应拒绝操作并保持资源可恢复。
- 当外部 logger 过慢或队列满时，系统应丢弃/采样低级日志并保留计数，不拖住网络线程。
- 本阶段不承诺 TLS、加密 UDP/KCP、`io_uring`、`recvmmsg/sendmmsg`、Windows/macOS 发布产物或未指定环境下的固定 QPS。

## 技术设计

### 模块划分

- **rnet-core**：runtime 状态机、generation handle/slab、配置、事件、buffer token、错误码和背压协调。
- **rnet-protocol**：28 字节固定帧头、增量 parser/encoder、长度/版本/flag 校验；提供 Prost 接入点，但不强制解码业务 Protobuf。
- **rnet-transport**：统一内部 driver 事件，TCP listener/client 与单写者队列，UDP endpoint/session 映射，及可替换的 `KcpEngine` 接口。
- **rnet-observe**：有界异步日志 dispatch、丢弃计数、按 worker/transport/endpoint 聚合的 metrics snapshot。
- **rnet-ffi**：opaque handle 注册表、C POD 结构、入参校验、`catch_unwind`、内存所有权 API，产出 `staticlib`/`cdylib` 和版本化 `include/rnet.h`。
- **cpp / go**：C++ RAII 封装与 Go cgo 封装，各自包含最小 echo 示例和真实最终链接测试。

### 关键接口

- C ABI 保留文档中的 `rnet_runtime_create`、`rnet_endpoint_open`、`rnet_send`、`rnet_poll_events`、`rnet_buffer_release`、`rnet_endpoint_close`、`rnet_runtime_stop`、`rnet_runtime_destroy`、`rnet_abi_version`。
- 所有公开配置/POD 结构首部包含 `struct_size` 和 `abi_version`；字段只能尾部追加；枚举使用显式整数值。
- 帧头为 `magic:u32 | version:u16 | flags:u16 | msg_type:u32 | stream_id:u32 | body_len:u32 | request_id:u64`，网络字节序，默认 body 上限 1 MiB，UDP 默认数据报上限 1200 字节，均可在更小范围配置。
- 稳定错误集至少包含 `OK/INVALID_ARGUMENT/INVALID_HANDLE/INVALID_STATE/WOULD_BLOCK/TIMEOUT/NOT_SUPPORTED/IO_ERROR/PROTOCOL_ERROR/MESSAGE_TOO_LARGE/INTERNAL_PANIC`。
- 事件至少包含 endpoint opened/error、session opened/closed、message、writable 和 runtime stopped；异步失败通过事件返回。

### 纯度边界

- **纯逻辑**：帧编解码、配置校验、错误映射、generation handle 判定、队列满载策略、状态迁移。
- **副作用**：socket I/O、Tokio 任务/线程、计时器、FFI 指针读写、日志回调和最终链接。
- 将纯逻辑放在可直接单测的 crate，`unsafe` 主要收敛在 `rnet-ffi`，每处记录指针、别名和生命周期不变量。

## 任务拆解

### 准备工作

- [x] 在 `/home/jeffsjchen/netlib` 初始化 workspace，固定 Rust edition/MSRV 与依赖版本，提交 `Cargo.lock`。
- [x] 确认 C/C++、Go、`protoc` 工具链可用性，建立串行测试基线与内存门槛检查。

### 实现任务

- [x] 任务 1：建立 workspace/crate 骨架、统一错误码、配置与 generation handle，先写边界测试。
- [x] 任务 2：用 RED 测试覆盖半包、粘包、长度溢出、非法版本/flag，再实现帧 parser/encoder。
- [x] 任务 3：实现 runtime 状态机、有界事件队列、buffer token 所有权与 metrics snapshot，覆盖关闭/竞态。
- [x] 任务 4：实现 TCP listener/client、增量读取和单写者队列，覆盖 IPv4/IPv6、半关闭、端口占用、慢消费者与优雅停止。
- [x] 任务 5：实现 UDP endpoint 和对端 session 映射，覆盖数据报上限、丢包策略、IPv4/IPv6 与关闭。
- [x] 任务 6：实现有界异步 logger、采样/丢弃计数与阻塞 logger 故障测试。
- [x] 任务 7：实现 C ABI 与版本化 `rnet.h`，对空指针、错长度、过期 handle、重复释放、panic 边界和并发调用做测试。
- [x] 任务 8：实现 C++ RAII 和 Go cgo 包装/回显示例，在 Linux x86_64 执行真实静态库最终链接。
- [x] 任务 9：固化 `KcpEngine` 适配接口、能力位与 `NOT_SUPPORTED` 行为，形成 Phase 2 选型/安全/弱网验收清单。

### 验收

- [ ] `cargo fmt --check` 与 `cargo clippy --workspace --all-targets -- -D warnings` 通过（当前 TencentOS Rust 工具链未安装 rustfmt/clippy，且没有 rustup）。
- [x] `RUSTFLAGS='-D warnings' cargo test --workspace` 通过（34 项测试）。
- [x] TCP/UDP Rust 集成测试、C++ 最终链接/回显测试、Go cgo 最终链接/回显测试通过。
- [x] 有界内存/队列测试证明慢消费者不造成无限增长，并且 `WOULD_BLOCK`/丢弃计数符合规格。
- [x] 检查实际 diff、对公开 ABI 做人工审查，然后执行 `dserver-workflows:vsdd-adversarial-review`。
- [x] 输出已实现能力、未纳入的 Phase 2–4 项目、已验证平台和不可声称的性能结论。

## 待确认的范围决策

本次默认交付 Phase 1，KCP 只交付可替换适配边界和明确的 `NOT_SUPPORTED`。如果要求本次必须完成可运行 KCP，规格需扩大为 Phase 1+2，并在实现前增加 KCP 依赖/许可/维护性选型、cookie/session 防伪造、统一 timer scheduler、fuzz 和 `tc netem` 弱网矩阵。
