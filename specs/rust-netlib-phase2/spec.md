# Rust Netlib Phase 2 规格说明

## 目标

在保持 Phase 1 C ABI 可兼容扩展的前提下，补齐可运行 KCP、C++11 包装、显式客户端 join，以及独立于 TCP/UDP/KCP 连接状态的应用层安全握手。传输类型在 endpoint 创建时确定并在其全部 session 生命周期内不可变；发送 API 不接受 transport 参数。

本阶段的“安全”含义是：握手后业务帧具备机密性、完整性、对端身份校验和重放防护。它不是正式安全认证；采用的 Rust Noise 实现仍需在生产部署前结合威胁模型做外部审计。

## 已确认事实与兼容策略

- 现有 `rnet_endpoint_config_t.transport` 已在 `rnet_endpoint_open` 时选择传输，`rnet_send` 只接收 session；保留该设计并增加不可变性测试。
- 现有 TCP client/UDP remote 能建立传输，但没有统一的应用层 join/认证完成语义；旧接口保留为低层兼容入口，新接口以握手成功作为 session 可用条件。
- 现有 C++ 包装使用 `std::string_view` 和 `std::exchange`，实际要求 C++17/C++14；改写为严格 C++11。
- ABI 结构继续只在尾部追加字段，并使用 `struct_size` 做版本兼容。ABI 主版本不破坏性升级；新增能力通过 feature bits 查询。

## 对外行为规格

### endpoint 与传输

- listener/client endpoint 创建时必须指定且只指定一种 `TCP`、`UDP` 或 `KCP`。
- endpoint 创建成功后，transport 不可修改；其产生的 session 继承同一 transport。
- `send`、`close session`、`auth decision` 等 session API 不得再次接收 transport。
- TCP listener、UDP listener 和 KCP listener 都向上层呈现逻辑 session；TCP 的 accept 只表示底层可用，不表示应用 session 已建立。

### 客户端 join

- 新增显式 `rnet_client_join`，输入已经固定 transport 的 client endpoint、远端地址和 join payload，返回 join attempt handle。
- TCP client endpoint 可在 endpoint 打开时配置远端；UDP/KCP client endpoint 可由每次 join 指定远端，但一个 session 的远端和 transport 建立后不可变。
- 客户端事件序列为 `JOIN_STARTED -> PEER_AUTH_REQUIRED/自动验证 -> SESSION_ESTABLISHED`，失败为 `JOIN_FAILED`，并带稳定错误原因。
- 服务端在加密握手完成并解密 join payload 后收到 `AUTH_REQUEST`；调用 `rnet_session_auth_decide` 接受或拒绝。只有接受后双方才收到 `SESSION_ESTABLISHED`，此后才允许普通业务发送。
- join/认证具有独立超时和可取消语义；重复决定、过期 attempt、握手中发送业务包均返回稳定错误。

### 应用层握手与加密

- TCP connect/accept、UDP 首包和 KCP conv 建立都只进入 `TRANSPORT_READY/HANDSHAKING`，不能直接产生已连接事件。
- 统一会话状态机：`NEW -> COOKIE_CHECK -> CRYPTO_HANDSHAKE -> AUTH_PENDING -> ESTABLISHED -> DRAINING -> CLOSED`。TCP 可跳过 cookie，但不能跳过加密握手。
- 默认握手套件为 `Noise_XX_25519_ChaChaPoly_BLAKE2s`；依赖固定到已包含已知修复的 `snow 0.10.x` 精确版本，并使用锁文件。密钥材料使用 `zeroize` 清除。
- 默认认证模型：客户端必须固定服务端静态公钥；服务端通过轮询到的 `AUTH_REQUEST` 校验客户端静态公钥和加密 join payload，可选择接受或拒绝。不得提供“无验证即成功”的默认配置。
- 最小明文 record header 只包含协议 magic/version、record kind、密文长度、会话路由 token 和显式序号；原 28 字节业务帧整体放入密文，避免暴露 `msg_type/request_id/body`。
- TCP/KCP 使用有序 Noise transport state；原始 UDP 使用显式 64 位 nonce 的 stateless transport，并维护滑动重放窗口，以允许有限乱序而不因丢包造成 nonce 状态失步。
- 每个方向独立 nonce；计数器耗尽、重复 nonce、窗口外旧包、标签校验失败均关闭会话并产生安全错误事件。不得自动回退为明文。
- 握手 transcript 绑定协议版本、transport、endpoint role 和双方临时会话随机数，防止跨协议/跨角色重放。
- UDP/KCP 在分配昂贵会话状态或放大响应前先验证服务端生成的无状态 cookie；限制每 IP/endpoint 的并发握手和失败速率。
- 日志、错误文本、metrics 和回调不得包含私钥、原始 join token、完整密文或可复用 cookie。

### KCP

- 不自行发明 KCP 算法。实现前对候选 Rust crate 与官方 `ikcp.c` 的版本、许可、维护状态、unsafe 面和已知问题做记录，选定版本后精确固定。
- KCP 运行在 UDP socket 上，由统一 timer scheduler 驱动 update/check，不为每个 session 创建永久线程或独立高频定时器。
- KCP endpoint 创建时固定 MTU、send/receive window、interval、nodelay/resend/nc；运行中只能修改明确标注为安全的调优项。
- conv/session token 必须由握手协商且不能作为身份凭据；cookie 和 Noise 完成前不得交付业务数据。
- 覆盖丢包、乱序、重复、延迟、MTU 边界、conv 冲突、超时清理、握手洪泛和优雅停止。

### C++11 与客户端包装

- `cpp/rnet.hpp` 必须能以 `-std=c++11 -Wall -Wextra -Werror` 编译；移除 `string_view`、`exchange` 和其他 C++14/17 依赖。
- 提供 `Listener`、`Client`、`JoinAttempt`、`Session` 的 move-only RAII 包装；析构不抛异常。
- payload 接口同时支持 `const void* + size_t` 和 `std::string`，二进制数据不能依赖 NUL 结尾。
- C++ 与 Go 都暴露 listener、client join、认证决定、握手事件、发送和关闭；两种包装不得绕过 C ABI 的状态校验。

## 主要接口草案

以下名称允许在实现时做不影响语义的小幅调整：

- `rnet_endpoint_open(runtime, endpoint_config, out_endpoint)`：保留，transport 在此固定。
- `rnet_client_join(runtime, endpoint, join_config, out_attempt)`：发起逻辑连接和安全握手。
- `rnet_session_auth_decide(runtime, session, decision)`：服务端接受/拒绝认证请求。
- `rnet_session_close(runtime, session, reason)`：显式关闭逻辑 session。
- `rnet_keypair_generate(keypair)`：生成 X25519 静态密钥；私钥所有权和清零规则写入头文件契约。
- `rnet_features()`：查询 TCP/UDP/KCP、Noise、C++11 等能力位。

配置新增：本地静态私钥、期望服务端公钥、握手/认证超时、join payload 上限、cookie secret/轮换周期、重放窗口及握手限流。所有密钥指针仅在调用期间借用，库内部需要长期使用时必须复制到受控且可清零的内存。

## 错误与边界

- 新增或细化 `HANDSHAKE_REQUIRED`、`HANDSHAKE_FAILED`、`AUTH_REJECTED`、`PEER_KEY_MISMATCH`、`REPLAY_DETECTED`、`CRYPTO_ERROR`、`RATE_LIMITED` 和 `CANCELLED`。
- 0 字节 join payload 合法；超限 payload 在分配/加密前拒绝。
- 畸形 record、截断握手、未知版本、错误角色、错误 transport binding、伪造 cookie 和 AEAD 失败均不得 panic、越界或泄露可区分的密钥信息。
- 同一 attempt/session 的事件有序；跨 session 不承诺全局顺序。
- runtime stop 时拒绝新 join，在 deadline 内通知并关闭已建立 session，未完成握手立即取消并回收。

## 实现拆解与 TDD 顺序

1. 建立 Phase 1 全量测试基线；安装项目所需的 stable Rust toolchain、rustfmt 和 clippy，不替换不可逆的系统配置。
2. RED：增加 C++11 编译/链接测试；GREEN：改写 C++ 包装并补 client/listener RAII。
3. RED：增加 endpoint transport 不可变和发送接口无 transport 的 ABI/编译期测试；GREEN：固化 capability/session 元数据。
4. RED：为纯状态机、record codec、超时、认证决定、nonce/replay window 写单测；GREEN：新增独立 `rnet-security` 模块和事件类型。
5. RED：TCP client/server 只有完成 Noise + auth 才能发送；GREEN：接入握手、加密 record 与 client join。
6. RED：UDP cookie、乱序、丢包、重放与伪造包测试；GREEN：接入 stateless encryption 和逻辑 session。
7. 完成 KCP 依赖审查和固定；RED：虚拟时钟/弱网测试；GREEN：实现 scheduler、KCP endpoint 和 Noise 集成。
8. 扩展 C ABI、C++11、Go API 与端到端示例，补空指针、生命周期、并发 destroy/poll/join/auth 测试。
9. 运行 fmt、clippy、workspace tests、C/C++11/Go 最终链接、fuzz smoke、弱网矩阵和 sanitizer/解释器可行项；检查 diff 后执行对抗性审查。

## 验收标准

- TCP、UDP、KCP 均能由客户端 API join 服务端，并且 transport 在 endpoint 创建后不可变。
- 抓取环回流量时看不到业务帧头和 payload 明文；篡改、重放、错误公钥和未认证发送均被拒绝。
- UDP 丢包/乱序不会造成后续合法包永久解密失步；KCP 在规定弱网矩阵下完成握手、双向回显和清理。
- C++ 示例严格以 C++11 编译并真实链接静态库；Go/cgo 示例同样完成安全 join 和回显。
- 所有队列、握手表、重放窗口和 join payload 有上限；慢认证方和洪泛方不会导致无限内存增长。
- `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`RUSTFLAGS='-D warnings' cargo test --workspace` 全部通过。
- 交付依赖/许可清单、安全限制和已验证平台；不把未审计实现描述为“生产级安全认证”。

## 本轮范围边界

本轮完成 Linux x86_64 上的功能、跨语言接入、KCP 弱网验证和质量工具闭环。固定 QPS/延迟目标、io_uring、Windows/macOS 原生产物仍属于后续平台与性能阶段，需要目标硬件、系统和指标后单独验收。

## 待确认决策

默认采用“客户端固定服务端静态公钥；服务端收到加密后的客户端公钥与 join payload，再由应用显式接受/拒绝”的认证模型。该模型支持匿名注册、token、账号票据或客户端公钥白名单，同时保持服务端身份强校验；确认后按此规格进入 TDD。

## 实施结果

- [x] C++11 包装与严格编译/最终链接测试。
- [x] endpoint/listener 创建时固定 TCP、UDP 或 KCP；session 发送接口不包含 transport。
- [x] TCP/UDP/KCP 的 Noise XX 握手、服务端显式认证决定和客户端 join。
- [x] UDP/KCP 无状态 cookie、显式 nonce、乱序容忍和重放窗口。
- [x] KCP 0.6.0 适配、丢包/乱序恢复和 4096 字节端到端安全传输。
- [x] C ABI、C++11、Go/cgo 接口与独立发布目录验收。
- [x] 安装隔离 rustfmt、clippy、cargo-fuzz/nightly；fmt、clippy、MSRV 测试和 10 秒 ASan/libFuzzer smoke 通过。
- [x] Phase 2 对抗性复审及高置信问题修复。

宽参数 `tc netem`、24–72 小时 soak、第三方密码审计、Windows/macOS 产物和固定性能指标仍按“本轮范围边界”留给后续平台/生产准入阶段。
