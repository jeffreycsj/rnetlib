# RNet Production v1 规格说明

## 需求概述

把 RNet 从功能完整的准生产版本提升为可执行生产准入的网络库：统一接口默认安全，所有传输路径具备明确的资源预算和拒绝服务保护；连接、错误、日志、指标和性能数据可以按时间、传输、端点、会话及原因追溯；Rust、C、C++11、Go 暴露一致的生产配置。保留现有 ABI 符号，但不安全的兼容接口与业务明文必须显式启用。

“生产版本”表示代码、测试、观测接口和准入工具完整，不表示在未知硬件和未知业务负载下无条件承诺吞吐；最终容量结论必须由目标环境的 SLO 压测产生。

## 行为规格

### 正常流程

- 当应用使用新版 runtime 配置时，系统应默认拒绝未认证兼容端点，并默认只允许加密业务数据。
- 当服务器显式允许业务明文时，客户端应自动跟随服务器的认证控制；接口应把该模式标记为不提供机密性或完整性的高风险策略，并产生安全审计事件。
- 当数据进入事件队列、会话发送队列、KCP 缓冲或握手状态时，系统应同时受条目数和总字节数预算约束。
- 当预算不足时，发送接口应同步返回明确错误；网络接收路径应只关闭或拒绝责任 session，并记录资源类型、端点、传输和原因。
- 当 TCP/UDP/KCP 会话发生 EOF、I/O、协议、密码、重放、认证、超时、限流或主动关闭时，`SessionClosed` 应携带真实且稳定的关闭原因。
- 当调用方查询观测数据时，系统应提供累计 counter、当前 gauge、峰值、分原因 counter，以及可按时间窗口轮转的 P50/P90/P95/P99/P99.9/max 延迟。
- 当配置结构化日志时，每条记录应包含时间戳、级别、事件名、runtime/endpoint/session、transport、error code 和可选 correlation ID；慢日志消费者不得阻塞 I/O。
- 当 C、C++11 或 Go 创建 runtime 时，应可配置与 Rust 一致的 session、握手、空闲、队列字节、日志和安全策略。
- 当 UDP/KCP 客户端使用多地址域名时，握手失败应自动尝试后续候选地址，并产生每次尝试的诊断事件。
- 当 KCP 服务端收到未知来源时，cookie 验证完成前不得创建长期 KCP engine 或 Noise/session 状态。

### 边界和异常

- 旧 ABI 符号继续导出；不修改已有结构布局。新增 `*_v3` 创建接口和带 `struct_size` 的扩展结构。
- 旧 `rnet_runtime_create` 保持 ABI 可调用，但兼容明文 endpoint 的启用状态必须在文档和日志中明确；新版 SDK 只走安全默认且资源上限完整的 v5 路径。
- 生命周期事件不得被另一条生命周期事件静默覆盖。若保留容量无法提交关键事件，系统应进入可观察的过载状态并返回/记录明确错误。
- 日志和指标中不得记录私钥、Noise 密钥、cookie、完整认证票据或业务 payload。
- 每 IP 限制必须正确处理 IPv4、IPv6 和可配置的 IPv6 前缀聚合；不能依赖容易伪造的应用字段。
- DNS 解析不得运行在 Tokio I/O worker 上；每个候选连接和整个解析/连接过程都必须有截止时间。
- 明文策略不能被客户端请求或网络报文单方面打开，只能由已认证的服务器控制流改变。
- 自动 rekey 应支持按时间和加密字节阈值触发，并保证切换期间消息不丢失、不重排。

## 技术设计

### 模块划分

- **安全策略**：新增 `SecurityPolicy`，控制是否允许业务明文、是否允许兼容端点、自动 rekey 阈值；新版默认全部采用安全值。
- **资源预算**：新增 runtime/endpoint/session 分层 `ResourceBudget`，统一追踪 queued bytes、event bytes、pending handshakes、sessions 和 KCP peers。
- **准入控制**：TCP 所有 listener 使用 semaphore；UDP/KCP 使用总量与 IP/prefix 令牌桶。KCP 增加原始 UDP cookie preflight，通过后才创建 engine。
- **生命周期与错误**：内部统一为 `CloseCause`，从底层错误映射到公开稳定错误码及 reason counter，禁止无原因的 `OK` 异常关闭。
- **观测模块**：增加窗口化直方图、gauges、分类 counters、结构化日志 v2 以及无额外依赖的 Prometheus 文本快照/通用 exporter callback。
- **数据报调度**：用 deadline heap 管理 KCP 更新，移除每 5ms 全表扫描；将收包、到期更新和发包限制为公平批次，避免单 peer 饥饿其他连接。
- **地址模块**：提取可测试的候选排序与重试状态机，UDP/KCP 在握手失败后切换地址；TCP 保留每候选 connect timeout。
- **SDK/ABI**：新增 C v3 配置和结构化日志/指标接口；C++11、Go 使用 v3，并保留显式命名的 legacy API。
- **生产验证**：扩展 fuzz target、故障注入、资源预算测试、并发/弱网/soak 工具，生成带环境信息、CPU、RSS、吞吐和尾延迟的结果。

### 关键接口

```rust
pub struct SecurityPolicy {
    pub allow_plaintext_business_data: bool,
    pub allow_legacy_unauthenticated_endpoints: bool,
    pub rekey_after: Option<Duration>,
    pub rekey_after_bytes: Option<u64>,
}

pub struct ResourceLimits {
    pub max_endpoints: usize,
    pub max_sessions_per_endpoint: usize,
    pub max_pending_handshakes: usize,
    pub max_sessions_per_ip: usize,
    pub max_event_bytes: usize,
    pub max_runtime_queued_bytes: usize,
    pub max_session_queued_bytes: usize,
}

pub struct RuntimeConfig {
    // 现有字段保持
    pub security_policy: SecurityPolicy,
    pub resource_limits: ResourceLimits,
    pub connect_timeout: Duration,
    pub dns_timeout: Duration,
}

pub fn metrics_snapshot_v2(&self) -> MetricsSnapshotV2;
pub fn drain_metric_series(&self, out: &mut [MetricSeries]) -> usize;
```

C ABI 新增 `rnet_config_v3_t`、`rnet_runtime_create_v3`、`rnet_metrics_snapshot_v2` 和 `rnet_logger_v2_t`；socket 调优使用冻结的 v4，全局 endpoint/handshake 预算及其 gauges 使用 v5 配置和 v3 指标，已有结构和函数保持二进制布局不变。

### 纯度边界

- **纯逻辑**：预算预留/释放、关闭原因映射、限流决定、DNS 候选状态机、KCP deadline 排序、指标标签编码。
- **副作用**：socket/DNS、Noise/KCP 状态、系统时钟、日志 callback、文件描述符和 FFI buffer 生命周期。

## 任务拆解

### 准备工作

- [x] 固化 Rust 测试、C++11/Go smoke 和性能探针基线。
- [x] 为关闭原因、资源预算、旧接口策略、KCP preflight、窗口指标补 RED 测试。

### 实现任务

- [x] 1. 统一关闭原因，修复 TCP/UDP/KCP 错误吞掉和 `status=OK` 异常关闭。
- [x] 2. 重构事件队列，保证关键生命周期/错误事件不被静默覆盖，并记录分类型丢弃。
- [x] 3. 实现 runtime/session 字节预算、KCP 未确认数据预算、队列 gauges 和峰值，覆盖成功/失败/关闭释放路径。
- [x] 4. 给兼容 TCP/UDP/secure listener 补准入上限、超时和空闲回收；新增安全默认策略。
- [x] 5. 实现每 IP/prefix 握手限额与令牌桶，并将拒绝纳入指标。
- [x] 6. 实现 KCP stateless cookie preflight 和 deadline heap/fair batch 调度。
- [x] 7. 增加自动 rekey 和明文策略审计，保持服务器权威及客户端透明。
- [x] 8. 实现结构化日志 v2、分类 metrics、gauges、窗口直方图和 Prometheus 快照。
- [x] 9. 新增 C ABI v3/v4/v5，并同步 C++11、Go 配置、结构化日志、指标和错误类型。
- [x] 10. 完成 DNS 多地址 TCP/UDP/KCP 跨地址族重试、解析 deadline、整体连接 deadline 和候选连接 timeout。
- [x] 11. 加入 TCP_NODELAY/可配置 socket buffer；KCP/UDP 使用公平批处理并限制临时与未确认数据内存。
- [x] 12. 扩展 frame/control/cookie/KCP/FFI fuzz，CI 加入 cargo-audit/deny、fuzz smoke 和可选 Miri。
- [x] 13. 增加并发、慢消费者、内存预算、确定性弱网测试和 soak runner，更新生产部署手册及威胁模型。
- [x] 14. 增加 runtime 全局 endpoint/pending-handshake 预算、当前/峰值 gauges 和八类准入拒绝原因。

### 验收

- [x] Rust、C ABI、C++11、Go 全量测试和 Clippy 全部通过。
- [x] 所有已覆盖异常关闭都具有可验证的非 OK 原因；关键错误具有关闭原因或结构化事件维度。
- [ ] 压力下 RSS 受配置字节预算约束，预算释放无泄漏，慢 peer 不影响其他 peer。
- [x] 伪造源无法在 cookie 验证前占满 KCP engine；每 IP/prefix 限流生效。
- [x] 日志和指标能从一次连接失败追溯到 endpoint/session/transport/reason 和时间窗口。
- [x] SDK 普通路径不暴露 transport 专用发送参数，并能配置生产限制。
- [x] CI 自动执行依赖安全检查；关键解析器与状态机具备 fuzz target/corpus。
- [ ] 目标环境完成弱网矩阵和至少 24 小时 soak，报告包含 CPU、RSS、吞吐、错误率及 P99/P99.9。
- [x] 执行 `vsdd-adversarial-review`，本轮发现的高置信问题已修复并加入回归。

## 待确认决策

1. 默认策略：新版 v5/SDK 默认禁用业务明文和未认证兼容端点；旧 C ABI 符号继续存在。
2. 明文语义：继续保留“无机密性、无完整性”的既有 wire 语义，但必须由服务端显式允许并产生审计记录；本阶段不发明 MAC-only 新协议。
3. 企业容量：代码提供预算与准入工具；最终吞吐/连接数 SLA 使用目标部署机器实测，不在代码中写死未经确认的数字。
4. 平台范围：先完成 Linux x86_64 生产准入；Windows/macOS 作为后续平台任务。
