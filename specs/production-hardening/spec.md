# RNet Production Hardening 规格说明

## 需求概述

修复统一 `listen`/`connect` 路径中的消息静默丢失、连接状态泄漏、共享端点背压、TCP 慢连接和延迟指标缺失问题，并把 Rust、C、C++11、Go 的推荐接口收敛为易用且行为一致的服务端/客户端 API。客户端目标地址支持数字 IP 或 DNS 主机名；服务端监听保持本机 IP/通配地址语义。

## 行为规格

### 正常流程

- 当应用调用统一发送接口并收到成功时，消息应已进入一个保证后续处理或明确失败通知的有界队列，不能在后台静默丢弃。
- 当 UDP 消息超过包含协议和加密开销后的实际数据报上限时，发送接口应同步返回 `MESSAGE_TOO_LARGE`。
- 当 KCP 消息不超过 `max_body_len` 时，系统应交给 KCP 分片；`max_datagram_size` 只约束 KCP 包 MTU，不限制完整业务消息。
- 当应用关闭 UDP/KCP session、连接超时或认证失败时，系统应同时回收 session route、peer 状态、重传缓存、deferred 队列和 KCP engine。
- 当事件消费变慢时，一个 session 的背压不能停止同 endpoint 上其他 peer 的收包、KCP ACK、重传和定时更新。
- 当客户端传入数字 IP 或 DNS 主机名时，系统应解析出匹配的 IPv4/IPv6 地址并自动选择相同地址族的本地通配 bind 地址。
- 当服务器发起安全模式切换或 rekey 时，完成事件应提供类型化的模式、epoch 和操作类型。
- 当调用统一 API 时，Rust、C、C++11、Go 的普通发送接口都只要求 session、消息类型和 payload；关联 ID 通过高级 options 提供。

### 边界和异常

- TCP accept 后必须从第一个字节开始受握手超时和并发准入上限保护；达到上限时拒绝新连接且不创建 session/task。
- UDP/KCP 的握手、应用认证、安全迁移和空闲状态都必须有截止时间；超时只关闭对应 peer。
- KCP 在 cookie 验证前不得保留长期 engine；非法首包和伪造来源不能永久占满全局 peer 配额。
- 已建立 datagram peer 再发送 `ClientHello` 时不得覆盖并泄漏旧 session；本版选择拒绝，重连必须先关闭旧 session 或等待其超时。
- 生命周期事件不得出现“接口返回失败但状态已经不可逆改变且事件丢失”的半提交结果。
- DNS 解析失败、连接超时和所有同步 FFI 错误必须保留可读取的详细错误文本。
- `PLAINTEXT` 业务数据仍按既有需求允许，但文档必须继续明确其不具备机密性和完整性。
- 完整 URL（scheme/path/query）不作为目标地址格式；客户端输入为 `host + port`。服务端 bind 不解析公网域名。

## 技术设计

### 模块划分

- **地址与连接模块**：`connect_host` 在调用线程解析 DNS，避免占用 I/O worker；TCP 按 IPv4/IPv6 候选和连接超时重试，datagram 自动选择地址族并由会话事件报告握手结果。
- **准入与生命周期模块**：为 TCP pending/active session、datagram pending/active peer 设置有界配额和 deadline；由 endpoint owner 统一执行关闭与资源回收。
- **Datagram wire 模块**：区分 UDP wire MTU 与 KCP message limit，提供 `remove_peer`；KCP 使用短生命周期预认证状态或 stateless cookie 前置流程。
- **事件模块**：共享 datagram driver 不等待事件队列；业务事件过载时只关闭对应 session，生命周期事件优先发布并保持总容量有界。
- **观测模块**：统一路径补齐 connect、handshake、auth wait、send queue、event queue、KCP RTT/update delay 和真实成功发送字节数。
- **SDK 模块**：Rust 简化 `send`，旧字段移入 `send_legacy`；所有 SDK 增加类型化安全事件和详细错误读取；兼容 C ABI 保留旧入口。

### 关键接口

```rust
pub struct HostClientConfig {
    pub transport: Transport,
    pub host: String,
    pub port: u16,
    pub join_payload: Vec<u8>,
}

pub fn send(&self, session: Handle, msg_type: u32, payload: &[u8]) -> Result<()>;
pub fn send_with_options(
    &self,
    session: Handle,
    msg_type: u32,
    payload: &[u8],
    options: SendOptions,
) -> Result<()>;
pub fn send_legacy(/* 旧 stream_id/request_id 参数 */) -> Result<()>;
```

- C ABI 保留现有 `rnet_client_connect_v2` 结构布局，现有 `remote_host` 从“仅数字 IP”扩展为“IP 或 DNS 主机名”。
- C++11/Go 继续使用 `Host + Port`，无需新增 transport 专用方法。
- 新增线程局部或 caller-owned 的 FFI 详细错误读取接口，SDK 错误对象同时包含 code 与 message。
- `SecurityChanged` 在 SDK 中解析为 `{operation, mode, epoch}`；旧 C event 字节布局保持兼容，必要时通过新增 accessor 解码。

### 纯度边界

- **纯逻辑**：wire 大小计算、地址候选排序、状态转移、超时判定、事件 payload 编解码。
- **副作用**：DNS、socket bind/connect、KCP update、事件投递、日志和 FFI buffer 生命周期。

## 任务拆解

### 准备工作

- [x] 运行现有 Rust/C/C++11/Go 测试基线并记录结果。
- [x] 为当前统一接口补充大消息、关闭回收、事件队列满和慢握手保护测试，确认 RED。

### 实现任务

- [x] 修复 UDP 实际 wire MTU 计算、KCP 完整消息分片和异步错误吞掉问题。
- [x] 重构 datagram session close/timeout，完整回收 peer、engine、pending、seen 和 deferred 状态。
- [x] 修复 KCP 预认证 engine 占用和重复 `ClientHello` 覆盖旧 session。
- [x] 增加 TCP 首包超时及 pending/active session 准入限制。
- [x] 重构事件背压，保证共享 UDP/KCP driver 不因单个满队列停摆，生命周期事件可收敛到最终状态。
- [x] KCP 按 engine deadline 跳过未到期更新，并复用热路径地址和输出缓冲。
- [x] 补齐统一路径延迟/流量指标及 P90/P95/P99 日志数据来源。
- [x] 增加客户端 IP/域名地址类型、IPv4/IPv6 自动 bind 和连接超时。
- [x] 统一 Rust 简化发送、类型化认证/安全事件以及 C/C++11/Go 详细错误接口。
- [x] 更新完整示例、配置说明、兼容性说明和生产部署限制。

### 验收

- [x] Rust、C ABI、C++11、Go 全量测试通过。
- [x] UDP 边界消息要么可靠入队，要么同步返回明确错误；不存在 `Ok` 后静默丢弃。
- [x] KCP 可传输明显大于单个 MTU 的业务消息并在丢包/乱序下恢复。
- [x] 关闭与超时会释放 peer/engine/session 及相关缓存。
- [x] 伪造或非法 KCP 首包无法永久耗尽准入容量。
- [x] 满事件队列不会阻塞共享 datagram driver，过载仅关闭对应 session。
- [x] 慢 TCP 客户端在截止时间内回收，连接数不超过配置上限。
- [x] 数字 IP、IPv4/IPv6 域名候选、DNS 失败和连接失败均有可诊断结果。
- [x] 统一路径的全部 latency kind 有对应数据来源和关键路径测试。
- [x] 执行 `vsdd-adversarial-review` 并处理高置信问题。
