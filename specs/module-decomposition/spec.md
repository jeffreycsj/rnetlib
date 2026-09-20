# RNet 模块拆分规格说明

## 需求概述

在不改变公开 C ABI、C++11/Go 调用方式、线协议和运行行为的前提下，拆除 `rnet-transport`、`rnet-ffi` 和 SDK 中的巨型单文件结构，让调用链可按“运行时 → 端点 → 会话 → 传输实现 → 可观测性”直接定位。此次只做结构重构与必要的可见性收敛，不顺带新增协议或功能。

## 行为规格

### 正常流程

- 当使用方包含 `rnet.h`、`rnet.hpp` 或导入 Go `rnet` 包时，现有公开符号、方法名和行为应保持不变。
- 当维护者查找 TCP、UDP、KCP、安全握手、session 或 metrics 实现时，应能从同名模块直接定位，而不是在数千行 `lib.rs` 中搜索。
- 当新增一种传输或观测指标时，应只修改对应驱动模块和受控的 runtime/registry 接口。

### 边界和异常

- 不改变 C POD 布局、`RNET_ABI_VERSION`、导出符号及错误码。
- 不改变线协议、握手顺序、队列容量、关闭语义或延迟统计口径。
- 不通过无约束 `pub` 暴露内部共享状态；跨模块能力优先使用窄的 `pub(crate)` 接口。
- 拆分后禁止形成 `utils.rs`、`common.rs` 之类新的职责垃圾桶。

## 技术设计

### Rust transport

- `lib.rs`：模块声明和稳定公开 re-export，目标不超过约 200 行。
- `config.rs`：运行时、端点和安全配置。
- `runtime.rs`：`NetworkRuntime` 公开操作及生命周期编排。
- `state.rs`：共享状态、route/record、事件构造和 session registry 原语。
- `metrics.rs`：计数器和延迟种类/快照。
- `kcp.rs`：KCP adapter、ACK RTT 解析和 update 调度辅助。
- `tcp.rs` / `udp.rs`：明文传输驱动。
- `secure_tcp.rs` / `secure_datagram.rs`：Noise 握手、认证和加密状态机。

传输驱动不得直接操作彼此的私有状态，通过 `state.rs` 的窄接口注册、查询和删除 session。

### Rust FFI

- `lib.rs`：模块声明与公开 re-export。
- `abi.rs`：常量、POD、默认值和枚举。
- `registry.rs`：runtime handle table、lease、buffer 与公共校验辅助。
- `runtime.rs` / `endpoint.rs` / `session.rs` / `events.rs` / `observe.rs`：按对外能力分组。

每个 `#[no_mangle]` 函数仍由 crate 导出；`lib.rs` re-export 保持现有 Rust 测试路径兼容。

### SDK

- C++：保留 `rnet.hpp` 作为唯一入口，内部拆为 `rnet/types.hpp`、`rnet/keypair.hpp`、`rnet/runtime.hpp`；发布脚本复制完整头文件树。
- Go：按 config/keypair/event/endpoint/session/observe/runtime 拆分；cgo 指令集中在 `native.go`，shim 集中在私有 `native.h`。
- `include/rnet.h` 是 C ABI 清单，当前 260 行规模合理，保留单文件。

## 结构约束

- 新 `rnet-transport/src/lib.rs` 与 `rnet-ffi/src/lib.rs` 各不超过约 200 行。
- 单个实现文件原则上不超过 600 行；超过时必须有明确、不可再分的状态机理由。
- 禁止以 `misc/helpers/common` 命名模块。
- 用编译器推动依赖收敛，不把所有内部类型改成公开类型来省事。

## 任务拆解

### 准备工作

- [x] 记录公开符号、文件规模和当前 Rust/C++11/Go 全量基线。
- [x] 增加 C 头文件与公开导出符号的结构保护检查。

### 实现任务

- [x] 拆分 `rnet-transport` 的 config/metrics/KCP/state。
- [x] 拆分明文 TCP、UDP驱动。
- [x] 拆分安全 TCP、UDP/KCP 状态机。
- [x] 拆分 `rnet-ffi` 的 ABI、registry 和功能入口。
- [x] 拆分 C++11 umbrella header并更新发布脚本。
- [x] 拆分 Go 包并集中 native/cgo 边界。
- [x] 更新架构文档和目录说明。

### 验收

- [x] `lib.rs` 和单文件规模满足结构约束。
- [x] rustfmt、Clippy、Rust 全量测试通过。
- [x] C++11 普通/安全回显及 Go/cgo 测试通过。
- [x] 从最终 dist 目录重新链接通过。
- [x] 对抗性审查确认没有循环依赖、过宽可见性或 ABI 漂移。
