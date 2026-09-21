# 对抗性审查报告

本文按时间保留各轮审查结论。前面的 Phase 1/Phase 2 段落是历史快照，后续实现已替代其中关于工具链、KCP 支持状态和固定 peer 上限的描述；当前结论以最后的 Production v1 复审为准。

## 发现的问题

| # | 严重度 | 位置 | 问题描述 | 修复结果 |
|---|---|---|---|---|
| 1 | 高 | `rnet-ffi::borrowed_slice` | 安全 Rust 函数可从原始指针构造任意生命周期的 slice，抽象本身不健全。 | 改为 `unsafe` 且不得泄漏引用的闭包边界；带指针的公开 Rust ABI 入口全部标记 `unsafe extern "C"`，并启用 `unsafe_op_in_unsafe_fn` 拒绝隐式 unsafe。 |
| 2 | 高 | `rnet_runtime_destroy` / `rnet_poll_events` | destroy 可与已取得 runtime `Arc` 的 poll 竞态，poll 可在句柄销毁后借出无法释放的 buffer。 | 增加在全局句柄锁内获取的 active-call lease；有活动调用时 destroy 返回 `WOULD_BLOCK`，并新增真实并发回归。 |
| 3 | 高 | `NetworkRuntime::open_endpoint` | endpoint-open 事件在队列满时被静默丢弃，调用方无法建立完整生命周期。 | 打开端点改为显式返回 `WOULD_BLOCK`，并回滚尚未启动的 endpoint 记录。 |
| 4 | 高 | UDP 新 peer 接收路径 | session-open 入队失败后仍保留 peer 映射，后续可能先交付 message 再交付 open。 | 只在 session-open 成功入队后提交 peer 映射；否则回滚 session 并丢弃数据报，下一数据报重试建会话。 |
| 5 | 中 | runtime stop 事件 | 队列满时 runtime-stopped 被静默丢弃。 | stop 完成关闭后返回 `WOULD_BLOCK`；调用方 poll 后重试 stop，只补发一次 stopped 事件。 |
| 6 | 中 | `go/rnet` cgo 链接布局 | Go wrapper 指向源码 `target/debug`，打包后无法独立链接；首版还把含 Go 指针的 C POD 直接传入 cgo。 | 改为发布包相对 `lib/librnet.a`，并用 C inline shim 在 C 栈上组装 POD；已从独立 `dist` 目录完成静态链接 echo。 |

## 测试盲区与后续阶段

- 当时仅提供了 `cargo-fuzz` 入口，未进行长时 fuzz campaign；Production v1 阶段已经安装工具并完成全部 target 编译验证。
- 尚未进行 `tc netem` 丢包/乱序/抖动矩阵、24–72 小时 soak、内存故障注入、Miri/loom 或 sanitizer 运行。
- 当时的 TencentOS Rust 工具链不包含 rustfmt/clippy；当前 Production v1 环境已实际通过 fmt 与严格 Clippy。
- KCP、cookie/session 防伪造、每 IP 准入限制和加密在当时属于 Phase 2；这些能力现已实现，只有旧的不安全兼容 KCP 入口仍显式返回 `NOT_SUPPORTED`。

## 整体评价

审查发现的高置信并发、FFI 和打包问题已修复并加入回归。Phase 1 在 Linux x86_64 的 Rust、C++ 和 Go/cgo 功能链路可交付，但不应将这一结论扩展为 KCP/安全或固定性能承诺。

## Phase 2 对抗性复审

| # | 严重度 | 位置 | 问题描述 | 修复结果 |
|---|---|---|---|---|
| 1 | 高 | 安全 UDP/KCP peer 状态表 | 未完成握手或未处理认证的 peer 可长期占用状态；KCP 对未知源创建的 engine 也可能无界增长。 | 每 endpoint 最多保留 1024 个安全数据报 peer/KCP engine；握手与应用认证均受 5 秒 deadline 约束，超时回收 session。 |
| 2 | 高 | UDP/KCP 已建立数据处理 | 处理前从 peer map 移出状态，伪造密文或非法业务帧触发错误时没有放回，导致一个坏包永久破坏合法会话并泄漏 session route。 | 解密或帧校验失败时恢复原 transport/session 状态；认证阶段错误则显式产生 `JOIN_FAILED` 并回收 session。 |
| 3 | 中 | cookie 标签验证 | 首版截断 HMAC 并用手写比较循环，无法可靠保证编译后的常量时间性质。 | cookie 改为完整 32 字节 HMAC-SHA256 标签，并使用 `ring::hmac::verify`；仍绑定源地址和当前/上一时间桶。 |
| 4 | 中 | client join payload | FFI 可先复制任意长度 join payload，异步握手阶段才失败，且 UDP 无法承载过大的握手记录。 | endpoint 打开前统一限制 join payload 为 `min(max_body_len, 60 KiB)`，超限返回 `MESSAGE_TOO_LARGE`。 |
| 5 | 中 | Go/cgo 测试产物 | 手工测试曾复制旧 `librnet.a`，导致新 UDP/KCP ABI 被误报为 `NOT_SUPPORTED`。 | 固定先 `cargo build -p rnet-ffi` 再复制静态库；`make go-test` 已遵循该顺序，回归使用 `-count=1` 验证。 |

### 仍需外部环境验证

- 当前已做 KCP 核心丢包/乱序恢复测试，但尚未执行宽参数 `tc netem` 矩阵和 24–72 小时 soak。
- 未对 `snow`、`kcp` 依赖或完整集成做独立第三方安全审计；不能把“测试通过”等同于安全认证。
- 1024 peer 是固定安全上限而非自适应每 IP 限流；公网部署仍需防火墙/负载均衡层限速。
- KCP 当前按远端 UDP 4-tuple 隔离并使用固定 conv；同一 4-tuple 多逻辑连接需要后续协商 conv 扩展。

### Phase 2 评价

复审发现的高置信资源耗尽和单包会话破坏问题已修复并加入相关边界测试。Linux x86_64 功能交付成立；长期弱网、DDoS 容量和第三方密码审计仍是生产准入项。

## API v2 与延迟可观测性复审

| # | 严重度 | 位置 | 问题描述 | 修复结果 |
|---|---|---|---|---|
| 1 | 高 | TCP session 读/写任务 | `rnet_session_close` 已发布关闭事件后，后台任务仍可能在 select 竞态下发布旧 session 的 message/writable。 | 所有 TCP 消息和 writable 入队前重新验证 session；阻塞等待事件队列期间也重复验证，新增跨端发送竞态回归。 |
| 2 | 中 | 延迟直方图 | 固定桶返回桶上界，非二次幂样本可能出现 P99 大于精确 max，违反公开快照不变量。 | 所有分位值以精确 max 封顶，测试改为直接断言 `P99 <= max`。 |
| 3 | 中 | Go `Runtime.Poll` | 容量小于等于零时提前返回，既接受负容量，也会隐藏已关闭 runtime 的失效句柄。 | 负容量返回 `INVALID_ARGUMENT`；零容量仍调用 `poll_ex` 校验句柄，并增加回归。 |
| 4 | 中 | FFI logger callback | callback panic 被异步 logger 捕获，但线程局部的 callback 标记会因 unwind 跳过复位，污染该日志线程后续状态。 | 使用 RAII guard 在正常返回和 unwind 时统一复位。 |
| 5 | 中 | UDP peer route | 单 session 关闭后 peer map 仍指向已删除 handle，下一数据报可能沿用陈旧 session。 | 收包时校验 session table，删除陈旧映射并按正常顺序建立新 session；测试覆盖 open-before-message。 |

本轮还核验了 C ABI 增量兼容、C++11 最终链接、Go/cgo、KCP ACK RTT、KCP update delay、UDP/KCP 握手/认证计时和异步 logger callback 计时。公开 API 的功能链路已覆盖；长期弱网、soak、sanitizer/Miri/loom 与第三方密码审计仍属于外部生产准入工作。

## 透明安全与通用接口复审

| # | 严重度 | 位置 | 问题描述 | 修复结果 |
|---|---|---|---|---|
| 1 | 高 | 自适应握手 | `ServerHello` 中的初始安全模式原本未绑定到加密认证结果，攻击者可能篡改客户端看到的模式。 | 服务端在 Noise 保护的 `AuthDecision` 中再次携带权威模式；客户端要求其与前言一致，否则按协议错误关闭。 |
| 2 | 高 | 自适应 KCP | 首版把 KCP 枚举路由到了原始 UDP 收发，接口虽显示 KCP，实际不具备可靠有序语义。 | 增加独立 record wire 层，每个 peer 使用 `RustKcpEngine`，定时 update 并记录 KCP RTT/update delay。 |
| 3 | 高 | UDP 安全迁移 | 握手和控制帧无重传；安全屏障期间从发送队列取出的业务消息会被丢弃。 | 控制密文按原字节重传并缓存重复请求响应；迁移期间业务消息进入有界 deferred 队列，屏障完成后按序发送。 |
| 4 | 高 | UDP 错误恢复 | 处理前移出 peer 状态，非法密文或 epoch 可导致合法已建立会话状态永久消失。 | 已建立状态在处理失败时原样放回；客户端握手错误产生 `JOIN_FAILED` 并回收 route。 |
| 5 | 高 | UDP/KCP 预认证资源 | Cookie 前的重复响应缓存和 KCP engine 可按伪造源地址增长，绕过原 cookie 防分配目标。 | engine、pending、duplicate cache 和已验证 peer 均限制为每 endpoint 1024；CookieChallenge 不创建待确认请求。 |
| 6 | 中 | UDP 终止确认 | AuthDecision 和最终切换 ACK 没有明确终点，空闲连接可能永久重传或无法发现迁移超时。 | 增加受 Noise 保护的 datagram `AuthAck`；最终 ACK 作为可缓存终止响应，不再等待 ACK-of-ACK；真正超时会关闭 session。 |
| 7 | 中 | C ABI 导出检查 | 头文件符号提取正则不接受函数名中的数字，因此新增 `_v2` 声明不会进入 ABI 一致性检查。 | 提取器允许数字并将全部新旧符号纳入 header/shared-library diff。 |

### 本轮测试盲区

- 已有确定性的 KCP 丢包/乱序恢复、UDP 控制重传/重复响应缓存和三传输状态迁移测试；尚未执行真实网络上的宽参数 `tc netem` 长时间矩阵。
- runtime verifier 的精确 key pin 经 C ABI 集成覆盖；任意 C 回调的线程安全仍由调用方负责，需在业务绑定层再做压力测试。
- 动态降级按需求允许，但降级后的业务记录不具备机密性与完整性；生产策略应限制哪些 session 可以执行该操作。

### 透明安全复审评价

高置信的降级篡改、伪 KCP、状态丢失、无界预认证缓存和迁移期丢包问题均已修复。统一接口与兼容接口通过 Rust、C ABI、C++11 和 Go 最终链接回归；外部弱网 soak 与独立密码协议审计仍是生产准入条件。

## Production v1 对抗性复审

| # | 严重度 | 位置 | 问题描述 | 修复结果 |
|---|---|---|---|---|
| 1 | 高 | `rnet_latency_snapshot_v2` | 两段式 SDK 调用先查询容量时就旋转窗口，第二次读取会得到空窗口。 | 容量查询不再产生状态变更；C++/Go drain 路径新增真实发送样本回归。 |
| 2 | 高 | FFI logger shutdown | stop 持有 logger mutex 等待 dispatch 线程；回调查询 metrics 会反向等待同一 mutex，递归 stop 也可能自等待。 | stop 先从 mutex 中取出 logger 再 join；回调内 stop 明确返回 `INVALID_STATE`；增加回调重入和超时回归。 |
| 3 | 高 | KCP 发送队列 | 应用字节预算在交给 KCP 后释放，但 KCP 未确认/重传队列没有独立硬上限，弱网下 RSS 可持续增长。 | KCP engine 增加会话级未确认字节上限，DatagramWire 增加 runtime 总上限；满载转为有界 deferred 背压，跨 peer 临时发包改为逐 peer 释放。 |
| 4 | 高 | DNS 与多地址连接 | 阻塞解析线程和候选地址数没有全局上限；TCP 候选逐个使用完整 timeout，最坏连接时长随候选数线性放大。 | DNS 解析移到受 64 并发许可保护的专用线程并限制为 32 个去重候选；TCP 使用总截止时间，并将单次尝试限制在剩余预算内。 |
| 5 | 中 | KCP 发送队列延迟 | KCP 背压后的 deferred 消息在每次失败重试前都记录延迟，导致样本数大于成功发送数并扭曲尾延迟。 | 仅在 record 被 wire 成功接收后采样；新增小预算背压集成测试，精确断言每条已接受消息只有一个样本。 |
| 6 | 高 | UDP/KCP 多地址连接 | 数据报客户端只复用首个地址族的 socket，IPv4/IPv6 候选切换会失败；候选重试也没有统一的整体截止时间。 | 地址族变化时重绑 wildcard socket并原子更新 endpoint/session 路由；所有候选共享一个连接 deadline，增加 IPv4 黑洞切换 IPv6 和总超时回归。 |
| 7 | 高 | runtime 资源准入 | 只有单 listener 的 session 限制，没有 runtime 全局 endpoint 和 pending-handshake 上限，多个 listener 可绕过局部预算。 | 配置 v5 增加两个全局上限；插入、建立、关闭、失败和 endpoint 回收路径统一预留/释放，指标 v3 暴露当前值、峰值与八类拒绝原因。 |
| 8 | 中 | deadline 配置与停止 | `Duration::MAX` 可让 runtime 建立、stop 或事件队列超时接口在构造 `Instant` 时 panic。 | 配置和 stop 在改变状态前用 `checked_add` 拒绝不可表示的 deadline；`EventQueue::push_timeout` 同样返回 `INVALID_ARGUMENT`，均有 panic 回归。 |
| 9 | 中 | 主动关闭指标 | `close_session` 发布了带原因的事件，但没有更新 `session_closed_by_reason`，导致事件与累计指标不一致。 | 所有主动 session/endpoint 关闭路径统一记录稳定错误码；FFI 指标 v3 回归验证 `CANCELLED` 计数。 |
| 10 | 高 | 事件队列构造 | 条目上限直接传给 `VecDeque::with_capacity`，错误的大容量配置会在尚无流量时触发巨额预分配甚至进程 abort。 | 改成按实际事件惰性增长，条目数和字节数逻辑上限保持不变；内部测试验证构造时 backing capacity 为零。 |

### 验证与剩余门禁

- `cargo-audit 0.22.2` 扫描 82 个锁定依赖，无漏洞或 warning。
- `cargo-deny 0.20.2` 的 licenses/bans/sources 通过；`getrandom`、`syn`、`windows-sys` 的上游双版本保留为可见 warning。
- 5 个 fuzz target 已编译，CI 执行短时 smoke；本地 TCP 短时 probe 以及 UDP/KCP 各 10,000 条加密消息 probe 验证了 CPU、RSS、吞吐、队列指标和 P99.9 报告链路，但不作为容量结论。
- UDP/KCP 域名连接现在会在候选地址族变化时重绑客户端 socket，并与 TCP 一样受单个整体连接 deadline 限制；IPv4 黑洞切换 IPv6 的 UDP/KCP 集成测试已覆盖该行为。
- 事件队列已验证在认证事件背压时回滚新建 session 和 pending-handshake 预算；关闭、失败、超时与 endpoint 回收均纳入资源计数回归。
- 目标环境的三传输 24 小时 soak、真实 `tc netem` 矩阵、独立密码协议/依赖审计仍未完成，因此不能把仓库门禁通过表述为外部生产认证。

## 游戏 wire-v4 与实时待发队列复审

- 版本范围连接、协商状态存储、受保护控制状态机和测试已拆为独立模块；结构门禁限制游戏层单文件不超过 800 行，避免协议、I/O 与测试重新堆入巨型文件。
- SELECT/ACK 丢失会按固定间隔重试，协商超过截止时间关闭会话；纯状态机测试覆盖重试抑制、再次重试、等待 SELECT 超时和 ready 会话不误关闭。
- TCP/KCP `LatestOnly` 只在 I/O worker 取槽前允许替换；pickup、替换及入队失败原因均有低基数累计指标。pickup 不代表送达，之后的 I/O 失败仍以会话关闭原因追踪。
- 尚未执行目标网络的 `tc netem` 丢包/延迟/乱序矩阵和 24 小时以上 soak；这些外部资格测试仍是生产认证边界。
