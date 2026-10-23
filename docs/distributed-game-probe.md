# 独立游戏网络探针（Linux）

`game_probe` 将服务端和负载发生器放在不同进程，可部署到不同机器；不改变 SDK 的 wire、ABI 或发送接口。当前验证平台是 Linux x86_64。发布包的 `bin/game_probe` 可直接运行，不要求目标机安装 Rust。源码开发可运行 `cargo build --locked -p rnet-game --bin game_probe --release`。

本地测试覆盖独立进程、真实 socket 和三种传输；**不是目标环境容量、弱网或 24–72 小时长稳结论**。原 `game_soak` 保留为回环探针。先冻结功能，再制定目标环境验收。

## 准备凭据与启动

所有参数采用 `--name value`，重复、未知参数报错；`--help` 查看摘要。负载启动至少需要 19 GiB MemAvailable，运行中每约 100 ms 检查 15 GiB 保留量。预算限制不是全进程 RSS 保证；不要同时启动其他重负载任务。

在可信机器上创建一个尚不存在的目录：

```sh
./bin/game_probe keygen --dir /secure/probe-run-73
```

生成目录权限 0700、`server.key` / `token` 权限 0600、`server.pub` 权限 0644，均为 32 字节原始文件（不是十六进制文本）。已有目录不会覆盖。私钥只发给服务器；经可信渠道将 `server.pub` 和 `token` 分发给每个客户端，token 保持 0600。不要把密钥放到命令行、Git、日志或公开归档。符号链接、非普通文件、长度错误和秘密文件权限过宽均被拒绝。

服务端示例（将监听地址改成测试网卡地址，防火墙仅允许发生器接入）：

```sh
./bin/game_probe server --transport tcp --bind 127.0.0.1:19073 \
  --run-id 73 --key-file /secure/probe-run-73/server.key \
  --token-file /secure/probe-run-73/token --connections 16 \
  --duration-ms 30000 --payload-bytes 64 > server-73.log 2>&1
```

等服务端输出 `status=listening` 后，在每个发生器上运行（远端测试将 `localhost` 改成实际主机名或 IP）：

```sh
./bin/game_probe client --transport tcp --host localhost --port 19073 \
  --run-id 73 --generator-id 1 --connections 4 \
  --pin-file /secure/probe-run-73/server.pub \
  --token-file /secure/probe-run-73/token --duration-ms 5000 \
  --join-timeout-ms 5000 --drain-ms 2000 --rate 20 --payload-bytes 64 \
  > client-73-1.log 2>&1
```

另一进程使用不同的 `--generator-id`。同一次运行的 `run-id`、传输和 payload 长度必须一致。换传输时两端同时改为 `udp` 或 `kcp`，重新启动本次测试。服务器 lifetime 从监听开始计算，必须覆盖启动间隔、客户端 join、load 和 **两段 drain**；上例客户端最长约 14 秒，须在服务器剩余时间足够时启动。

服务器始终启用加密，客户端固定校验公钥；join ticket 使用 token 的 HMAC-SHA256 绑定运行/发生器/流。测试不开放匿名 echo。凭据只用于受控测试，不是生产玩家登录体系；相同 token 的持有者可生成任何发生器 ID，应按运行轮换。

## 参数与边界

| 参数 | 默认值与含义 |
| --- | --- |
| `--connections` | 1；客户端流数量，服务端**整个运行期间**不同流的总预算，1–4096。不是仅在线连接上限；同一身份结束后也不能重用 |
| `--payload-bytes` | 64；含探针头的业务消息总字节，32–65536；UDP 上限 1024，避免把大包分片当成默认基线 |
| `--rate` | 20；每个客户端流每秒期望发起数量，1–10000，不保证成功入队数量 |
| `--duration-ms` | 10000；客户端 load 时长/服务器监听时长，1–259200000；支持时长不代表已做过长稳 |
| `--join-timeout-ms` | 10000；客户端所有连接建立总期限，同时约束底层建连/握手，100–60000 |
| `--drain-ms` | 5000；load 后回包等待，再额外同长度等待结束确认，100–60000 |
| `--report-ms` | 1000；累计报告间隔，100–60000 |
| `--queue-messages` | 4096；应用重试/游戏调度层分别有界，1–65536；底层每写队列最多 1024 |
| `--queue-bytes` | 8388608；各排队层分别使用的字节预算，256–67108864，须容纳 payload 加 256 字节余量；**不是总内存上限** |
| `--inflight` | 4096；所有客户端流共享的未完成时间戳数量，1–65536 |
| `--max-rtt-p99-us` | 未设置；客户端 RTT P99 上限，1–60000000 微秒，无样本失败 |
| `--max-unobserved-ppm` | 未设置；未观察回包比例上限，0–1000000 ppm，无 admitted 样本失败 |

服务器的总流预算须覆盖所有发生器。每个流的身份由 run/generator/stream 三元组组成；本地网络 session 句柄不跨进程比较。本工具不自动重连、不恢复、不调整线上加密策略。

## 载荷与完成条件

32 字节探针业务头按大端排列：4 字节 tag、8 字节 run、8 字节 generator、4 字节 stream、8 字节 sequence。数据 tag 为 `RNPD`，其余数据填充 `0x5a` 并逐字节验证；结束请求 `RNPF` 和确认 `RNPA` 都是 32 字节，sequence 表示该流最终 admitted 数量。这是探针自己的业务协议，不新增库层 msg_type。

TCP/KCP 要求序号严格连续；服务端按流 FIFO、跨流轮转重试。客户端必须收到所有已 admitted 消息的 echo，并收到匹配的结束 ACK 才完成。黑洞、断线、坏包、重试预算耗尽或 drain 超时均非零退出。

UDP 用 64 序号窗口区分新消息、重复、乱序与窗口外消息。窗口外无法证明唯一性，不计成功接收。客户端在 echo grace 后发送 finish，每 100 ms 重试直至额外 ACK 期限结束；晚到 echo 没有本地时间戳时计 `late`，不伪造 RTT。服务器只确认 finish 已处理，不证明 ACK 已到客户端，因此必须同时检查所有客户端报告。服务端零有效流量不能报告成功。

## 如何读报告

报告是每行 `key=value`，`running` 为累计快照，`completed` 为成功结束，`failed` 为带原因的失败。保留每个进程完整日志及退出码，不只截最后一行。

- 客户端 `offered = admitted + rejected`。admitted 仅表示库接受发送；received 才是验证通过且窗口可识别的新回包。调度赶不上 rate 的 tick 计入 `schedule_missed` 和 rejected，不偷偷降低目标负载。inflight 拒绝与 WouldBlock 另计。
- 服务端 received 是合法唯一数据数量，echoed 是 echo 被库接受的数量，不证明客户端已收到；服务端 offered/admitted/rejected、RTT 与 echo_unobserved 不适用，保持零。`auth_rejected` 是拒绝认证数。
- 客户端 `echo_unobserved = admitted - received`。UDP 这是**往返未观察比例**，包含两个方向丢失、晚到、窗口外、库/应用排队或丢弃；不是单方向物理丢包率。运行中的差值包含仍在途消息。
- `pending_messages/bytes`：服务端是应用重试队列；客户端是本地时间戳数量，pending_bytes 为零（不存 payload），不代表没有内存占用。scheduled/transport 字段描述另外的队列层，不能混成同一预算。
- RTT 来自客户端入队前到 echo 处理时的本地单调时钟，包含排队和 poll 延迟。P95/P99/P999 使用现有 2 的幂直方图桶近似，`rtt_samples=0` 表示未采样，不能把零 P99 当低延迟。快照从进程启动累计，不是滑动窗口；不能直接平均多个发生器的 P99。
- CPU 为平均进程 CPU 百分比，一核约 100%；RSS 单位 KiB。资源读数缺失用 `unavailable`，不是零。`closed_by_reason` 数组索引是 SDK `ErrorCode` 判别值的绝对值（Ok 为 0）；报告还含心跳超时、丢弃、协议错误和队列尾延迟。
- `binary_sha256` 关联实际执行文件；包内运行另有 `package_manifest_sha256`，源码构建通常为 unavailable。结合发布证据里的 BUILD-INFO、归档哈希、源码指纹保存；哈希本身不是签名。
- 不设置 SLO 时输出 `slo=not_evaluated`。退出 0 只证明本次完成，**不等于达到目标速率或容量**；若需要要求 admitted/offered 比例，也必须在验收中核对。SLO 配置后没有样本不能通过。

正常运行和受控失败先保存运行时快照再停止 runtime，批量发送中途失败保留此前成功入队计数。启动失败尚无 runtime 时仅报告原因。SIGKILL、崩溃或宿主机丢失不能保证最终报告，缺少 completed 一律视为未完成；外部任务编排应保存退出码并清理子进程。

## 已有短测与仍需目标验收

`cargo test -p rnet-game --bin game_probe --test game_probe -- --test-threads=1` 覆盖三传输独立进程、多个发生器、公钥/凭据错误、身份重用拒绝、背压、坏回包、可靠黑洞与缺失 finish ACK。`make package-smoke` 将相同进程测试指向**实际解压的发布包二进制**。

资源不足的测试输出 `SKIPPED game probe smoke:`；发布证据工具将其记为 blocked，不能当作通过。完整验收仍需干净冻结提交、实际目标机与游戏业务集成、弱网矩阵、性能预算、长期运行及独立安全审查。
