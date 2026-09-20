# RNet 透明安全会话与使用文档规格说明

## 需求概述

服务端在创建监听器时决定初始安全策略；客户端只使用统一的 connect/join、send、poll 接口，不选择明文或加密分支。网络库完成能力协商、握手、加解密和会话内安全状态迁移，业务消息接口始终不暴露密码学状态。同时补齐 C、C++11、Go、Rust 公共接口契约注释，以及可复制运行的服务端/客户端使用指南。

旧的 C ABI、C++/Go 方法和线协议入口继续保留，新增统一接口，不直接删除兼容 API。

## 行为规格

### 正常流程

- 当服务端创建监听器时，应通过 `security_policy` 明确选择 `PLAINTEXT` 或 `ENCRYPTED_REQUIRED`，传输类型仍在监听时固定为 TCP、UDP 或 KCP。
- 当客户端连接任意监听器时，应只调用统一的 connect/join 接口；库自动识别服务端策略并完成应用握手，业务层的 send/poll 不区分是否加密。
- 当服务端要求加密时，客户端应在交付 `SESSION_OPENED` 前完成服务端身份验证、Noise 握手和服务端业务认证；任何握手前发送继续返回 `HANDSHAKE_REQUIRED`。
- 当服务端初始为明文并在会话中请求升级为加密时，双方应使用控制帧、epoch 和确认屏障完成切换；切换期间业务消息保持有序且不以错误密钥解释。
- 当已加密会话需要更换密钥时，应自动 rekey，调用方的 session handle、send 和 poll 行为不变。
- 当调用方使用 C、C++11、Go 或 Rust 时，应能从入门文档分别找到最小服务端、最小客户端、认证、发送、轮询、关闭和延迟指标示例。

### 边界和异常

- 客户端不得根据可篡改的明文协商结果静默降级；若本地信任策略要求加密，而服务端声明明文，应拒绝连接。
- 服务端配置为 `ENCRYPTED_REQUIRED` 时，不接受旧明文客户端，不回退到明文。
- 安全迁移必须是双阶段提交：发起方发送新 epoch 提议，接收方准备完成并 ACK 后，双方才切换发送状态；超时或校验失败时关闭 session，不允许两端继续处于不同状态。
- UDP/KCP 必须按 epoch 区分迁移前后的乱序包；旧 epoch 只在有界过渡窗口内可接收，窗口结束后拒绝和计数。
- 控制帧不得作为业务 `MESSAGE` 事件上抛；控制帧大小、并发迁移数和缓存必须有界。
- 已加密会话允许由服务端请求切回明文。降级命令必须在旧加密 epoch 内认证并经双方确认；切换完成后的业务数据不再具备机密性和完整性。
- 客户端通过 runtime 级 `peer_verifier` 验证首次见到的服务端身份；不得把“加密了”误写成“已认证”。

## 技术设计

### 模块划分

- **rnet-protocol/control**：独立的连接前言、协商消息、安全 epoch、升级/确认/rekey 控制帧；业务帧格式保持兼容。
- **rnet-security/session**：`Plaintext`、`Negotiating`、`Encrypted`、`Rekeying` 状态机，以及服务端身份验证和有界旧 epoch 接收窗口。
- **rnet-transport/session**：统一 session route；send 根据当前安全状态自动封装，receive 自动验证、解密后才生成业务事件。
- **rnet-transport/policy**：服务端监听策略和客户端信任策略，传输选择与安全选择互相独立。
- **rnet-ffi**：新增版本化 server/client 配置与统一 open/connect API；旧 `rnet_endpoint_open`、`rnet_listener_open`、`rnet_client_join` 保留。
- **C++11/Go SDK**：默认暴露 `listen(options)`、`connect(options)`，接口名不包含 TCP/UDP/KCP，传输只通过 options 的 `transport` 枚举选择；旧方法保留并标记 deprecated。
- **docs/examples**：README 快速入口、完整 getting-started 文档和可编译的 C/C++11/Go 示例。

### 建议关键接口

```c
enum {
  RNET_SECURITY_PLAINTEXT = 1,
  RNET_SECURITY_ENCRYPTED = 2
};

typedef struct rnet_server_config_v2 {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t transport;
  uint32_t initial_security;
  rnet_slice_t bind_host;
  uint16_t bind_port;
  rnet_slice_t local_private_key; /* encrypted policy requires 32 bytes */
} rnet_server_config_v2_t;

typedef struct rnet_client_config_v2 {
  uint32_t struct_size;
  uint32_t abi_version;
  uint32_t transport;
  rnet_slice_t remote_host;
  uint16_t remote_port;
  rnet_slice_t join_payload;
} rnet_client_config_v2_t;

int32_t rnet_server_open_v2(rnet_runtime_t runtime,
                            const rnet_server_config_v2_t *config,
                            rnet_endpoint_t *out);
int32_t rnet_client_connect_v2(rnet_runtime_t runtime,
                               const rnet_client_config_v2_t *config,
                               rnet_endpoint_t *out);
int32_t rnet_session_security_set(rnet_runtime_t runtime,
                                  rnet_session_t session,
                                  uint32_t policy);
```

客户端身份材料和服务端验证器不放在每次 connect 参数中，而由 runtime 级 `identity_provider` / `peer_verifier` 提供；这样 connect/send API 不感知加密，同时不会牺牲身份认证。C ABI 使用回调，C++/Go 包装成类型安全的配置对象。

### 安全状态迁移

```text
PLAIN(epoch 0)
  -> server UPGRADE_PROPOSE(epoch 1, handshake)
  -> client UPGRADE_READY(epoch 1)
  -> server SWITCH(epoch 1)
  -> client SWITCH_ACK(epoch 1)
  -> ENCRYPTED(epoch 1)

ENCRYPTED(epoch N)
  -> authenticated REKEY_PROPOSE(epoch N+1)
  -> REKEY_READY / SWITCH / SWITCH_ACK
  -> ENCRYPTED(epoch N+1)
```

业务发送在迁移屏障期间进入现有有界发送队列；完成后自动按新 epoch 发送。超时使 session 关闭并产生带错误码的 `SESSION_CLOSED`，不静默回滚。

### 注释范围

- Rust：所有公开 struct/enum/trait/方法增加 rustdoc，说明参数语义、状态前置条件、错误、线程安全和兼容入口；内部复杂状态机只注释不变量与迁移原因。
- C：所有公开枚举、POD、回调和函数增加稳定契约注释，包含所有权、生命周期、空指针、线程和错误语义。
- C++11：公开类和方法增加 Doxygen 风格说明，明确 RAII、异常、线程安全与旧接口关系。
- Go：所有 exported identifier 增加符合 golint/godoc 的注释，并提供 package comment。
- 禁止逐行复述代码，也不在注释中引用易失效的文件行号。

### 使用文档

- README 增加 5 分钟快速开始与文档索引。
- `docs/getting-started.md` 覆盖：选择 TCP/UDP/KCP、服务端策略、客户端统一连接、认证事件、收发、动态升级/rekey、指标、关闭顺序和常见错误。
- 增加可编译的 C 服务端/客户端、C++11 服务端/客户端和 Go 服务端/客户端示例；CI/Makefile 实际构建运行示例，避免文档漂移。

## 兼容与版本策略

- `RNET_ABI_VERSION` 仍保持 1，使用带 `struct_size` 的新 POD 和新符号扩展 ABI；不修改现有 POD 布局。
- 旧 secure/plaintext 分离入口保留一个兼容周期，并在 C++/Go 注释中标记 deprecated；新文档只展示统一入口。
- 新统一入口使用带版本的连接前言；旧入口仍走现有 wire v1，避免已有程序突然无法互通。

## 任务拆解

### 准备工作

- [x] 确认动态安全策略和客户端信任模型两个待定决策。
- [x] 冻结现有 23 个 C ABI 符号、SDK 编译和 wire v1 行为基线。
- [x] 为统一 API、策略协商增加 RED 测试；注释/示例完整性测试待补。

### 实现任务

- [x] 实现版本化连接前言和控制帧 codec，覆盖截断、超长、未知类型/epoch。
- [x] 实现统一安全 session 状态机和 TCP 有序迁移。
- [x] 实现 UDP 控制重传、KCP 可靠有序承载、epoch 和重放保护迁移。
- [x] 接入服务端策略、runtime 级客户端身份/验证器和统一 connect/listen。
- [x] 新增 C ABI，并保留旧符号与旧 wire 行为。
- [x] 更新 C++11/Go/Rust 易用接口与兼容入口，接口名不暴露传输类型。
- [ ] 补齐 Rust/C/C++11/Go 公共接口与状态机契约注释。
- [x] 编写 getting-started，并将 C++11/Go 可编译示例迁移到统一接口。
- [ ] 更新打包脚本、结构/导出检查和 CI 验证。

### 验收

- [ ] 业务 send/poll 接口不接收加密参数，明文与加密回显使用同一套客户端代码。
- [ ] 服务端 required 模式不可降级，错误 key、篡改控制帧、迁移超时均失败关闭。
- [ ] TCP/UDP/KCP 的升级、rekey、乱序、重放和队列背压测试通过。
- [ ] 旧 23 个导出符号及现有 C++11/Go 调用继续通过。
- [ ] rustdoc、C 头契约、Go doc、示例和最终 dist 独立链接通过。
- [ ] fmt、Clippy、Rust 全量测试、C++11、Go race 和对抗性审查通过。

## 已确认决策

1. **中途改变的范围**：允许 `明文 -> 加密`、`加密 -> 明文` 和加密状态下 rekey，全部由服务端发起；客户端自动跟随。降级提议必须经过当前加密状态认证，降级完成后的数据不再受加密保护。
2. **客户端首次信任**：使用 runtime 级 `peer_verifier` 校验服务端公钥，connect/send 不暴露加密选择，不默认采用 TOFU。
3. **传输选择接口**：服务器和客户端只使用通用 `listen/connect` 名称，TCP/UDP/KCP 由创建配置中的枚举决定；发送接口不再携带传输类型。
