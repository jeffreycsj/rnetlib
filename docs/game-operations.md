# 游戏库采集、告警与排障

适用范围：当前 SDK、Linux 专服、持续正容量 poll、同 runtime 恢复。本文及 `examples/operations` 是接入示例，不代表实际集群已部署监控、通知链路已验证或生产容量已获认证。所有阈值必须经过部署方确认。

## 采集接入：由宿主提供端点

库返回 Prometheus 文本，**不启动 HTTP 服务**。将下面的接口接到应用已有的受保护管理端点；不要在游戏协议监听端口上直接提供 `/metrics`。

| 接入语言 | 已有游戏 API | 所有权和异常处理 |
| --- | --- | --- |
| Rust | `GameRuntime::prometheus_snapshot()` | 返回 owned String；并发更新不是全字段原子快照 |
| C | `rnet_game_prometheus_snapshot(runtime, &buffer)` | 检查状态码，按 `data/len` 复制，不当作 NUL 字符串；最后释放非零 token：`rnet_game_buffer_release(runtime, buffer.token)` |
| C++11 | `GameRuntime::prometheus_snapshot()` | 返回 owned std::string；错误抛异常，管理请求需捕获 |
| Go | `(*GameRuntime).PrometheusSnapshot()` | 返回 `(string, error)`；wrapper 负责释放 native buffer |
| C# | `GameRuntime.PrometheusSnapshot()` | 返回 owned string；异常不要转成 HTTP 200 的空内容 |

宿主接入约定：

1. scrape 只查询已有 runtime，不创建新 runtime，也不调用 `poll` 或驱动业务。游戏线程仍须持续正容量 poll；零容量不能替代消费事件。
2. 在已有管理 HTTP 处理器返回 UTF-8 文本，Content-Type 为 `text/plain; version=0.0.4; charset=utf-8`。异常、空快照或已销毁 runtime 返回失败状态，不伪装成功。避免 scrape 与销毁交错：Rust 保持引用，外语 wrapper 保持 lease，C 由宿主协调生命周期。
3. 示例采集地址为 `127.0.0.1:9464/metrics`，**没有程序替你监听它**。仅同机管理访问可使用这个 HTTP 示例；跨机须由已有采集设施提供私网 ACL、认证及 TLS，不能直接暴露公网。
4. 默认一个 target 对应一个 runtime。多个 runtime 不可直接拼接相同系列文本：使用已有指标注册器加固定、有限的 `runtime` 槽位标签，或拆成不同 target。不得使用玩家 ID、session handle、ticket、correlation ID 或不断变化的启动 ID 作为指标标签。
5. 采样失败不可无限期返回缓存的成功快照。`up=1` 只能证明 HTTP 可用，不能证明游戏线程还在 poll。必须另接宿主事件循环/watchdog 健康监测；SDK 没有导出 poll 最后成功时间。下面的缺失告警只能识别整个 target 的 facade 指标缺失，不能证明多 runtime 中每一个都存在。

`examples/operations/prometheus.example.json` 是 Prometheus 可读取的 JSON/YAML 配置，避免开发工具引入 YAML 解析依赖。默认 scrape/evaluation 间隔 15 秒、请求超时 5 秒、`honor_labels=false`；将该 job 和规则合并到已有配置，不覆盖其他监控配置。跨进程计数由 Prometheus 的 job/instance 标签隔离，跨实例百分位不能直接平均。

本批没有部署 Prometheus、Alertmanager、管理 HTTP 端点或通知接收人。部署方必须在预发布环境验证报警实际送达，不能仅看规则单测通过。

## 指标口径与查询

| 指标族 | 类型/单位 | 注意事项 |
| --- | --- | --- |
| `rnet_queued_send_bytes` / `rnet_queued_event_bytes` | 当前 gauge，字节 | 发送与事件是不同预算层；不要用 lifetime peak 判断当前压力 |
| `rnet_game_scheduled_queued_bytes` | 当前 gauge，字节 | 游戏调度层与 transport 队列独立，不相互抵扣预算 |
| `rnet_events_dropped_total` / `rnet_lifecycle_events_rejected_total` | 累计 counter | 事件交付/生命周期准入失败不是业务成功 |
| `rnet_send_would_block_total` / `rnet_game_scheduled_admission_rejected_total` | 累计 counter | 背压拒绝不是被接受的发送，不能混入吞吐分母 |
| `rnet_game_scheduled_expired_dropped_total` / `*_send_failed_total` | 累计 counter | 已入队不代表最终交付，查业务 deadline 与关联日志 |
| `rnet_game_realtime_backpressure_dropped_total` | 累计 counter | 快照因压力丢弃；正常 LatestOnly replacement 本身不是故障 |
| `rnet_admission_rejected_reason_total{reason=...}` | 按有限原因分组的 counter | 限流可能是正确执行策略；先查原因，不立即扩大准入 |
| `rnet_session_closed_total{reason_code=...}` | counter | 包括建立失败，不仅已 Ready 会话；原因是 SDK ErrorCode 的负值 |
| `rnet_game_heartbeat_timeouts_total` | counter | 触发关会话请求，不是物理丢包计数 |
| `rnet_game_logger_enabled` / `*_dropped_total` / `*_sink_panics_total` | enabled 是 gauge，其余 counter | 默认可不配置 logger；生产排障应显式配置并监测丢日志 |
| `rnet_game_heartbeat_rtt_us{quantile=...}` | gauge，微秒 | 生命周期累计近似百分位；配套 `rnet_game_heartbeat_rtt_samples_total` |
| `rnet_latency_us{kind=...,quantile=...}` | gauge，微秒 | kind 4=发送队列、5=事件队列、7=KCP tick 延迟；配套 `rnet_latency_samples_total{kind=...}` |

`*_total` 在 runtime 重建时归零；用 `rate` / `increase` 处理重置，**先对每条 counter 求变化，再按需要聚合**，不要先 sum 再 rate。零样本表示未采样，不是零延迟。以下查询保留实例/运行时隔离：

```promql
# 各实例事件交付丢失速率；不要给 gauge 用 rate。
rate(rnet_events_dropped_total{job="rnet-game"}[5m])

# 生命周期心跳 P95，换成 0.99 可看 P99；微秒换算为毫秒。
rnet_game_heartbeat_rtt_us{job="rnet-game",quantile="0.95"} / 1000

# 关闭原因保留 reason_code，排查前不要把错误原因全部聚合掉。
increase(rnet_session_closed_total{job="rnet-game"}[5m])

# KCP 维护 tick 延迟，不能称作网络 RTT。
rnet_latency_us{job="rnet-game",kind="7",quantile="0.99"} / 1000
```

导出的百分位是约 2 的幂直方图桶估计，没有导出 Prometheus `_bucket`，不能对它们使用 `histogram_quantile`。`increase(samples[5m])` 只判断近期确有新采样，**不会把累计 P99 变成五分钟 P99**，旧慢样本仍可能影响告警。生产 SLO 如要求时间窗口：由单一采集者调用 `drain_latency_window()` / `drain_heartbeat_rtt_window()`，或宿主另外记录业务端到端直方图；其他读者不可再次 drain 同一个共享窗口。窗口读取并非跨所有 bucket 的原子时间切片。

UDP 缺口、KCP 重传和 TCP 心跳 RTT 语义不同。`network_quality` 适合受限的单 session 排障，不要把每个 session 的质量常驻为高基数标签；TCP 没有可报告的应用丢包率。业务端到端延迟也不等于受保护心跳 RTT。

## 示例告警：须改阈值与路由

规则名与排障入口一一对应；只加载到 `job="rnet-game"` 的受控部署。

| 告警 | 示例条件 | 首要行动 |
| --- | --- | --- |
| RNetScrapeDown / RNetNoTargets | scrape 失败/发现中已无 target，持续 1 分钟 | 查进程、管理链路和服务发现，不先归因游戏网络 |
| RNetMetricsMissing | HTTP 成功但缺少 game facade 的 logger-enabled 系列，1 分钟 | 查是否误用了 transport API、空快照或错误管理处理器 |
| RNetQueuePressure | 当前 queued bytes 超阈值 2 分钟 | 核对实际预算、消费速度与 poll 卡顿，再查发送拥塞 |
| RNetEventLoss | 5 分钟累计变化 >0，持续 1 分钟 | 查事件预算及消费者；高优先级保留故障时间线 |
| RNetLogLoss / RNetLoggerDisabled | 丢日志/回调 panic，或 5 分钟未配置 logger | 查日志组件阻塞、异常与队列；不要在回调重入销毁 runtime |
| RNetHeartbeatTimeouts | 5 分钟有超时，持续 1 分钟 | 先查两端事件循环停顿、排队，再查链路与协议日志 |
| RNetAdmissionRejected | 原因计数或发送调度拒绝增长，1 分钟 | 区分防护成功、配置过紧、连接风暴与真实容量耗尽 |
| RNetUnexpectedClosures | timeout/IO/protocol/panic/handshake/key/crypto 关闭增长，1 分钟 | 用原因码和结构化日志定位本地/对端失败路径 |
| RNetGameDeliveryFailure | expiry、永久发送失败或实时压力丢弃，1 分钟 | 查业务 deadline、快照策略、priority 与应用过载 |
| RNetHeartbeatHistoricalTail | 累计 P99 >100 ms，近 5 分钟至少 20 新样本，持续 2 分钟 | 历史尾延迟诊断提示，**不是窗口 SLO 违规判定** |
| RNetQueueHistoricalTail | kind 4/5/7 累计 P99 >50 ms，同样有新样本，2 分钟 | 分清发送队列、事件消费与 KCP 维护延期 |

QueuePressure 中发送/事件阈值 `53687091` 字节来自 **示例** 64 MiB 预算的约 80%；调度层 `6710886` 来自示例 8 MiB 的约 80%。这些不是 SDK 默认值，也不会自动读取配置。实际三层预算改变时必须一起更新规则并重新测试。库当前没有统一导出每层容量，不能凭空使用 `rnet_queue_capacity_bytes` 等不存在的指标。

`for` 会滤掉短暂条件，但 `increase[5m]>0` 可能在故障已停止后继续保持数分钟；反之，全程缺指标会令该故障规则不触发。采集失败、指标缺失、宿主 watchdog 与业务 SLO 必须一起运行。NoTargets 只有在确认该部署应存在 game target 时才加载；计划停服应通过有期限的告警静默处理。

普通主动关闭/取消、业务拒绝登录不属于这里的 UnexpectedClosures，但仍应保留其计数。常用码：`-5` Timeout、`-7` IoError、`-8` ProtocolError、`-10` InternalPanic、`-12` HandshakeFailed、`-14` PeerKeyMismatch、`-16` CryptoError；`-18` Cancelled。这些原因不是玩家违规证据。

### 外语日志组件的额外异常

Go 和 C# 在 native 捕获之外还会拦截宿主日志回调异常。通用 Rust 指标契约不包含它们；部署对应 wrapper 时，在已有告警系统额外配置：

```promql
increase(rnet_game_go_logger_panics_total{job="rnet-game"}[5m]) > 0
increase(rnet_game_csharp_logger_exceptions_total{job="rnet-game"}[5m]) > 0
```

Go 未配置 sink 时该系列可能不存在，不可把 absent 当 0 次回调异常已验证。以上为语言特定接入项，不宣称已包含在通用 13 条规则里。应由 wrapper 联调验证触发与通知。

## 日志与故障时间线

调用方通过现有 BoundedLogger / C logger 回调 / 对应语言 logger 接口接入自己的组件。使用 `set_metrics_log_interval`（或对应语言入口）开启有界累计摘要；它依赖 poll，不是独立后台健康守护。保留 `timestamp_unix_ms`、level、event_name、runtime、endpoint、session、transport、error_code、correlation_id 和经过安全审查的 message。时间戳是 Unix 毫秒；跨机时间先核对主机同步，游戏 ClockSync 不是可信墙钟或反外挂依据。

事件驱动日志要等事件被 poll；停 poll 时日志停更不能证明没有失败。生命周期日志保留错误原因，不记私钥、join/resume ticket、cookie、业务 payload。correlation ID 只用于有界日志检索，不放进 Prometheus 标签；session 为本 runtime 的本地句柄，恢复映射为新句柄后必须按映射关联，不能跨进程直接比数值。

统一排障顺序：

1. 固定时间范围、服务实例、native/SDK 包哈希、启动配置和业务版本；保留原始报告，勿仅截 P99。
2. 检查进程与管理采集是否可用、应用是否持续 poll、CPU/RSS/宿主网络是否异常。
3. 对照各层当前队列与实际预算、事件丢弃、WouldBlock、游戏 expiry/requeue 以及 KCP 缓存；不同层不能当成同一个上限。
4. 检查 heartbeat matched/rejected、关闭原因和握手/授权日志。密钥不匹配需修凭据分发，不要通过关闭公钥校验“恢复”。
5. 采集已经就绪且仍存活的有限 session 的 quality/clock 样本，标记缺失值；只在授权环境重现弱网，不对在线机器直接修改 tc。
6. 验证恢复条件：队列消退、无新增交付失败、样本足够、业务成功率回升、日志无丢失，再撤销临时措施。

## 升级、回退与验收

使用[容量/SLO 模板](game-capacity-template.md)记录明确边界；发布证据与[独立探针](distributed-game-probe.md)用于可追溯的验收，不代替应用业务成功率。

- 发布前冻结干净提交，按[发布证据门禁](release-evidence.md)重新生成、测试并归档包。记录实际工具版本、规则配置、阈值及通知演练结果；规则 JSON 随 SDK 包分发。
- 当前 SDK 不兼容旧 SDK/wire。灰度使用隔离、匹配版本的客户端/服务端池，不把旧 native DLL 与新 wrapper 混搭。保留上一套**完整匹配包**、公钥配置及业务数据兼容性说明。
- 停止向旧池分配新会话，用宿主游戏逻辑排空比赛/保存业务状态，再关闭端点与 runtime。不要把关闭监听当作透明迁移；操作前确认在途玩家影响。
- 网络恢复票据只在同 runtime、同 listener 内一次性有效。进程重启、滚动升级、回退都需要重新登录；跨实例恢复不是当前支持项。
- 依据预先写定的错误率、延迟、队列恢复和资源阈值决定回退；停止新流量并保存证据，再将客户端/服务端池一起切回匹配包。密钥及账号数据的回退需单独安全审批，不在本库脚本自动处理。
- 功能和规则冻结后才开始目标机弱网与长稳；本批不启动长稳、远端服务或任何生产变更。

## 校验方法与参考

```sh
PROMTOOL=/trusted/path/promtool make ops-test
# 也可以在交付包 examples/operations 目录执行：
promtool check config prometheus.example.json
promtool test rules alerts.test.json outcomes.test.json
```

`make check` 和包验证都要求 promtool，缺失即失败，不跳过。CI 固定校验了官方 2.55.1 归档 SHA256，并且只提取离线 promtool；这是测试基线，不是推荐部署该旧版 Prometheus 服务。部署方仍须用实际监控栈版本再次验证。规则测试包括 8 组正反场景；Rust 测试通过真实 runtime 核对导出 selector，Python 检查规则引用是否在该契约内。

参考：[Prometheus 规则单测](https://prometheus.io/docs/prometheus/latest/configuration/unit_testing_rules/)、[counter 变化函数](https://prometheus.io/docs/prometheus/latest/querying/functions/)、[向量匹配](https://prometheus.io/docs/prometheus/latest/querying/operators/)。本库单位、累计/窗口及丢弃语义以本仓库实现契约为准。
