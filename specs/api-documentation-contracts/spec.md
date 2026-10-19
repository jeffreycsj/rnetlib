# 网络库注释与接入契约补全

状态：用户已确认，实施中。初始基线：`437bcec`；不改变 wire、ABI 或默认值。发现的行为缺陷按独立 RED → GREEN 修复并记录，不混作纯注释变更。

## 目标与已确认缺口

调用方应能通过 IDE 悬停和生成的 API 文档理解如何使用库，而不必阅读实现来猜参数含义。覆盖 Rust、C、C++11、Go、C# 的公开接口和内部关键状态路径；不能只补 README，或给函数加一句重复函数名的说明。

- Rust `GameProtocol`、`GameSendOptions` 部分成员及 `auth_decide` 等操作缺少契约；`poll` 的公开说明没有解释零容量、等待与驱动状态机的关系。
- C 头文件配置大量成员没有逐项说明，若干函数只有声明；事件复用字段缺少完整的事件适用范围说明。
- C++ 游戏配置、事件、质量快照和多数方法缺少可供 IDE 展示的文档。
- Go 的类型级说明不能替代字段说明；配置零值、事件数据用途、可用性标记和统计单位需要逐项核实。
- C# options 大多数属性和方法缺 XML 文档；项目尚未开启 XML 文档产物。
- 当前有格式、结构、测试门禁，但缺公开 API 注释覆盖门禁。

## 注释契约

代码注释沿用现有英文风格，中文接入文档负责串联使用过程。每种语言的必要注释必须在本地类型/方法上可找到，不以“自行查 C 头文件”替代。

| 对象 | 必须解释的内容 |
| --- | --- |
| 函数/方法 | 用途、前置状态、参数、返回值、主要错误条件、同步返回与异步事件的区别、副作用 |
| 配置成员 | 作用范围、单位、有效范围、零值/null/None 含义、默认来源、创建后能否修改 |
| 句柄/缓冲区 | endpoint 与 session 区别、有效期、借用或复制、释放责任、是否含敏感信息 |
| 事件成员 | 哪些事件有效、其他场景能否读取、旧/新会话映射、载荷和辅助载荷的具体用途 |
| 指标/日志 | 累计或窗口、计数或瞬时值、采样范围、单位、无样本/不可用、回调线程与生命周期 |
| 内部关键路径 | 不变量、锁顺序、资源预算归还、失败回滚、可信与未校验数据的不同处理原因 |

公共规则集中在模块/类型说明中，具体方法用稳定符号链接引用。每个公开成员仍须有自身语义说明；不添加逐行翻译、脆弱行号或与实现不符的承诺。

## 实施顺序

1. [x] Rust 游戏公开 API：配置、事件、收发、认证、恢复、加密切换、质量与日志。核实实现和测试后补注释，补可编译接入示例；补全文档的模块先启用缺失文档检查。
2. [x] C ABI 与 C++11：逐参数/成员说明、错误与异步完成事件、token 释放、日志回调重入限制和 RAII 生命周期。对照 FFI 转换核实，保持布局和签名不变。
3. [x] Go 与 C#：按实际包装语义说明零值/默认值、错误返回/异常、复制与销毁；GoDoc 和 XML 文档覆盖公开类型、常量、成员与方法。
4. [x] Rust 底层及内部关键路径：core、transport、protocol、security、observe 的公开契约；重点补握手、SELECT/ACK/READY、旧会话接管、心跳调度、动态安全状态、发送队列预算与端点清理的设计原因。
   - [x] core / protocol / security 公开契约及缺失文档门禁；队列预算/锁序、导出内存所有权、握手信任、重放窗口与换钥屏障说明。
   - [x] observe 公开契约（第一阶段已完成）。
   - [x] transport 公开配置、生命周期、发送与遥测契约。
   - [x] game / transport 的 SELECT/ACK/READY、恢复接管、心跳调度及端点清理内部路径收尾。
5. [ ] 接入文档与防回退：统一多语言建立、授权、持续 poll、发送、接收、背压、关闭和日志示例；将可验证的注释/文档构建门禁接入现有检查。
6. [ ] 对抗性复查：抽查每种语言的完整调用链，对照实际 diff、边界测试与生成文档检查语义；发现运行缺陷时，独立建立 RED → GREEN 回归后修复，不把行为变化藏在注释提交中。

补注释超过现有文件行数限制时，按职责拆分，不通过放宽结构门禁或巨型公共文件解决。

## 优先核对的易误用契约

- `connect` 返回 endpoint 不等于游戏就绪；授权同意也不等于可以立即发送。
- 普通发送成功代表本地接受，不承诺到达；区分负载上限、封装开销、背压及瞬时拒绝。
- LatestOnly 不能撤回 socket/KCP 已接管内容；队列过期也不是撤回协议。准确说明各阶段的替换边界。
- 服务器发起安全策略变化不等于对端立即完成切换；客户端自动跟随，明文业务没有完整性和机密性保证，控制通道仍受保护。
- 恢复只在同一服务端 runtime 内有效；重新授权和协商成功后才接管旧会话，不承诺重启、滚动发布或跨实例恢复。
- 持续 poll 是游戏状态推进的条件；零容量仍推进内部工作。等待参数不应被描述为包含锁等待等全部开销的硬实时上限。
- 日志回调不能销毁自己的 runtime；错误原因、采样计数与 P95/P99 的统计口径需要能够追溯。
- UDP 缺口、KCP 重传不是同一种丢包指标；TCP 不提供丢包率，时钟偏移不是可信反外挂时间。

## 验收与范围边界

- 按阶段建立公开符号清单，声明/成员文档覆盖与人工契约检查都通过才勾选完成；不以新增注释行数作为完成指标。
- Rust 文档检查缺失文档和断链，并运行 doctest；C/C++ 保持 ABI 导出和 C++11 示例可编译；Go 检查 GoDoc 并执行测试；C# 构建 XML 文档并对已覆盖范围启用缺文档诊断。
- 结构、格式、严格 Clippy 与相关测试通过；涉及拆分、示例或检查工具时验证相关行为。纯注释修改不捏造功能 RED，新增行为测试必须真实 RED → GREEN。
- 结束时复查实际 diff，不改接口、默认值或协议；如必须修改这些契约，单独说明原因及测试证据。
- 构建/测试串行，运行前和期间保留至少 15 GiB 可用内存；未运行或未通过的检查明确记录。
- 不做跨实例恢复，本阶段不启动长时间压测，不提交或推送未获授权的修改。补完注释不等于取得生产资格；最终干净发布树验证、目标平台验证与长稳单独收尾。

## 第一阶段记录（2026-10-14）

- `rnet-game`、`rnet-observe` 已启用 `deny(missing_docs)`，覆盖各自可达的公开类型、成员、枚举分支及方法。先验证门禁分别报出 199 / 37 处缺失，再补注释后通过；门禁不能替代人工核对参数与实现的语义一致性。
- 补齐游戏配置、授权/版本协商/恢复、收发、背压、零容量 poll、加密切换、质量/时钟、队列/监控、回调生命周期；同步补充游戏 API 重导出的 KCP 重传和 transport Latest 快照成员。底层其他类型及内部状态字段尚未全覆盖。
- 现有 `game_quickstart` 源文件直接纳入 crate 文档并作为 doctest 实际运行，不维护两份示例。新增 `make doc-check`，覆盖日志及游戏文档严格构建、游戏 all-features 文档和 doctest；接入 `make check` 与 Linux CI。
- 初次完整回归暴露原有自动换钥测试丢事件：辅助函数取出一批事件后只保留 Message，丢失同批 SecurityChanged。通过先等待真实消息与双方换钥通知入队确定性复现，再改为一次循环验证消息及双方换钥。TCP、UDP、KCP 均通过；未修改换钥实现、ABI 或 wire。
- 最终当前修改树已通过：workspace 全部测试、严格 Clippy、格式/结构、严格文档构建和 doctest。构建串行并使用 15 GiB 可用内存守护。未重跑多语言 SDK、ASan/Miri/Loom 专项、发布打包或目标长稳；没有把历史资格转记到本阶段。
- 阶段日志保存在本机 `/tmp/netlib-api-docs-gates-final.log`；换钥回归 RED/GREEN 分别为 `/tmp/netlib-rekey-batch-red.log` 和 `/tmp/netlib-rekey-batch-green.log`。这些是当前工作树的临时验证证据，不是干净发布提交的资格报告。

### 本批对抗性复查

| 发现 | 处理 |
| --- | --- |
| 发送说明过度承诺不会丢失，混淆本地入队和远端送达 | 明确成功仅代表 admission，说明断线/过期/替换边界和异步失败指标 |
| EndpointError 容易被误解为端点必然关闭 | 对照地址切换和 TCP 错误路径，明确必须查询端点有效性 |
| 部分质量查询的跟踪状态不等于游戏就绪 | 注释按实际跟踪时机描述，不承诺查询成功即可发送 |
| 窗口指标容易被误解为整个快照原子一致 | 明确独立桶更新/轮换及并发读取边界，区别无样本与零值 |
| 自动换钥测试丢弃同批安全通知 | 独立 RED → GREEN 修复测试收集逻辑，保留消息与双方完成检查 |

本批没有遗留已确认的新运行逻辑缺陷；这不是对整个库的安全认证。后续继续第 2/3 项多语言注释，第 4 项底层/内部关键路径，以及第 5/6 项的全范围文档和审查收尾。

## 第二阶段记录（2026-10-14）

本批基线：`7f0a5ff`。完成 C ABI 与 C++11 公开契约，仍未完成整个库的注释计划。

- C 头文件覆盖 55 个导出入口、24 个结构体的 303 个成员、75 个枚举常量，以及日志/公钥校验回调。说明输入与输出布局、参数单位/零值、句柄所属 runtime、异步就绪、发送背压、加密切换、恢复边界、事件 token 与敏感字节释放、P95/P99/样本口径。
- C++11 六个职责头文件补齐公开类型、枚举、方法与 160 个结构体成员；说明 move-only 生命周期、错误异常、借用/复制、显式 close 与析构兜底的差异。配置、质量、恢复、排队/淘汰和日志不要求接入方去读 Rust 实现。
- 通过 C 预处理结果与基线对比确认声明/常量/布局未变；保持单一当前 ABI/wire，没有兼容旧 SDK。所有 C++ 职责头文件仍在现有 600 行限制以内，没有放宽门禁。
- 独立运行修复：底层 C++ `Runtime::poll` 的 reserve/载荷复制异常会遗留未释放 token；游戏 `GameRuntime::poll` 在 ABI 部分导出后先抛错，也会遗留该前缀的主/辅助 token。增加批次作用域守卫，先接管所有已导出 token 再转换错误；底层仍逐条释放已复制载荷，避免整个批次同时保留原生载荷和 C++ 副本。
- 新增 `cpp/tests/poll_ownership.cpp` 并接入 `make cpp-test`。测试使用真实包装层、受控 C 边界与分配失败注入，不耗尽机器内存；RED 明确复现三个底层复制位置及游戏部分导出泄漏，再 GREEN。覆盖正常二进制/空数据、零容量、主/辅助 token、未导出槽不释放、精确一次释放、错误诊断在清理前保存，以及及时释放已复制载荷的内存边界。

### 验证结果

- 修改前 `make -j1 cpp-test abi-check` 基线通过。
- workspace 全部测试（含 doctest）、严格 Clippy、Rust 格式、结构检查、ABI 导出检查通过。
- 最终 C11/C++11 头文件以 Clang `-Wdocumentation -Werror` 检查通过；最终异常安全回归和四个真实联网 C++11 示例均通过，`git diff --check` 通过。
- Clang 文档诊断检查语法，不等于缺失文档门禁；本批用声明/成员清单加人工语义核对确认覆盖。C/C++ 永久缺失文档检查和文档生成仍属于第 5 项，未伪称已经交付。
- 最终构建/测试串行，使用至少保留 15 GiB 可用内存的守护。临时日志：`/tmp/netlib-cpp-docs-baseline.log`、`/tmp/netlib-cpp-ownership-red.log`、`/tmp/netlib-cpp-ownership-green.log`、`/tmp/netlib-cpp-contracts-validation-final.log`、`/tmp/netlib-cpp-contracts-postreview.log`。
- 未重跑 Go/C# 接入、非 Linux 平台、ASan fuzz/Miri/Loom 专项、发布打包或目标长稳。没有提交/推送，也没有把本批工作树验证转记成生产资格。

### 本批对抗性审查

| 严重度 | 位置/问题 | 处理 |
| --- | --- | --- |
| 中 | C++ 底层 poll 的分配异常遗漏批次 token；游戏 poll 的部分导出失败在守卫前抛错 | 独立 RED → GREEN 加批次守卫和两类包装层回归，保护返回前缀与辅助 token |
| 中 | 单纯延后释放整个底层批次会提高复制期间的峰值内存 | 保留逐条释放并清零已释放 token；回归断言复制下一条前已释放上一条 |
| 中 | 旧 SECURITY_CHANGED 注释漏 operation 字节；LatestOnly 边界仍写成全部止于游戏队列 | 按实际十字节事件格式及 TCP/KCP worker pickup 边界纠正 |
| 低 | 同模式设置可能无事件；ProtocolViolation 的 status 为 OK；tick 延迟不是窗口 | 明确 no-op、不以 status 代替事件类型、tick-bearing 累计样本口径 |
| 低 | 容易把跨 runtime token 当成全局唯一并依赖错误拒绝 | 明确调用方必须保留所属 runtime，不承诺跨 runtime 数值碰撞能被拒绝 |

测试盲区：故障注入验证 C++ 所有权，不替代真实 Rust 分配器 OOM/abort 场景或所有平台的异常 ABI；四个联网示例验证正常集成，不代替目标容量/弱网长稳。Go/C# 与底层内部注释仍待后续阶段。

整体评价：本批 C/C++ 公开契约和确认的包装层异常安全问题已闭环；继续第 3 项 Go/C#、第 4 项底层关键路径与第 5/6 项全范围门禁/审查。长时间压测继续留在功能和接口收尾之后。

## 第三阶段记录（2026-10-14）

本批延续 `7f0a5ff` 上第二阶段的未提交修改，没有覆盖或撤销已有 C/C++ 变更。完成 Go 与 C# 公开契约；整个注释计划仍在实施中。

- Go 逐项覆盖公开类型、常量、结构体成员与方法，说明 transport/GameRuntime 差异、零值与默认来源、截断/向上取整的时间单位、授权与游戏就绪、同 runtime 恢复、发送背压/替换边界、日志线程与资源销毁。补充包级说明和现有接入指南的 GoDoc 查询入口。
- 新增 Go AST 文档存在性门禁，普通 `go test` 即执行。补完字段后先确认它仍检出 131 处类型/方法/常量遗漏，再补齐至通过；不是以类型注释替代字段注释，也不将存在性检查当作语义证明。
- C# 公开类型、枚举成员、属性、方法、参数和返回值补 XML 文档，开启 `GenerateDocumentationFile`。与已有 warnings-as-errors 配合，缺文档即 CS1591 失败：先确认 184 处错误，再补齐后严格构建零警告/零错误。生成的 `RNet.xml` 含 221 个文档 member；发布脚本增加与 DLL 同目录安装 XML，完整发布包本批未重新生成。
- Go 九个涉及结构体的文件已逐字段对比类型、名称与顺序；C# 顺序布局 GameMetrics 的声明拆分也逐字段对比，未改变布局。没有修改 wire、ABI、方法签名或默认值，没有放宽结构行数门禁。
- 独立运行修复：Go `GameRuntime.Poll` 在原生部分导出后失败时，原先先返回错误、未登记清理，导致已导出前缀的主/辅助 token 遗漏。现先保存原始线程局部错误，再登记前缀清理，最后判断错误返回。
- 新增仅 Linux 且显式 `rnet_test_faults` 标签启用的链接故障注入。使用真实 native 分配/释放/销毁，不 mock release 次数；覆盖零容量、部分导出的主/辅助 token、未导出槽仍归测试所有，以及清理不能覆盖原始错误。RED 时 native destroy 明确拒绝 outstanding buffers；GREEN 后正常销毁。该回归接入 `make go-test`，强制重新链接当前 native archive。

### 验证结果

- 修改前 `make -j1 go-test csharp-test` 基线通过。
- 最终 `make -j1 go-test csharp-test structure-check` 通过。C# TCP/UDP/KCP 均覆盖授权、echo、动态加密/换钥、Latest、元数据、恢复/版本范围、指标、GC/logger、错误和销毁；另覆盖错误公钥、回调异常/关闭禁止/重入查询、非法配置与并发销毁。
- `go test -race -a -p 1 -count=1 -tags rnet_test_faults ./go/rnet` 通过；同时包含注释门禁和部分导出错误路径。
- Go 格式检查、`git diff --check`、发布脚本 `bash -n` 通过，XML 文件实际生成且非空。高内存命令串行、启用至少保留 15 GiB 可用内存的守护。
- 本批没有重跑 Rust workspace 全套、ASan fuzz/Miri/Loom、完整打包或目标环境长稳；不将第二阶段历史结果当作当前发布资格。C# 仍只验收 Linux x86_64/.NET 8，不扩大平台承诺。
- 临时证据：`/tmp/netlib-managed-docs-baseline.log`、`/tmp/netlib-go-poll-red.log`、`/tmp/netlib-go-poll-green.log`、`/tmp/netlib-go-docs-missing.log`、`/tmp/netlib-go-docs-green.log`、`/tmp/netlib-csharp-docs-missing.log`、`/tmp/netlib-csharp-docs-build.log`、`/tmp/netlib-managed-contracts-validation.log`、`/tmp/netlib-go-contracts-race.log`。未提交或推送。

### 本批对抗性审查

| 严重度 | 位置/问题 | 处理 |
| --- | --- | --- |
| 中 | Go game Poll 部分导出失败时，提前返回遗漏主/辅助 token | 独立 RED → GREEN 修复清理顺序，以真实 native 销毁前置条件验证无泄漏 |
| 中 | C# Dispose 文档容易承诺返回时 native 与回调均已结束 | 明确 SafeHandle lease 延迟销毁、并发外部资源协调，以及清理失败不通过 RNetException 抛出 |
| 低 | 选定业务版本被误写成游戏就绪，或将查询成功当作可发送 | 对照 range 状态修正 Go 方法契约；C# 同样明确可先于授权/就绪 |
| 低 | 语言包装默认值、时长取整和异常计数口径不同 | 分别记录 Go 普通 Config/GameConfig 默认来源、expiry/log interval 的向上取整与 poll/heartbeat 的截断、C# managed logger exception 独立计数 |
| 低 | 原生 archive 更新不在 Go 缓存依赖图中 | 新故障回归和正常集成一样使用 `-a` 强制重新构建/链接 |

测试盲区：故障注入只验证 Go 包装层的错误前缀所有权，不证明真实分配器 OOM/abort 可恢复；Go race 不检测所有原生 Rust/C 数据竞争；文档存在性/XML 构建不自动证明语义。全平台验证、完整发布包和目标弱网长稳仍未覆盖。

整体评价：本批 Go/C# 公开契约、文档门禁和确认的 Go 资源泄漏已闭环。下一步继续第 4 项 Rust 底层与内部关键路径，以及第 5/6 项全范围示例、门禁和审查收尾；长稳仍在整体完善后进行，不能因此宣称生产认证完成。

## 第四阶段 A 记录（2026-10-15）

本批完成 core / protocol / security 子项，不将第 4 项整体勾选完成。记录时 HEAD 为 `05a2281`；前批 C/C++/Go/C# 成果已在该提交中，本轮没有执行提交或推送。

- `rnet-core` 从单个 600 多行入口拆分为 error、event、event_queue、handle、buffer、lifecycle 六个职责模块，crate 根继续重导出原公开符号。逐模块去除注释/导入/空白后与拆分前比较，原实现和测试 token 完全一致；入口变为 25 行，各模块均低于 600 行。
- 补齐 core 的错误分类、事件字段、优先队列可能淘汰业务/控制事件的边界、失败非事务语义、count/byte 预算、锁顺序、句柄所属表与代际回绕、导出缓冲区释放/空值/跨线程读取约束、生命周期原子操作及并发失败语义。
- 补齐 protocol 的帧字段、输入长度/零值、复制与错误语义；明确单帧上限不等于输入总内存上限、增量解析失败不可重放已经消费的前缀、序列化/解码本身不认证。新增真实运行的分片输入 doctest。
- 补齐 security 的身份密钥所有权、握手调用顺序与业务授权区别、有序记录/数据报区别、nonce 消耗、验签后更新重放窗口、换钥 commit 用旧密钥而 ack 用新密钥的驱动顺序。控制器不负责 I/O、重试或超时，不能把输入处理当作可回滚事务。
- 三个 crate 启用 `deny(missing_docs)`。实施前分别确认 core 84、protocol 17、security 29 个缺失诊断，再补至通过；严格 rustdoc 同时检查断链。扩大 `make doc-check` 和结构门禁，不放宽行数限制。

### 独立安全修复：公钥拒绝后的握手完成

原 InitiatorHandshake 的 Noise 状态在 read_response 校验 pin 之前已经推进；若直接使用该底层对象的调用方忽略 PeerKeyMismatch，仍可调用 finish / finish_datagram，得到发往错误服务器的最后一条握手消息（包含 join 凭据）和传输状态。

- 用真实不同服务器密钥构造两个回归，分别覆盖 ordered / datagram；RED 均确认公钥校验已失败但 finish 仍成功，不使用模拟密码学对象。
- 最小修复增加私有 pin-verified 状态，只有成功处理并匹配服务器公钥的响应才放行两种 finish；每次读响应先撤销旧验证状态。未验证时返回 InvalidState，不生成最终凭据消息。
- 增补第三个回归，覆盖曾校验成功后再次读入错误响应，不能复用旧验证结果；原成功握手、密文篡改、乱序/重放和动态切换测试继续验证。
- 现有 TCP / UDP/KCP 驱动在 read_response 后都传播错误并退出，所以这是底层 Rust 公开接口的纵深防护缺口，不宣称现行 SDK 正常建连路径已经存在可利用绕过。没有修改 wire、C ABI 或业务版本协商。

### 本批对抗性审查

| 严重度 | 位置/问题 | 处理 |
| --- | --- | --- |
| 中 | 底层 InitiatorHandshake 的 Noise 进度不能证明已通过服务器 pin 校验 | 独立 RED → GREEN 增加完成前信任检查及两种传输/失败复用回归 |
| 低 | 句柄数字容易被误解为跨 runtime 唯一、永不重复的身份 | 说明 owner-local、有限代际空间和跨表碰撞，不能承诺错误表一定拒绝 |
| 低 | 编解码和公开 Control 构造器容易被误认为会认证并验证全部状态 | 区分字节形态、认证和状态机检查；明确 caller 输入预算及构造器前置条件 |
| 低 | 优先事件失败被误解为没有副作用；poll timeout 被误解为硬截止 | 明确部分淘汰不可回滚、条件变量虚假唤醒和锁等待边界 |

测试盲区：缺失注释检查只能覆盖可达公开声明，不证明文档语义或内部注释完整；core 拆分保持实现，不代表修复其所有底层使用限制；新增真实握手测试不等于第三方密码学审计。transport 与 game 内部注释、全范围示例/门禁收尾、目标平台和最终长稳仍待后续。

### 验证结果

- 修改前完整 Rust workspace 测试基线通过；握手 pin-failure 两条新回归先失败，最小修复后完整 workspace 再次通过。
- 最终当前修改树通过 `cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、Rust 格式、结构与 `git diff --check`。
- 扩充后的 `make doc-check` 通过：core/protocol/security/observe 严格 rustdoc、game all-features 严格 rustdoc，以及五个 crate 的 doctest。期间检出的文档方括号误识别链接已修正，没有屏蔽诊断。
- 当前 native 重新链接后的 C++11 所有权故障回归和四个联网示例、Go 全套和部分导出故障回归、C# TCP/UDP/KCP 与错误 pin/回调/并发销毁检查均通过。
- 构建与测试串行，15 GiB 可用内存守护全程开启。未重跑独立 Go race、Miri、Loom、ASan fuzz、release 导出检查、完整打包或目标环境长稳；本批不是生产资格认证，也未提交/推送。
- 本机临时证据：`/tmp/netlib-rust-contracts-baseline.log`、`/tmp/netlib-rnet-{core,protocol,security}-docs-missing.log`、`/tmp/netlib-pin-finish-red.log`、`/tmp/netlib-pin-finish-green.log`、`/tmp/netlib-core-split-green.log`、`/tmp/netlib-core-protocol-security-docs-green.log`、`/tmp/netlib-core-security-final-validation.log`。

整体评价：底层三个 crate 的公开文档、core 职责拆分和确认的握手完成前信任检查已完成并回归；第 4 项仍需继续 transport 和游戏内部关键路径，第 5/6 项仍待全范围收尾。长稳继续安排在整体完善之后。

## 第四阶段 B 记录（2026-10-16）

本批基线为干净的 `83bd6c1`。完成第 4 项剩余的 transport 公开契约和已列出的游戏/传输关键路径注释；不代表所有内部成员均须逐行解释，也不代表第 5/6 项或生产资格已经完成。

- transport 全特性严格文档检查先检出 93 处缺失，再补至通过；新增 `deny(missing_docs)`，并将 transport all-features rustdoc 与 doctest 纳入 `make doc-check`。既检查公开项遗漏，也检查文档断链，不用屏蔽诊断通过。
- 补齐构造即启动、endpoint/session 区别、认证和异步打开、参数单位/限制/默认来源、DNS 候选与超时、发送复制/背压、TCP/KCP Latest 的 worker pickup 边界、服务器控制加密、关闭和 stop 的非投递屏障语义。
- 明确观测口径：P95/P99 等为微秒直方图估计，累计与窗口分别查询；KCP RTT 会在 update 重复采样，不等于游戏心跳 RTT；发送 frame/header/control 计数与接收 body 计数不对称，不能当成双向网络线速或送达量。
- 内部注释覆盖协商 phase/成员、READY 本地发布与旧句柄接管、票据已消费但候选失败时保留旧会话、恢复撤销锁与清理、心跳入队时间/超时、零容量 poll 的有界工作、端点注册与清理锁序、Latest 增量预算回滚和动态换钥回复前后效果。
- 源码 diff 除新增 lint 外仅有注释；未改运行逻辑、公开签名、结构布局、默认值、wire 或 SDK 版本，没有为纯注释变更伪造功能 RED。文件仍在原结构行数限制以内。

### 本批对抗性复查

| 发现的契约风险 | 处理 |
| --- | --- |
| 原有源地址限流说明只提 TCP，实际 UDP/KCP 的 cookie 验证后也持有准入许可 | 改为每 listener、按 IPv4 地址/IPv6 前缀的三种传输准入范围；区分全 runtime pending route 预算和 datagram 早期状态 |
| 将 datagram idle 描述为“无接收超时”，忽略发送也刷新活动时间 | 明确双向活动回收语义，不能代替游戏心跳 |
| 容易把 connect、命令入队、stop 和安全通知当成对端完成/消息送达 | 分别记录异步完成、本地屏障、尽力停止与并发命令边界 |
| 零容量 poll 与 completed 队列注释容易过度承诺 ACK 一定优先 | 说明有界扫描且遇到需延期的公开事件会停止，仍需持续正常容量 poll |
| 句柄和指标快照容易被当成全局唯一/原子一致 | 明确 owner-local、跨 runtime 数值碰撞和独立字段/桶采样 |

### 明确保留的运行逻辑待办

以下是核实旧实现时确认的限制，本批只补诚实契约，**没有实现修复**。应先用独立回归约束再改，不能把它们藏在注释验收结论中：

- TCP 的 `ClientConfig.bind_addr` 目前被连接器忽略。后续应支持显式本地绑定，或在无法支持时明确拒绝；回归需要检查实际本地地址/端口以及失败不能悄悄回退。
- TCP 和 UDP/KCP 驱动在安全转换期间仍可能取出下一条命令，控制器拒绝后没有上抛失败；因此命令入队成功不能保证最终发生转换，也可能与自动换钥竞争。后续应保留待执行命令或显式拒绝，覆盖连续 mode/rekey 请求、自动换钥竞争及背压。当前说明已明确此限制，不能当成已修复。
- 连续零容量 poll 每轮仍可能把一条普通公开事件移入 completed，而这些非 early-data 事件不占 early-data 预算；单次工作量有界不等于跨调用积压总量有界。现有测试只生成有限事件，不能证明持续输入下的内存上界。后续需要持续输入回归与 completed count/byte 背压；当前调用方必须使用正容量 poll 取走事件，不能长期只做零容量维护。

### 验证与边界

- 修改前 `cargo test --workspace` 基线通过；补注释和契约复查后完整 workspace、严格 Clippy、格式/结构、严格文档和 doctest 均通过。最后仅更正零容量积压的一处内部注释，再次通过严格 Clippy、格式/结构、严格文档和 doctest；源码运行逻辑未变。
- 构建/测试命令串行，使用运行中至少保留 15 GiB 可用内存的守护。没有重跑 C++/Go/C# 集成、Go race、ASan fuzz、Miri、Loom、完整打包、非 Linux 平台或目标环境长稳；没有把历史通过记录计入本批发布资格。
- 临时证据：`/tmp/netlib-stage4b-baseline.log`、`/tmp/netlib-stage4b-docs-missing.log`、`/tmp/netlib-stage4b-docs-green.log`、`/tmp/netlib-stage4b-final-validation.log`、`/tmp/netlib-stage4b-postreview.log`、`/tmp/netlib-stage4b-doc-final.log`。没有提交或推送。
- 测试盲区：缺失文档门禁只证明公开项有文档，不证明语义正确；现有协商/恢复/心跳/队列回归不能证明上述三个待办已解决。正向接入示例的全语言核对、C/C++ 永久注释门禁和全范围审查仍属于第 5/6 项。

整体评价：第 4 项注释范围已完成；后续先处理上述已确认的行为限制，再继续第 5/6 项收尾。目标平台与长稳仍留在功能和接口整体完善之后，不宣称生产认证完成。

## 第四阶段 C：行为限制修复（2026-10-16）

承接上一节三个已确认的待办。本批保留原有未提交的注释修改，不增加 wire/ABI 版本，也不变更密码学协议。本节记录当前进展，上一节的“没有实现修复”是当时的历史状态。

### 已实现

- TCP 显式本地绑定：在 connect 前使用现有 socket2 依赖创建并绑定 socket；未指定地址仍走原 OS 选址路径。IPv4/IPv6 地址族必须匹配，绑定失败直接返回，不悄悄换地址或端口。失败发生在 endpoint/session 注册之前，不留下假成功句柄。
- 安全命令保留：TCP 和 UDP/KCP 驱动仅在当前安全屏障完成后取下一条命令；自动换钥屏障采用相同规则。仍使用原有有界 mpsc，无新增暂存队列；满额继续返回 WouldBlock，断开或屏障超时取消排队命令。同模式请求仍可成为无新事件的 no-op，入队成功仍不代表远端已完成转换。
- 同步 ClientConfig.bind_addr、set_security_mode/rekey_session 的契约和关键路径原因注释。测试独立成文件，不把驱动回归堆进生产模块。

### TDD 与测试质量

- 修改前运行 transport 的 loopback/adaptive_security/secure_loopback 基线。TCP 三条回归先观察到真实 RED：显式端口被换掉、已占用端口和地址族不匹配仍被忽略；修复后三条通过，覆盖 IPv4/IPv6、真实 accept 观察到的来源、失败无 endpoint/session/pending-handshake 泄漏。随后完整 Rust workspace 通过。
- 安全命令四条回归先观察到真实 RED：暂不回复 ACK 时，驱动提前消耗下一条命令；修复后均通过，随后完整 Rust workspace 通过。
- 增补 TCP ACK 超时清理回归：有待执行命令时仍按截止时间关闭会话，接收端关闭且只有一次 Timeout 关闭事件，没有虚假 SecurityChanged。成功路径还核对实际业务报文内容和 epoch 2/3 的通知顺序。
- UDP/KCP 驱动回归覆盖手动换钥→切明文、切明文→换钥、自动换钥→切明文；确定性推进维护轮次，确认屏障中队列容量不被释放、结束后下一命令被执行。此项直接推进真实控制器的 ACK 状态，不声称它单独证明弱网重传；完整 wire/密码学往返由现有集成测试及新增 TCP 真实 Noise 往返覆盖。

### 尚未实施：零容量 poll 的累计积压

普通公开事件进入 completed 的 count/byte 总预算仍未修复，不能将单次扫描上限称为累计内存上限。调用方仍须持续使用正容量 poll 取走事件。

拟采用无静默丢失的背压：普通 completed 使用独立 count/byte 预算；满额停止从底层取更多事件，仍推进定时器，正容量 poll 释放预算后恢复。需要在底层按可用字节预算取 FIFO 前缀，避免先出队再发现无法存放；early-data 继续单独计费，并保留 READY/RESUMED 先于业务消息的顺序。长期不取事件时，队首业务仍可能阻挡其后的 ACK，因此不保证“零容量维护可以无限维持会话”。已向用户询问此策略和“满额关闭会话”的取舍，尚未收到选择；本批不擅自增加满额踢人策略。

### 本批对抗性复查

| 检查项 | 结果与边界 |
| --- | --- |
| 显式 bind 失败的资源回收和回退 | socket 在注册前创建，失败按 RAII 释放；测试检查实际来源和失败无连接/句柄，不只检查配置字段 |
| ACK 等待、自动换钥与有界命令队列 | 禁用过早取队列的分支，不扩大容量；成功顺序、WouldBlock 和超时清理均有回归 |
| 测试是否过度模拟 | TCP 使用真实套接字和 Noise；UDP/KCP 明确是驱动/控制器队列测试，不能替代目标弱网矩阵 |
| 本批完成范围 | 仅上述两个行为缺陷；普通 completed 积压、文档第 5/6 项和目标环境资格仍未闭环 |

### 最终验证与资格边界

- 当前修改树通过 Rust 格式、结构门禁、`make doc-check`（严格 rustdoc 与 doctest）、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 和 `git diff --check`。新增 8 个测试函数，包含表驱动的 IPv4/IPv6 与 UDP/KCP 场景。
- 当前 native 重新链接后的 C++11 所有权测试和四个联网示例、Go 全套及部分导出故障测试、C# TCP/UDP/KCP 和错误公钥/回调/并发销毁测试均通过。
- 高内存命令串行，并启用至少保留 15 GiB 可用内存的守护。未重跑独立 Go race、ASan fuzz、Miri、Loom、release 导出检查、完整打包或目标弱网长稳；仅验证当前 Linux 开发机，C# 为 .NET 8，不扩展平台或容量承诺。
- 临时证据：`/tmp/netlib-runtime-fixes-baseline.log`、`/tmp/netlib-tcp-bind-red.log`、`/tmp/netlib-tcp-bind-green.log`、`/tmp/netlib-tcp-bind-workspace.log`、`/tmp/netlib-security-commands-red.log`、`/tmp/netlib-security-commands-green.log`、`/tmp/netlib-security-commands-workspace.log`、`/tmp/netlib-security-commands-edges.log`、`/tmp/netlib-bind-security-final-validation.log`。

未提交或推送，不把本批修复称为生产资格认证；长稳仍放在功能与接口整体完善之后。

## 第四阶段 D：普通事件积压背压

用户已确认满额背压、要求持续正容量 poll，不采用满额直接踢人。沿用上一节方案：普通 completed 的数量与有效载荷字节分别受 event_queue_capacity/max_event_bytes 限制，独立于 early-data 预算；底层按剩余字节预算取 FIFO 前缀，不重排、不先出队再丢弃。零容量和满额仍驱动定时器；正容量取走积压后恢复。

此预算不是全进程内存上限，底层事件队列、early-data、普通 completed 各自计费，返回给调用方的事件不再属于库内积压。底层原有 UDP/KCP 每会话事件溢出关闭和优先生命周期事件策略不变，不承诺跨层绝对无丢包或不断线。满额时 ACK 也可能被挡在队首业务之后，正常心跳超时仍然生效。

- [x] 持续输入的 count/byte 积压与恢复回归先 RED。
- [x] core FIFO 字节受限提取、transport 保持原延迟采样。
- [x] game 普通积压计费，poll/close/stop 释放预算，READY 顺序不变。
- [x] 文档契约、完整回归与对抗性审查。

### 实施与对抗性复查

- 新增普通积压计数/载荷字节计费；每次取底层事件前查询剩余预算，不额外扩大 mpsc 或建立无限旁路。使用 poll 串行化保证查询容量到转换入队期间不会被另一 poll 消费；并发清理只会释放预算。
- core 新增按字节预算提取 FIFO 前缀，零字节仍可取空载荷事件、零容量不取任何事件，不越过超预算队首。transport 继续在实际出队时记录原有 EventQueue 延迟；对外 wire/C ABI 不变。
- 普通业务、认证凭据、恢复身份和票据均计费；early-data 仍独立保留自己的预留和既有指标口径。READY/RESUMED 插入到同会话早期消息之前的顺序不变。
- Rust/C/C++/Go/C# 的 poll 契约及接入文档同步，明确正容量消费、定时器继续运行、底层过载策略及 `range_buffer_snapshot()` 不含普通积压。

| 审查项 | 证据/处理 |
| --- | --- |
| 原测试只提供有限事件，不能证明累计上界 | 新增持续创建/关闭监听的数量测试；真实 TCP 发送三份 100 字节消息，256 字节暂存预算必须阻止第三份进入；两条均真实 RED 后 GREEN，排空后验证完整 FIFO 和继续接入 |
| 解码后载荷是否可能大于 native 预算 | 穷举计费所有 GameEvent 变体；恢复身份上限用编译期断言约束在原 ticket/key 载荷以内，转换处额外检查预算不变量，覆盖身份/双凭据计费 |
| 零载荷、精确边界、生产者预算、清理归还 | core 检查零值、精确字节边界、非匹配队首不被绕过、未取事件不释放生产者容量；game 检查 early/public 独立、READY 顺序、session 清理及 clear 归还预算 |
| 满额是否绕过生存检测 | 填满普通暂存后零容量 poll，验证已过期心跳和协商均被处理，原公开事件仍可取回 |
| 旧 UDP 单槽测试存在错误的恢复承诺 | 最终首轮回归出现 SessionClosed：既有 `try_push_session_event` 在队满时关闭会话；旧测试连续发 early/later 却要求同会话恢复，结果依赖时序。改为等待确定的溢出清理，检查 SessionOpened 未被数据或关闭通知覆盖、旧句柄失效、监听接受新连接且可收包；不修改生产溢出策略 |

新增 7 个测试函数，另修正 1 个已有时序性测试。满额定时器测试是确定性驱动检查，不等于目标环境弱网验证；真实 TCP 字节回归不宣称覆盖所有 UDP/KCP 过载排列。生产资格和第 5/6 项完整收尾仍是独立工作。

### 最终验证（2026-10-16）

- 修正旧 UDP 测试后，完整当前树再次通过 Rust workspace、严格 Clippy、格式/结构、严格 rustdoc/doctest；C++11 所有权测试和四个联网示例、Go 全套及部分导出故障回归、C# TCP/UDP/KCP 与错误公钥/回调/并发销毁测试均通过。最后仅更正内部注释的 ACK 优先表述与此记录，格式及 diff 检查再次通过。
- 过程记录：`/tmp/netlib-poll-backpressure-baseline.log`、`/tmp/netlib-poll-backpressure-red.log`、`/tmp/netlib-poll-backpressure-green.log`、`/tmp/netlib-poll-backpressure-workspace.log`、`/tmp/netlib-poll-backpressure-edges.log`。首轮最终检查中的 UDP 失败保留在 `/tmp/netlib-poll-backpressure-final-validation.log`；修正后针对性结果 `/tmp/netlib-poll-backpressure-udp-contract.log`，最终通过 `/tmp/netlib-poll-backpressure-verified.log`。
- 所有高内存命令串行并启用 15 GiB 可用内存守护。未重跑独立 Go race、ASan fuzz、Miri、Loom、release 导出检查或完整打包；未做目标弱网/长稳和其他平台资格测试。没有提交或推送。

本节完成上一节保留的普通 completed 积压待办。既有 UDP/KCP 原生事件队列溢出仍可能关闭会话，本节没有把该策略改成无限等待。长稳继续留在整体完善之后，不宣称生产认证完成。
