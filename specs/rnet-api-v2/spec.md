# RNet API v2 与延迟可观测性规格说明

## 需求概述

在保留 ABI v1 二进制兼容入口的前提下，提供面向普通调用方的精简消息 API，移除基础发送路径中没有实际语义的 `stream_id` 和强制 `request_id`；补齐可诊断的 poll 错误、单 session 关闭、延迟分位统计与周期结构化日志，并让 C++11/Go 包装提供完整、类型安全且默认参数友好的能力。

本轮不把标签透传伪装成多路复用。只有具备独立 stream 生命周期、顺序、流控和背压后，才引入 stream handle。延迟指标只描述库内可测阶段，不把无法推断的业务请求耗时或公网单向时延冒充为端到端延迟。

## 行为规格

### 正常流程

- 普通发送只要求 `session`、`msg_type` 和 `payload`；兼容帧内 `stream_id/request_id` 写零值。
- 高级调用方可通过可选 `send_options` 透传 `correlation_id`，普通调用方无需构造。
- poll 分别返回状态码和事件数量，使正常超时可与坏参数、失效句柄和 panic 区分。
- 调用方可关闭单个 session，不影响同 endpoint 的其他 session，并收到一次有序关闭事件。
- 系统记录发送排队、事件排队、连接、加密握手和认证等待耗时，使用有界直方图。
- 延迟快照返回样本数、P50/P90/P95/P99/max，单位固定为微秒并标明种类。
- 配置非零日志周期和 logger 后，异步输出结构化延迟摘要，不逐包同步打印。
- C++11/Go 默认入口提供合理默认值、自动 buffer 释放、可读错误及可选 join payload。
- C++ 可从持久化的 32 字节私钥恢复服务身份。

### 边界和异常

- ABI v1 的 `rnet_send`、`rnet_poll_events` 和 POD 布局保留并标记 deprecated，已有调用方行为不变。
- `rnet_session_send_ex` 的 options 可为空；非空时校验 `struct_size/abi_version`。
- 本轮不实现真实多路复用；旧 `stream_id` 仅作为 v1 兼容字段，不在新 SDK 暴露。
- 延迟记录路径固定内存、不得阻塞或调用外部 logger；使用单调时钟。
- 无样本时分位值为 0；直方图值溢出时封顶且不 panic。
- `poll_ex` 容量为零允许 events 为空；容量非零且 events 为空返回 `INVALID_ARGUMENT`。
- 重复关闭 session 的行为在 C/C++/Go 保持一致并由测试锁定。
- logger 队列满可丢弃摘要，但必须增加 `logs_dropped`，不得拖慢网络线程。

## 技术设计

### 模块划分

- **rnet-protocol**：保持线协议兼容；基础发送写零字段，高级关联标识复用原 `request_id`。
- **rnet-core**：事件增加内部入队时刻；poll 时计算事件排队延迟。
- **rnet-transport**：发送队列元素携带入队时刻；写 socket/KCP 时记录排队延迟；记录连接、握手、认证耗时。
- **rnet-observe**：实现固定桶并发直方图，热路径只做原子操作，输出分位快照。
- **rnet-ffi**：新增兼容扩展 API 和版本化 latency POD，v1 API 保留。
- **C++11/Go SDK**：提供精简 send、完整 close/metrics、默认配置、密钥导入和认证访问器。

### 关键 C ABI

```c
typedef struct rnet_send_options {
    uint32_t struct_size;
    uint32_t abi_version;
    uint64_t correlation_id;
    uint32_t flags;
    uint32_t reserved;
} rnet_send_options_t;

int32_t rnet_session_send(rnet_runtime_t runtime, rnet_session_t session,
                          uint32_t msg_type, rnet_slice_t payload);
int32_t rnet_session_send_ex(rnet_runtime_t runtime, rnet_session_t session,
                             uint32_t msg_type, rnet_slice_t payload,
                             const rnet_send_options_t *options);
int32_t rnet_poll_events_ex(rnet_runtime_t runtime, rnet_event_t *events,
                            size_t capacity, uint32_t timeout_ms,
                            size_t *out_count);
int32_t rnet_session_close(rnet_runtime_t runtime, rnet_session_t session,
                           int32_t reason);
```

首版延迟种类：`CONNECT`、`CRYPTO_HANDSHAKE`、`AUTH_WAIT`、`SEND_QUEUE`、`EVENT_QUEUE`、`KCP_RTT`、`KCP_UPDATE_DELAY`、`LOGGER_CALLBACK`。

```c
typedef struct rnet_latency_metric {
    uint32_t struct_size;
    uint32_t kind;
    uint64_t sample_count;
    uint64_t p50_us;
    uint64_t p90_us;
    uint64_t p95_us;
    uint64_t p99_us;
    uint64_t max_us;
} rnet_latency_metric_t;

int32_t rnet_latency_snapshot(rnet_runtime_t runtime,
                              rnet_latency_metric_t *metrics,
                              size_t capacity, size_t *out_count);
```

### 易用层

- C++11：`Session::send(msg_type, payload)` 为默认入口；`SendOptions` 为高级入口；`Keypair::from_private_key` 支持持久身份；Runtime 暴露 session/endpoint close、metrics 和 latency snapshot。
- Go：`NewRuntime()` 使用默认值，`NewRuntimeWithConfig` 用于调优；`Send` 不要求 stream/request；增加 `SendWithOptions`、关闭和 metrics API。
- 认证事件提供 `ClientPublicKey()` 和 `JoinPayload()`，调用方不手工切分字节。

### 纯度边界

- **纯逻辑**：options 映射、桶选择、分位计算、POD 转换、认证 payload 解析。
- **副作用**：时钟采样、网络写入、轮询、日志回调和 session task 终止。

## 兼容策略

- `RNET_ABI_VERSION` 保持 1；只新增函数符号和 POD，不改变已有签名和布局。
- C++/Go 以精简方法为主；旧方法改名为 `send_legacy`/`SendLegacy` 并标注废弃。
- 线协议保持 v1。基础 API 写零值，高级 `correlation_id` 复用 `request_id`；新 SDK 对外称 `CorrelationID`。
- 若确认从未有外部使用方，可后续单独删除 v1；本轮不假设允许破坏兼容性。

## 任务拆解

### 准备工作

- [x] 冻结 Rust/C++11/Go 测试基线并记录公开 ABI 布局。
- [x] 为新增 C ABI 和 v1 兼容增加保护测试。

### 实现任务

- [x] 任务 1：RED 精简 send/options 测试；GREEN 实现 `rnet_session_send(_ex)` 和 v1 转发。
- [x] 任务 2：RED poll 超时/坏句柄/空指针/panic；GREEN 实现 `rnet_poll_events_ex`。
- [x] 任务 3：RED 单 session 关闭和幂等；GREEN 实现定向关闭与 `rnet_session_close`。
- [x] 任务 4：RED 直方图边界/并发/无样本/分位；GREEN 实现有界延迟直方图。
- [x] 任务 5：RED 发送、事件队列和握手阶段快照；GREEN 接入埋点与 snapshot。
- [x] 任务 6：RED 周期日志不阻塞及丢弃；GREEN 增加 P50/P90/P95/P99 摘要。
- [x] 任务 7：RED C++11 易用 API、密钥恢复、认证解析、关闭和 metrics；GREEN 重构包装。
- [x] 任务 8：RED Go 默认配置、精简发送、认证解析、关闭和 metrics；GREEN 重构包装。
- [x] 任务 9：更新示例、README、头文件契约和发布包。

### 验收

- [x] fmt、clippy、`RUSTFLAGS='-D warnings' cargo test --workspace` 全通过。
- [x] C ABI 兼容、C++11 最终链接/安全回显、Go/cgo 测试通过。
- [x] 基础示例不再传 `stream_id/request_id`。
- [x] 分位满足 `p50 <= p90 <= p95 <= p99 <= max`，快照和周期日志可读取。
- [x] 检查实现范围并执行 `dserver-workflows:vsdd-adversarial-review`。

## 范围边界

- 不实现伪 stream；没有独立流控前不增加 `rnet_stream_open`。
- 不实现完整 RPC、请求超时表或自动业务响应关联；`correlation_id` 仅高级透传。
- 不声称能从通用消息流测出业务端到端耗时。
- 不在本轮执行长稳、完整公网弱网矩阵或 Windows/macOS 发布。

## 待确认决策

默认采用“ABI v1 二进制兼容 + 新增精简函数”，而不是删除旧接口；延迟首轮只实现库内可准确测量的阶段，以聚合方式输出 P50/P90/P95/P99，不逐包打日志。
