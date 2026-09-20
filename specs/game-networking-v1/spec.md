# RNet Game Networking v1 规格说明

## 需求概述

将 RNet 从通用安全传输库演进为面向在线游戏的客户端/服务器网络库。保留现有传输、加密、资源预算、C ABI 和多语言 SDK 作为稳定底座，在其上提供服务端权威的游戏会话、协议版本协商、心跳与时钟同步、网络质量、断线恢复、帧号语义和实时消息背压策略。TCP、UDP、KCP 只在监听或连接配置中选择并在会话生命周期内保持不变；普通发送和接收接口不暴露 `stream_id`、`msg_type` 或 transport，客户端也不感知当前业务数据是否加密。

首个版本面向“中心服务器权威”的实时或会话型游戏，包括动作、FPS、竞技、MMO 和房间制游戏。P2P NAT 穿透、语音/视频、AOI、状态同步算法、预测回滚、匹配和房间业务不属于网络库职责；库提供实现这些系统所需的传输与会话原语。

## 当前实施状态

- 已落地 Rust `rnet-game` 的无 `msg_type`/`stream_id` 默认发送与接收、TCP/UDP/KCP 同名接入、域名连接、生命周期与底层指标入口、服务端加密切换和密钥轮换，以及加入阶段的**精确协议 ID/版本门禁**。业务收到原始 ticket，构建号和能力位是认证后的应用元数据。
- 已实现经 Noise 保护的自动心跳、匹配挑战与超时关闭，以及每会话 RTT/平滑 RTT/抖动快照；心跳依赖应用持续 `poll`，测得延迟包含本地待发送时间。时钟同步、丢包与质量等级事件尚未完成。
- 本轮观测切片：在游戏层暴露无玩家标签的累计心跳计数（探测/回执发送、发送失败、匹配/拒绝回执、限流、超时）和 RTT P50/P90/P95/P99/P99.9 快照，并追加到 Prometheus 输出；另提供独立的 RTT 窗口轮转快照，供短时异常排查。只有匹配且未超时的认证回执进入 RTT 分布；空样本为零，计数不因会话关闭清空，窗口轮转不清累计分布。该切片不声称具备丢包率或链路质量等级。
- 版本范围协商与服务端签名/认证的“选定版本”回复、恢复票据、实时淘汰队列、游戏层 C/C++11/Go API 尚未完成。未实现的其他游戏控制 kind 目前按协议错误拒绝，而不是静默忽略。
- KCP 旧崩溃样本已回归；无 sanitizer 的 15 秒覆盖引导 fuzz 完成约 168 万次。新增 game envelope/join fuzz 入口，并完成 game-wire 与 FFI-config 各 10 秒短时 fuzz。当前系统 Rust 工具链缺 ASan 运行库，ASan fuzz 与目标环境长稳压测仍未完成。

## 行为规格

### 正常流程

- 当游戏服务器启动监听时，系统应由 `GameServerConfig.transport` 一次性确定 TCP、UDP 或 KCP，并由 `GameProfile` 给出适合实时、可靠会话或回合制游戏的安全默认值。
- 当客户端加入时，系统应在现有 Noise 握手内协商应用协议 ID、协议版本、客户端构建版本和能力位；版本不兼容必须在创建游戏会话前被拒绝。
- 当服务器验证加入票据后，双方才应收到 `GameSessionReady`；应用不得观察到“传输已连接但认证未完成”的伪游戏会话。
- 当应用发送普通游戏消息时，只需提供 session 和 payload；接收事件也只提供 session、payload 和库自动维护的可选元数据。业务若需要区分 Protobuf、FlatBuffers 或自定义命令，应在自己的 payload 协议中完成，不由网络库强制要求 `msg_type`。
- 当应用发送输入、快照等实时消息时，可指定逻辑 tick 和 `LatestOnly` 过期键；尚未写入 socket 的同键旧消息应被新消息替换，并记录丢弃原因，不能无限积压旧状态。
- 当可靠队列达到预算时，发送应立即返回 `WouldBlock`；库不得静默丢弃可靠消息。实时可丢弃消息应按配置淘汰并产生 counter。
- 当会话存活时，库应自动执行心跳、单调时钟偏移采样和网络质量估计，并提供 RTT、抖动、丢包/重传、排队延迟和质量等级事件。
- 当传输短暂中断且策略允许恢复时，客户端应使用服务端签发的短期恢复票据重连；服务端决定是否恢复原逻辑游戏会话，客户端不能自行声明身份或回退安全策略。
- 当服务器在已连接会话上关闭业务加密时，后续业务 frame 应真实以明文传输；当服务器重新开启时，后续业务 frame 应恢复加密。游戏层会话与发送接口保持不变，客户端没有加密开关并自动跟随经过认证的服务端控制消息。
- 当游戏服务查询日志和指标时，应能按 endpoint/session/transport、协议版本、关闭原因、消息类别和时间窗口追溯握手、排队、RTT、抖动、恢复及丢弃行为。

### 传输能力约束

- TCP 提供可靠有序传输，适合登录、回合制和对实时头阻塞不敏感的业务。
- KCP 提供可靠有序低延迟传输，作为大多数需要可靠游戏消息的默认推荐。
- UDP 提供不可靠数据报，适合应用可容忍丢失并自行使用 tick/序号淘汰旧状态的实时消息。
- v1 的一个游戏 session 只绑定一个 transport。库不得在每次发送时切换 transport，也不伪装 UDP 为可靠传输。
- 若调用方请求当前 transport 无法保证的可靠性，配置或发送必须明确返回 `NotSupported`，不能静默降低语义。

### 边界和异常

- tick 和网络序号使用明确的环绕比较；不能用可能溢出的普通有符号减法比较外部输入。
- 心跳、时钟同步、恢复和安全控制使用消息体前缀中仅供库内部识别的 envelope kind；该字段不进入业务发送/接收接口。应用消息的业务类型由 protobuf、FlatBuffers 或调用方自己的 payload schema 表达。
- 即使业务数据处于明文模式，安全模式切换控制仍必须经过认证和加密，否则网络攻击者可以伪造“关闭加密”命令。明文业务数据本身明确不提供机密性或完整性。
- 加入票据、恢复票据、私钥、完整 payload 不得写入日志；可记录长度、摘要标识或服务端提供的脱敏账号标签。
- 恢复票据必须有过期时间、服务端认证和单次/代次约束；旧票据不得使已踢下线或已迁移的玩家重新接管会话。
- 心跳、质量采样不能绕过 runtime/endpoint/session 的字节与速率预算，也不能在 I/O worker 上执行用户回调。
- UDP/KCP 未通过 cookie 与 Noise 验证前不得创建长期游戏会话、恢复状态或大对象。
- 慢客户端只能影响自己的会话；公平批处理和 per-session 预算应防止一个玩家占满 endpoint。
- C、C++11、Go 新接口使用带 `struct_size` 的追加版本；已有 ABI 符号和结构布局保持不变。
- 未在目标硬件完成容量、弱网和 24 小时稳定性验证前，只能标记为生产候选，不能宣称已完成该部署环境的生产认证。

## 技术设计

### 模块划分

- **`rnet-game` facade**：提供 `GameRuntime`、`GameServer`、`GameClient`、`GameSession`、配置预设和面向游戏的事件；组合而非复制底层 runtime。
- **会话协议**：在业务 frame 内部封装不向调用方暴露的 game envelope，负责区分应用数据与协议协商、tick、心跳、恢复代次等控制消息。
- **实时发送调度**：可靠 FIFO 与 `LatestOnly` 可淘汰队列分开计费；按优先级公平出队，但不承诺 transport 本身不具备的交付语义。
- **网络质量**：使用单调时间计算 RTT、抖动、排队延迟、KCP 重传和 UDP 序号缺口；按阈值抑制频繁质量事件。
- **恢复管理**：服务端签发/验证短期认证票据，应用回调决定逻辑玩家状态能否接回；网络库不持有玩法状态。
- **安全与准入**：复用 Noise、cookie、rekey、IP/prefix 限流和资源预算，并为游戏控制报文增加严格长度、速率和状态机验证。
- **SDK/ABI**：Rust 先建立类型化接口；C v6、C++11、Go 暴露等价的配置、发送、事件和网络质量快照。
- **可观测性**：在现有 metrics/logger 上增加游戏会话阶段、协议拒绝、心跳超时、恢复结果、实时淘汰、tick 延迟和质量等级。

### 关键接口

```rust
pub enum GameProfile {
    Realtime,
    ReliableRealtime,
    Session,
}

pub struct GameProtocol {
    pub protocol_id: u64,
    pub min_version: u32,
    pub max_version: u32,
    pub build_id: u64,
}

pub struct GameRuntimeConfig {
    pub network: RuntimeConfig,
    pub heartbeat_interval: Duration,
    pub heartbeat_timeout: Duration,
    pub quality_sample_interval: Duration,
    pub reconnect_window: Duration,
}

pub struct GameServerConfig {
    pub transport: Transport,
    pub bind_addr: SocketAddr,
    pub profile: GameProfile,
    pub protocol: GameProtocol,
    pub local_key: Keypair,
    pub initial_encryption: bool,
}

pub struct GameClientConfig {
    pub transport: Transport,
    pub server: ServerAddress,
    pub profile: GameProfile,
    pub protocol: GameProtocol,
    pub join_ticket: Vec<u8>,
    pub resume_ticket: Option<Vec<u8>>,
}

pub struct GameSendOptions {
    pub tick: Option<u32>,
    pub priority: MessagePriority,
    pub expiry: ExpiryPolicy,
    pub correlation_id: u64,
}

impl GameRuntime {
    pub fn listen(&self, config: GameServerConfig) -> Result<GameEndpoint>;
    pub fn connect(&self, config: GameClientConfig) -> Result<GameEndpoint>;
    pub fn send(&self, session: GameSession, payload: &[u8]) -> Result<()>;
    pub fn send_with_options(
        &self,
        session: GameSession,
        payload: &[u8],
        options: GameSendOptions,
    ) -> Result<()>;
    /// Server-only operation. Clients follow the authenticated transition automatically.
    pub fn set_encryption(&self, session: GameSession, enabled: bool) -> Result<()>;
    pub fn network_quality(&self, session: GameSession) -> Result<NetworkQuality>;
}
```

普通接口不要求 `msg_type`、tick、priority、expiry、correlation ID。`stream_id` 不出现在游戏层接口；底层兼容 frame 中需要的内部字段始终由库填写。接收侧的普通 `GameMessage` 只包含 session、payload 和可选 tick/correlation 元数据，不返回 `msg_type` 或 transport。

### 游戏消息体布局

```text
kind:u8 | flags:u8 | header_len:u16 | sequence:u32 | tick:u32 | extension:[u8] | payload:[u8]
```

- `kind=0` 表示应用数据；`payload` 原样保存业务 protobuf/FlatBuffers/自定义协议，网络库不解析业务包类型。
- 其他 `kind` 只供网络库的心跳、时钟同步、恢复和协议控制使用，不会作为应用消息上抛。
- `flags` 决定 sequence/tick 等内部字段是否有效；`header_len` 允许未来扩展并使旧版本安全跳过未知字段。
- 底层兼容 frame 的 `msg_type` 和 `stream_id` 固定写为保留值 `0`，不得根据业务内容填写，也不得进入游戏层 API。
- 解析器先验证最小头、`header_len`、总长度、保留位及 kind，再访问任何可选字段；未知控制 kind 返回协议错误，不能透传为业务数据。

### 状态与不变量

```text
Connecting -> Handshaking -> AwaitingAuthorization -> Ready
     |              |                 |                 |
     +--------------+-----------------+-----------------+-> Closed
                                                       |
                                                       +-> Reconnecting -> Ready/Closed
```

- 只有 `Ready` 会话可以收发业务消息。
- session 的 transport 在 endpoint 创建时固定；任何发送选项都不能改变 TCP、UDP 或 KCP 路由。
- 只有服务端会话可以发起加密模式切换；每次切换使用递增安全 epoch，双方在同一 frame 边界提交，旧 epoch 数据不能在切换后重新生效。
- 一个恢复代次至多对应一个当前 transport session；新代次接管后旧代次的数据全部拒绝。
- 每次队列预留必须恰好释放一次；替换 `LatestOnly` 消息时先完成新预留，再释放旧消息，避免预算短暂失真。
- 网络质量统计只使用已认证报文和本地单调时钟，不能信任客户端上报的绝对时间。
- 心跳探测与回执使用同一 64 位随机挑战，服务端只接受当前待决挑战的回执；重复、过期或不匹配回执不得更新 RTT 或存活时间。RTT 取本地单调时钟差，抖动取相邻 RTT 差的有界平滑值。
- 心跳控制必须走始终受 Noise 认证的控制通道；业务明文模式下不得把普通 `Data` 消息当作可信存活证明。探测超时只在已建立的游戏会话上生效，且每会话最多存在一个待决挑战。

### 兼容与迁移

- `rnet-transport`、现有 C ABI 和 SDK 继续可用，避免破坏已有接入。
- README 主入口改为游戏网络库；通用底层接口移动到“Advanced transport API”。
- 新项目默认使用 `GameRuntimeConfig::production()`；旧 `RuntimeConfig::default()` 的兼容语义不作为游戏层默认值传播。
- wire v1 外层 frame 不修改；其 `msg_type=0`、`stream_id=0`，game envelope 放入消息体，便于灰度和版本协商且不破坏已有 ABI。
- 当前阶段使用精确版本门禁；客户端和服务端必须配置相同非零协议 ID/版本。真正的版本范围协商不能只取两端范围交集，必须在受认证的服务端响应中确认最终版本，因此保留为后续任务。

### 纯度边界

- **纯逻辑**：协议兼容判断、tick 环绕比较、质量等级、淘汰选择、恢复代次验证、错误/指标分类。
- **副作用**：socket、DNS、Noise/KCP、单调时钟、票据密钥、事件队列、日志 callback 和 FFI buffer。

## 任务拆解

### 准备工作

- [ ] 完成当前 KCP 恶意报文 fuzz 修复及回归，避免把已知崩溃入口带入游戏层。
- [ ] 补齐连接/握手、UDP/KCP 状态机、安全切换、资源预算、FFI 生命周期和观测关键路径注释。
- [ ] 固化现有 workspace、C++11、Go、ABI、fuzz 与短时负载基线。

### 实现任务

- [ ] 1. 新建 `rnet-game` crate，定义游戏配置、句柄、事件、profile 与 transport 能力矩阵。
- [ ] 2. 以测试先行实现消息体前缀形式的 game envelope、保留 kind、协议/构建版本协商和严格解析；外层 `msg_type/stream_id` 恒为零。
- [ ] 3. 实现服务端权威授权与 `GameSessionReady` 状态机，映射现有安全会话且不泄漏半连接。
- [ ] 4. 实现无 `stream_id`、`msg_type` 和 transport 参数的默认发送/接收接口，以及可选 tick/correlation 高级接口。
- [ ] 5. 实现可靠 FIFO、`LatestOnly` 替换、优先级公平调度、字节预算与分原因丢弃指标。
- [ ] 6. 实现心跳、超时、时钟偏移采样、RTT/抖动/丢包质量快照和抑制后的质量事件。
  - [x] 认证心跳、匹配挑战、超时关闭、每会话 RTT/平滑 RTT/抖动快照；三种传输及明文业务模式测试。
  - [ ] 时钟偏移、丢包/重传质量、等级事件和长时间弱网验证。
- [ ] 7. 实现服务端签发的恢复票据、恢复代次和应用授权回调，覆盖重放、过期及双连接竞争。
- [ ] 8. 将游戏维度接入结构化日志、百分位延迟、Prometheus 文本和关闭原因。
  - [x] 累计心跳计数、认证 RTT 累计/窗口分位数进入 Rust 快照，累计值进入无玩家标签的 Prometheus 文本。
  - [ ] 游戏加入/恢复/实时丢弃的结构化日志与关闭原因仍待补齐。
- [ ] 9. 新增 C ABI v6，并同步 C++11 与 Go 游戏 facade；普通路径保持参数最少。
- [ ] 10. 编写 Rust/C/C++11/Go 快速接入示例、传输选型、弱网调优和服务端 tick 集成文档。
- [ ] 11. 增加解析器/状态机 fuzz、确定性丢包乱序、慢客户端、恢复风暴、长时间 tick 环绕测试。
- [ ] 12. 执行对抗性代码审查，修复高置信安全、性能、可用性和测试盲区。

### 验收

- [ ] Rust、C ABI、C++11、Go 全量测试、Clippy、格式、依赖审计和 ABI 导出检查通过。
- [ ] TCP/UDP/KCP 的客户端与服务端示例均使用同名 `listen/connect/send/poll` 接口，仅配置中的 transport 不同。
- [ ] 普通游戏接入不传 `stream_id`、`msg_type`、transport、安全模式、tick、priority 或 correlation ID。
- [ ] 服务端可在连接存续期间关闭或开启业务加密；抓包验证明文阶段业务 payload 可见、加密阶段不可见，客户端业务接口和 session 句柄不变化。
- [ ] 可靠消息不静默丢失；实时旧消息可控淘汰且每次淘汰可观测。
- [ ] 协议不兼容、认证失败、心跳超时、恢复成功/失败和安全切换均有稳定事件及结构化原因。
- [ ] 恶意控制报文、序号环绕、恢复票据重放和队列过载不能导致 panic、越界增长或会话串线。
- [ ] 目标硬件弱网矩阵和至少 24 小时 soak 报告满足项目设定的 CPU、RSS、吞吐、错误率及 P99/P99.9 SLO。

## 待确认决策

1. 默认目标采用“中心服务器权威”的实时在线游戏；不在 v1 内实现 P2P。
2. 一个游戏 session 在 v1 只使用一个 transport；多路径 TCP+UDP 聚合留作后续版本。
3. 游戏层采用新增 facade 并保留 RNet 名称和已有 ABI，不进行破坏性全仓库改名。
4. KCP 作为可靠实时 profile 的默认推荐；UDP 不承诺可靠，TCP 不承诺消除队头阻塞。
