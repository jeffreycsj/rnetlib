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
5. [x] 接入文档与防回退：统一多语言建立、授权、持续 poll、发送、接收、背压、关闭和日志示例；将可验证的注释/文档构建门禁接入现有检查。第五阶段 A–F 完成五语言示例与存在性门禁，不代表第 6 项全范围语义复查或生产资格完成。
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

## 第五阶段 A：C/C++ 永久文档门禁（2026-10-19）

延续已确认的第 5 项，基线为干净的 `3356d60`。本批先补原生公开声明的文档防回退，不将完整多语言示例/文档生成及第 6 项全范围审查提前勾选完成。

- 使用已安装的 Clang 前端识别 C11/C++11 声明及绑定的文档注释，覆盖类型、字段、枚举常量、回调 typedef、公开/受保护方法（含构造、析构、删除重载）。不把类型注释当作成员文档，不把私有成员、局部 RAII 类型或隐式生成的方法误判为公开 API。
- 真实编译器驱动的删除注释、空注释、非法文档参数及编译失败测试先 RED，再实现检查；缺少编译器或无法解析 AST 应失败，不静默跳过。
- 接入 make doc-check 和 Linux CI；说明构建依赖、覆盖范围和局限。此门禁不验证注释含义，不检查宏或命名空间注释，也不替代人工参数/返回值契约审查。
- 不改变 wire、SDK ABI、默认配置或网络行为，不启动长稳，不提交或推送。

### 实施结果与对抗性复查

- 新增 `scripts/check_native_docs.py` 与 11 个真实 Clang 驱动的测试函数，接入 `make native-doc-check`、`make doc-check` 和 Linux CI。当前覆盖 C 486 个声明、C++ 290 个声明；计数包含 tag/typedef 等不同声明，不等同于独立 API 或导出符号数量。
- 删除字段、枚举、回调或重载文档，空注释、普通注释、非法 `@param`、编译错误和编译器不可用均会拒绝通过。命令行测试还验证缺少头文件清单时返回失败；不把零覆盖当成功。
- 需求/范围复查：仅增加文档存在性及编译器文档语法检查，不改变协议和运行时；宏、命名空间摘要、语义准确性仍需人工核对。按本机预处理条件检查，不声称覆盖所有平台条件分支。
- 实现/边界复查：重复命名空间、依赖头文件、访问控制及局部类型均有测试。发现 C 依赖头文件缺注释时错误归属到入口文件，先用位置断言复现失败，再修正 Clang 省略文件名时的来源跟踪；C++ 依赖按各自头文件单独检查。新增未支持的公开模板声明会失败，不能悄悄漏检。
- 测试/成本复查：测试实际删除绑定的注释并调用编译器，不用手造 AST 代替前端；增加命令行成功/失败回归。C++ AST 限定命名空间以避免保留标准库大树，每次编译设置 60 秒超时，头文件串行检查。
- 文档/交付复查：明确 Python/Clang 依赖和工具覆盖参数，纠正 README 的语言示例覆盖说明；不声称已生成 C/C++ HTML 文档或完成整个第 5/6 项。

### 本批验证与边界

- 基线 C++11 回归通过；检查器先 RED 后 GREEN，依赖文件诊断另有独立 RED → GREEN。最终 `make -j1 native-doc-check doc-check cpp-test structure-check` 通过：11 个检查器测试、全部当前公开头文件、六个 Rust crate 的严格 rustdoc/doctest、C++11 所有权测试及四个联网示例。
- 临时证据：`/tmp/netlib-native-docs-baseline.log`、`/tmp/netlib-native-docs-red.log`、`/tmp/netlib-native-docs-green.log`、`/tmp/netlib-native-docs-review-red.log`、`/tmp/netlib-native-docs-final.log`。这是本机修改树结果；CI 已接入但没有远端运行结果。
- 高内存命令串行并使用 15 GiB 可用内存守护。本批没有 Rust/网络实现改动，未重跑完整 workspace、Clippy、Go/C#、Go race、ASan fuzz、Miri、Loom、ABI 发布包或目标平台弱网/长稳；不继承历史资格为当前发布认证。未提交或推送。

## 第五阶段 B：C11 完整游戏接入示例（2026-10-19）

延续已确认的多语言接入文档任务，先补 C 的可运行路径。现有 C 文档在 poll 错误时直接返回，未释放已导出前缀；本批单独建立真实 native 缓冲区所有权回归后修正示例，不更改 SDK ABI 或网络实现。保留本轮开始时尚未提交的第五阶段 A 改动。

- [x] 演示两个 runtime 的身份固定、建立、授权、就绪、双向二进制/空消息收发、应用有界重试、日志注入、显式关闭及 stop/destroy；命令行选择 TCP/UDP/KCP，拒绝授权作为错误场景。
- [x] poll 或业务处理失败仍释放全部已导出主/辅助 token，只处理 out_count 前缀；保留最初错误，避免清理覆盖诊断。重试必须保留 payload 副本，继续正容量 poll，受整体截止时间限制。
- [x] C 示例及所有权失败测试接入 make/CI，发布脚本收录源码；文档以可编译源码为准，明确 UDP 不保证送达、演示票据不用于生产和 SDK/业务版本的区别。

### 对抗性复查

| 检查 | 证据与边界 |
| --- | --- |
| 原 C 文档失败时直接返回，泄漏已导出的主/辅助缓冲区 | 从原处理方式建立两条 RED（poll 失败、handler 失败），真实 native destroy 因遗留 token 拒绝；改为保存首个错误、停止业务分派但释放整个前缀，GREEN |
| 清理不能越界或覆盖诊断 | 额外导出未计入 out_count 的测试自有 token，确认示例没有释放；验证原错误字符串、首个/后段 handler 失败、零导出前缀及关闭时丢弃模式，共 6 个所有权场景 |
| 不把成功入队当送达、不因零字节消息误判为空队列 | TCP/UDP/KCP 各做真实二进制及空消息双向回显；客户端校验字节与长度，成功标记仅在双端销毁成功后打印 |
| 背压不是无限等待或持有悬空接收指针 | 链接拦截让双方前两次发送拒绝，然后走真实 native 发送；每端持有 128 字节单槽副本，持续 poll，验证完整回显。持续拒绝场景验证十秒截止和清理；不冒充真实高负载容量测试 |
| 异常、日志、销毁边界 | 三传输授权拒绝须返回非零及 -13，而非等到超时；未知传输/参数必须拒绝。回调不输出业务或票据、不停止 runtime；所有 setup 分支汇聚清理，私钥副本擦除。stderr 仅为演示，生产需快速日志组件 |
| 文档与交付范围 | 修正 C 失败路径片段，提供源码和运行命令；统一五语言入口但明确 Go/C# 是回归程序，其他语言的完整背压示例仍未收尾。删除指南中残留的独立 range 入口表述，不改变实际 SDK |

本批不是全部五种语言契约复查完成；第 5/6 项仍未整体勾选，不启动长稳，不提交或推送。

### 本批验证

- 基线 `cargo test -p rnet-ffi --test game_ffi` 的 9 条测试通过。最终 `make -j1 c-test doc-check cpp-test structure-check` 通过；收紧授权失败断言后再次完整运行 `make c-test` 通过（6 个所有权场景、5 个端到端测试函数及其各传输子场景）。Rust 格式、`git diff --check` 和发布脚本 `bash -n` 通过。
- RED 证据为 `/tmp/netlib-c-poll-red.log`、`/tmp/netlib-c-handler-red.log`，均为真实 native destroy 检出未释放缓冲区，并非编译错误。基线与最终结果分别见 `/tmp/netlib-c-quickstart-baseline.log`、`/tmp/netlib-c-quickstart-final.log`、`/tmp/netlib-c-quickstart-verified.log`；临时日志不等于发布资格。
- 本机 Linux C11 验证，重命名/分文件仅涉及示例；未修改运行库、头文件 ABI 或 wire。构建串行并有 15 GiB 可用内存守护。没有重跑完整 Rust workspace/Clippy、Go/C#、Go race、ASan/Miri/Loom、目标弱网或长稳，也未重新生成发布包；CI 与发布清单只是接入，没有远端 CI/完整打包结果。

## 第五阶段 C：Rust 示例背压与生命周期（2026-10-21）

继续已确认的第 5 项，保留未提交的 C 示例和文档门禁。本轮只完善 Rust 可执行示例，不改运行库、wire 或 ABI。

- [x] 有界应用待发副本：单端最多两条、每条最多 128 字节；WouldBlock 保留原消息、停止本轮重试并继续双端正容量 poll；终止错误保留原因，截止时间前未完成则失败。
- [x] 同一示例运行 TCP/UDP/KCP，支持演示授权拒绝和单消息发送预算；真实回显验证二进制与空消息，而非只确认 send 返回成功。
- [x] 将失败事件、日志注入、显式关闭以及双端 stop 汇入完整生命周期；不能以最终超时掩盖 JoinFailed/ProtocolRejected，也不能在一个 stop 失败后跳过另一端。
- [x] 按 TDD 先验证重试队列的丢失/过期边界，再运行真实网络回归；继续使用同一源文件生成 rustdoc，示例测试纳入 workspace。完整多语言及长稳资格不在本批完成声明内。

### 对抗性复查

| 检查 | 结果与边界 |
| --- | --- |
| 原示例 send 返回 WouldBlock 就退出 | 应用待发队列提取后的 RED 复现背压退出、过期仍调用 send、终止错误过早出队；GREEN 改为仅在成功入队后删除，阻塞时返回外层 poll，保留副本 |
| 不能只用假发送闭包证明网络恢复 | 闭包检查确定性的 FIFO/复制/边界；另在 TCP/UDP/KCP 各跑默认预算与真实单消息预算，要求单槽模式实际出现阻塞并最终收到两条精确回显 |
| 部分入队和空消息可能被重复/遗漏 | 二进制含 NUL/非 UTF-8 字节；零长度单独占消息数。测试先接受第一条再拒绝第二条，下一轮不得重发第一条；128/129 字节和两条/三条计数边界均验证 |
| 掩盖连接失败或清理失败 | 三传输拒绝授权均保留 AuthRejected；错误事件转换检查握手、协议、关闭及端点类别。先执行双端 stop 再选返回错误，保留原始失败且输出清理失败；成功路径不忽略 stop 错误 |
| 示例的保守策略不能变成库承诺 | 单会话、最多两条应用待发，不是多人队列；遇 EndpointError 一律终止是演示策略，库允许部分端点错误恢复。UDP 丢失/重复/乱序可使严格回显失败，不额外承诺可靠性 |
| 文档、日志和测试入口 | 错误日志只用库诊断，示例不打印业务载荷/票据；stderr 与固定票据仅作演示。明确默认 30 秒摘要不会必然出现在短回显中，示例测试通过 Cargo 的 test=true 纳入 workspace |

仅完成 Rust 示例子项。C++11/Go/C# 的完整有界重试示例、跨语言契约复查及目标发布资格仍待后续；不启动长稳，不提交或推送。

### 本批验证

- 现有 game doctest 基线通过。新增队列测试首次为 3 失败/1 通过，失败分别是背压直接返回错误、过期仍发送和错误提前出队；最小修复后 4 条全部通过，再接入完整示例与其他回归，最终 11 个示例测试通过。
- 当前树通过 `cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`make -j1 doc-check structure-check`、Rust 格式及 diff 检查。workspace 日志确认实际执行了示例测试目标，而非仅编译。README 的 TCP 单槽命令实际返回两条成功回显并记录一次背压；KCP `--deny` 实际以 AuthRejected/退出码 1 结束，没有成功标记或清理错误。
- 本机临时证据：`/tmp/netlib-rust-example-baseline.log`、`/tmp/netlib-rust-example-red.log`、`/tmp/netlib-rust-example-green.log`、`/tmp/netlib-rust-example-integration.log`、`/tmp/netlib-rust-example-doctest.log`、`/tmp/netlib-rust-example-final.log`、`/tmp/netlib-rust-example-cli.log`、`/tmp/netlib-rust-example-denied.log`。
- 所有高内存命令串行并受 15 GiB 可用内存守护。没有重跑 C/C++/Go/C# 外部接入测试、Go race、ASan/Miri/Loom、完整发布包或目标弱网/长稳；Rust workspace 中的短 soak smoke 不等于长稳。CI 配置没有远端执行证据，不将本机修改树认证为生产版本。

## 第五阶段 D：C++11 有界重试接入示例（2026-10-21）

沿用已确认的第 5 项，保留全部既有未提交修改。不改变 SDK、wire 或运行库；已有恢复/版本协商 smoke 不删除，另加易读的双 runtime 示例。

- 应用待发副本每端最多两条、每条最多 128 字节；只在成功发送后移除，WouldBlock 回到正容量 poll，不重复已接受前缀。单调截止时间防止无期限重试。
- TCP/UDP/KCP 同一入口选择类型，使用真实单消息 scheduler 预算验证恢复；覆盖二进制、空消息、授权拒绝和错误公钥。
- 日志回调不抛跨 C 边界、不持有载荷、不停止 runtime；显式关闭双端，即使第一端或主流程抛异常也继续第二端，保留主错误和清理诊断。
- TDD 先验证复制、FIFO、过期、条数/字节及异常；示例与测试接入 cpp-test/CI，发布清单包含依赖头文件。Go/C# 与全范围契约复查仍是后续，长稳不提前做。

### 实施与对抗性复查

- 新增 `examples/cpp/game_quickstart.cpp`（双端建立/授权/收发/日志/关闭）和 `game_quickstart_support.hpp`（有界应用重试与异常汇合），保持职责分离，不向 SDK 添加接口。原有四个联网 smoke 与所有权回归均保留。
- 队列初版 RED 明确复现背压抛出、过期仍发送、终止错误提前出队；GREEN 仅在成功入队后删除。7 个 C++ 边界场景覆盖复制、二进制/空消息、128/129 字节、两条/三条、部分接受不重发前缀、bad_alloc 传播、双端关闭及原始异常保存。发送回调只作为外部边界注入；真实队列背压另行联网验证。
- 4 个 Python CLI 测试函数覆盖三传输默认/单槽真实预算、拒绝授权、错误公钥、默认入口及非法参数。单槽测试必须观察到 blocked > 0 并验证两个精确回显，不用 send 返回成功替代送达判断。
- 首轮错误公钥 TCP 场景的断言过严：双 runtime 中服务器可能先观察到握手 EOF（IoError），尚未 poll 客户端的具体公钥错误。保留这个真实的首个本地原因，测试只在明确握手 EOF 日志下接受 -7，否则要求握手/公钥/加密拒绝；所有分支都要求没有业务授权/就绪且非超时退出。不是把任意网络错误算作信任校验通过；单端精确错误语义仍由已有库测试覆盖。
- 资源与异常审查：客户端创建失败也进入服务端显式清理；第一端清理抛出后仍尝试第二端，primary 通过 exception_ptr 保存原异常类别/内容。callback 不抛跨 C 边界、不持有借用字节、不关闭 runtime；敏感业务数据不由示例日志输出。
- 文档说明保守端点错误策略、UDP 不可靠、双消息演示队列范围、非抢占式截止时间以及默认 30 秒 P95/P99 摘要。发布清单包含两个源码依赖；该路径未重新完整打包，不提前声称发布验证完成。

### 验证与资格边界

- 修改前 `make -j1 cpp-test` 基线通过。最终 `make -j1 cpp-test doc-check structure-check` 通过，包含旧 C++ 回归、新边界/CLI 检查、原生文档门禁、六个 Rust crate 严格 rustdoc/doctest。Rust 格式、`git diff --check`、发布脚本 `bash -n` 通过；最终仅补充重试回调不可重入注释和本记录。
- 临时证据：`/tmp/netlib-cpp-quickstart-baseline.log`、`/tmp/netlib-cpp-quickstart-red.log`、`/tmp/netlib-cpp-quickstart-green.log`、`/tmp/netlib-cpp-quickstart-integration.log`（保留首轮错误公钥断言失败）、`/tmp/netlib-cpp-quickstart-final.log`（最终通过）。
- 仅本机 Linux C++11；构建串行、使用 15 GiB 可用内存守护。没有重跑完整 Rust workspace/Clippy、C/Go/C# 接入、Go race、ASan/Miri/Loom、独立依赖审计、远端 CI 或完整发布包，更未做目标弱网/长稳。保留已有修改，未提交或推送；整体第 5/6 项和生产资格仍未完成。

## 第五阶段 E：Go 有界重试接入示例（2026-10-21）

延续已确认的第 5 项，保留前序全部未提交变更。本批不修改 SDK、ABI、wire 或默认安全策略，仅补全 Go 游戏接入示例、测试及交付入口。

- `examples/go/game_quickstart` 按命令行、runtime 建立/收发/关闭、应用重试队列分为三个实现文件。演示两个独立 runtime、固定服务端公钥、授权、游戏就绪、二进制/空消息回显、日志注入与显式关闭。
- 每端至多两条、每条至多 128 字节的应用副本；只有成功 admission 才移除消息，WouldBlock 保留消息并回到双端正容量 poll。十秒交换截止时间限制重试尝试，不承诺抢占 native 调用或阻塞日志回调。
- TCP/UDP/KCP 共用 API；`--one-slot` 限制真实 native 游戏 scheduler 为一条消息，各传输测试要求确实出现背压并完成回显。`--deny` 必须返回 AuthRejected，而不是静默成功或超时。
- `errors.Join` 保留原始与清理错误，双端关闭均尝试；每端清理有独立五秒背压重试期限。示例在收到全部回显后主动关闭 session 和 listener，最终成功信息仅在 runtime 清理成功后打印。
- 更新 README、游戏指南和 getting-started 的 Go 入口，移除忽略错误且未展示收发的旧片段。`make go-test`、CI/race 和发布清单包含示例；CI 增加从发布包目录编译运行 Go 源码的检查。

### 对抗性复查

| 检查 | 结果与边界 |
| --- | --- |
| 需求与接口 | 无新增 SDK 包类型或 transport 参数；建立时选择传输，发送只有 session/payload。始终等待 GameSessionReady，示例保持加密；策略切换、恢复、协商的既有 SDK 测试保留 |
| 测试质量 | 先建立可编译接口骨架，队列/清理四项 RED 因能力未实现而失败，GREEN 后再做真实三传输与命令行 RED → GREEN；不是编译错误，也不声称骨架是旧 SDK 缺陷。最终八个测试函数及其子场景覆盖复制、二进制/空值、128/129 字节、两条/三条、部分 admission 不重发、过期、终止错误、双端清理和事件类别 |
| 实际背压与送达 | 闭包仅替代外部发送边界验证确定性 FIFO/失败行为；另在真实 TCP/UDP/KCP 的默认/单槽预算下验证两条字节匹配回显和 blocked > 0。UDP 丢失、重复、乱序可使严格示例失败，不额外承诺可靠性或容量 |
| 资源与并发 | logger 使用同一个 log.Logger 串行化双 runtime 原生回调；无异步应用发送 goroutine。首端创建后即登记双端清理，第二端创建失败也不会漏清理。回调不 Stop/Close；Go 侧清理及日志并发由 race 回归验证，但 race 不检测 Rust/C 内部的数据竞争 |
| 安全与可追溯性 | 错误保留 native 类别、endpoint/session 与库日志；ProtocolViolation 的 status=0 不作为成功。示例不输出凭证或业务载荷；明确 Go 值/编译器复制意味着擦除自有私钥缓冲区不等于保证擦净所有副本。30 秒 P95/P99 摘要通常不会在短回显中出现 |
| 交付与文档 | 独立 Go 包布局首次编译因本机 /tmp 上层不可用的 .git 占位导致自动 VCS 标记失败；不改该目录，将源码包命令/CI 明确为 -buildvcs=false，重跑通过。避免源码包依赖外部 checkout 元数据；完整发布包与远端 CI 尚未执行 |

本轮未发现需要修改运行库的高置信新增缺陷。测试盲区：没有强制触发真实 Go/native OOM、Close 持续背压或永不返回的日志组件；双端清理错误通过外部 closer 边界测试，不冒充全部原生关闭故障覆盖。示例队列只是单会话演示，生产需要自行定义每玩家预算和过载策略。

### 验证与资格边界

- 修改前 `make -j1 go-test` 通过。最终 `make -j1 go-test` 通过，包含现有 SDK、八个示例测试函数及子场景、可执行文件构建和部分导出失败时的原生 token 释放回归。
- `go test -a -p 1 -count=1 -race ./go/rnet ./examples/go/game_quickstart` 通过；随后仅补内部契约注释、包内编译参数和记录，示例普通测试再次通过。
- `make -j1 doc-check structure-check` 通过；包含原生注释门禁、六个 Rust crate 严格文档与 doctest。独立临时 Go 发布布局（当前 debug archive，不是完整 release 包）编译成功，实际进程验证 TCP 单槽、UDP/KCP、拒绝授权退出 1、非法参数退出 2。Go 格式、diff 和发布脚本 bash 语法检查通过。
- 临时证据：`/tmp/netlib-go-quickstart-baseline.log`、`/tmp/netlib-go-quickstart-red.log`、`/tmp/netlib-go-quickstart-green.log`、`/tmp/netlib-go-quickstart-exchange-red.log`、`/tmp/netlib-go-quickstart-integration.log`、`/tmp/netlib-go-quickstart-cli-red.log`、`/tmp/netlib-go-quickstart-final.log`、`/tmp/netlib-go-quickstart-race.log`、`/tmp/netlib-go-quickstart-postreview.log`（末尾保留首次包布局 VCS 失败）、`/tmp/netlib-go-quickstart-package.log`（包布局最终通过）。
- 构建串行并保留至少 15 GiB 可用内存；本机 Linux/Go 1.22.2。不重跑 Rust 全 workspace/Clippy、C/C++/C# 接入、ASan/Miri/Loom、独立审计或完整 release 包；不将文档 doctest 或回环回显当作长稳。没有提交/推送，仍需完成 C# 完整示例、第 5/6 项整体契约收尾和最终目标发布资格。

## 第五阶段 F：C# 有界重试接入示例（2026-10-21）

沿用已确认的第 5 项，保留全部既有修改，不改 SDK、ABI、wire 或默认安全策略。五语言完整接入示例与文档门禁已分别交付，第 6 项全范围契约复查仍未结束。

- 新增 `csharp/RNet.Quickstart`，分为命令行、双 runtime 游戏流程、应用待发队列/清理异常汇合三个实现文件。覆盖固定公钥、授权、SessionReady、二进制/空消息、正容量 poll、有界背压、日志、CloseSession/CloseEndpoint 和显式 Stop/Dispose。
- 每端至多两条、每条至多 128 字节的应用副本；空消息也计数，成功 admission 才出队，终止错误/分配异常不提前移除，WouldBlock 回到 poll。单调 Stopwatch 限制交换重试为十秒、各端 Stop 重试为五秒，不承诺抢占 native 调用、Dispose 或阻塞日志。
- 保留原有 `RNet.Smoke` 全部回归，新增无外部 NuGet 测试依赖的四类队列/清理场景和三个 Python CLI 测试函数。CLI 覆盖 TCP/UDP/KCP 默认/真实单槽预算、拒绝授权、默认入口及非法/重复参数；单槽须观察到 blocked > 0 且两个字节匹配回显，拒绝授权须是 -13/退出 1，非法参数退出 2。
- 补充关键字段和接口约束注释，替换 C# 指南中遇背压直接退出、没有显式 Stop 的旧片段；更新 README 和多语言入口。Make/CI/发布清单包含示例及依赖源码，CI 增加包内引用同批 DLL 的运行检查。

### 对抗性复查

| 检查 | 结果与边界 |
| --- | --- |
| 契约与可用性 | 同一 Send/Poll 用于三传输，建立时选择类型，AuthDecide 后仍等待游戏就绪；示例保持加密，原有动态策略/恢复/协商 SDK 测试保留 |
| TDD 与测试质量 | 可编译骨架阶段四类契约测试因缺少实现失败，最小 GREEN 后完成队列；CLI 骨架再以真实进程退出/输出验证 RED，完整流程 GREEN。不是编译错误或 SDK 旧缺陷。发送闭包只隔离外部 admission 边界，真实背压由联网单槽测试另验 |
| 错误与关闭 | 单一异常保留对象/原堆栈，多异常 AggregateException 保留主错误在前及两端清理原因；第一端失败仍尝试另一端。明确 SafeHandle Dispose 不抛内部 native 清理错误，成功标记仅声称完成 Stop/Dispose 序列，不声称已验证原生 destroy 成功 |
| 所有权与并发 | Keypair 作用域释放，登录数组在 Connect 后清零，授权事件用后清零；应用无并发调用者，双 native 日志回调经 Console.Error 串行写，不引用短生命周期外部资源，也不关闭 runtime。SDK 捕获日志异常不等于保证日志不丢失 |
| 安全/诊断 | 生命周期错误保留 native code 和 endpoint/session；ProtocolViolation 的零 status 不被当作成功。日志带时间/runtime/endpoint/session/correlation，不打印凭证或业务载荷。默认摘要间隔 30 秒，不能要求短回显一定输出 P95/P99 |
| 发布与平台 | 在没有 SDK 项目源码的独立临时目录，仅以示例源码、RNet.dll/XML 和当前 debug native 库构建并运行全部 CLI 用例，验证 RNetDll 引用和 native 搜索路径。不是完整 release 打包/跨平台验证，也不扩大 Unity/IL2CPP/AOT 支持声明 |

测试盲区：未强制耗尽真实托管/native 内存，OutOfMemoryException 仅是发送边界异常注入；未注入持久 Stop 背压、SafeHandle destroy 失败或永不返回的日志组件。异常汇合测试不替代全部 native 故障测试。UDP 丢失/重复/乱序可使严格回显失败，未增加可靠 UDP 或多人队列承诺。本轮未发现需要修改运行库的高置信新增缺陷。

### 验证与资格边界

- 修改前 `make -j1 csharp-test` 基线通过；最终 `make -j1 csharp-test doc-check structure-check` 通过，包含原 smoke、四类托管边界测试、三个真实 CLI 测试函数及子场景、原生注释门禁、六个 Rust crate 严格文档和 doctest。C# warnings-as-errors 构建零警告/零错误。
- 独立 C# 包布局构建及全部 CLI 回归通过；最终 diff 和发布脚本 `bash -n` 检查通过。本批仅补示例/测试/文档/构建入口，没有 SDK 声明或布局变更。
- 临时证据：`/tmp/netlib-csharp-quickstart-baseline.log`、`/tmp/netlib-csharp-quickstart-red.log`、`/tmp/netlib-csharp-quickstart-green.log`、`/tmp/netlib-csharp-quickstart-cli-red.log`、`/tmp/netlib-csharp-quickstart-integration.log`、`/tmp/netlib-csharp-quickstart-final.log`、`/tmp/netlib-csharp-quickstart-package.log`。
- 构建串行，使用至少保留 15 GiB 可用内存的守护；仅本机 Linux x86_64/.NET 8。未重跑 Rust workspace/Clippy、C/C++/Go 接入或 Go race、ASan/Miri/Loom、完整 release 打包、独立审计或远端 CI；未做目标环境弱网与长稳。保留修改，未提交/推送；后续先完成第 6 项跨语言契约复查，再进行干净发布树与目标环境资格验证。

## 第六阶段 A：跨语言核心契约抽查与 Go 时钟转换（2026-10-23）

本批按已确认的第 6 项继续，保留前序未提交修改。结合代码/API 契约与对抗性审查，先横向核对五语言的建连就绪、poll 所有权、时间表示、错误和关闭路径；本地 SDK 不套用 HTTP 资源命名/分页规则，也不新增 RPC 或兼容层。没有把此批抽查视为全部状态路径/平台审查完成。

### 发现与处理

| 级别 | 证据/影响 | 处理 |
| --- | --- | --- |
| 必须修复：数值正确性（中） | Go ClockSyncSnapshot 将 native int64 微秒直接乘成纳秒 Duration；RED 中最大正值变成 -1µs、最小负值变成 0，刚越界的正负值反号。Rust/C/C++/C# 保留 int64 微秒，不存在此纳秒转换 | 先在实际 Go 查询入口做仅测试标签启用的 ABI 边界注入，确认四个越界失败与五个正常/边界成功；再于乘法前按符号限幅到可表示的整微秒边界，沿用已有无符号 RTT 转换的限幅原则。不取绝对值，避免 MinInt64 再次溢出 |
| 建议修订：前置条件说明 | Rust clock_sync_snapshot 先 ensure_game_ready；C/C++/Go/C# 的“无样本”说明容易被理解为未就绪也返回 absent | 补充 game-ready 要求，pending/stale 是错误，不是 None/available=false/null；不改实际错误码或状态机 |
| 建议修订：语言差异可见性 | 五语言的单位、收包所有权、expiry 禁用值和关闭错误暴露不完全相同，不能只说“同一 API” | getting-started 增加逐语言契约表；明确 Rust Some(0) 立即过期而 C 系列 expiry_ms=0 禁用，Go 正期限向上取整到毫秒；明确 C# Dispose 与 C++ 析构不等于 checked destroy |

### 抽查路径与不变量

- Rust GameRuntime → C game FFI → C++/Go/C# 包装：endpoint/auth accept 不等于游戏就绪；句柄归所属 runtime，发送只用 session/opaque payload，成功只代表本地 admission。业务版本范围和 SDK/wire 版本分开，本批无接口、布局、枚举或 wire 变更。
- Poll：Rust 返回拥有的 Bytes；C 导出字节由 token 持有，失败也释放 out_count 前缀主/辅助 token；其余包装层复制后释放。C++ 作用域守卫、Go defer、C# finally 都覆盖已导出前缀，不承诺失败事件重放。Go 容量上限 65536、C# 上限 4096，未擅自统一为新的限制。
- 零容量 poll：只做有界维护；公共积压满额暂停后续传输读取，定时器仍推进，长期零容量不能保证心跳存活。文档保留用户已选的持续正容量 poll 背压契约。
- 时钟：native signed microseconds 与 Go signed nanoseconds 范围不同；新增限幅保持符号，不把巨大偏移变成貌似正常的零/负微秒。无样本仍与零偏移样本区分，未知句柄仍报错。时钟控制受认证不等于对端时钟可信，不修改 Rust 的四时间戳估计公式。
- 关闭与日志：C/Go/C++ 显式错误路径可观察 stop/destroy；C# 显式 Stop 可观察错误，SafeHandle Dispose 内部失败不通过 RNetException 暴露，在途 lease 可延后清理。所有语言均要求退出并发调用者后清理、回调不 Stop/destroy。此已知 C# 可观测性边界未被示例成功标记掩盖，也未在本轮改造为新 API。

### 测试质量与剩余边界

- 新 hook 仅在 Linux 且显式 rnet_test_faults 标签启用，针对指定 runtime 一次性注入 native 返回结构，其他调用走真实库；测试调用公开 ClockSyncSnapshot，不仅测试一份重新实现的转换公式。普通 go list 确认两个故障注入模块都在 IgnoredGoFiles。
- 覆盖正负普通偏移、零样本、可表示上下界、上下界外一微秒、int64 极值、无样本、无效真实 session 错误，以及 RTT/样本数字段未被改坏；测试不依赖等待数百年的真实时钟，也不冒充弱网时钟精度验证。
- make go-test 的故障标签路径改跑整个 Go SDK 测试，避免新测试被旧的单函数过滤漏掉；CI 增加故障标签 race。无外部测试服务、无真实 OOM 或网络洪泛。
- 第 6 项仍未整体勾选：仍需更完整的生命周期失败/并发处置和跨语言高级接口对照；C# 原生清理失败可观测性、目标平台资格与最终干净发布树验证仍须明确处理。未开始长稳、未提交/推送，不将本批标为生产认证。

### 本批验证

- 修改前 `make -j1 go-test` 通过。新增时钟回归 RED 明确是四个极值转换错误，非编译失败；修复后整个故障标签 Go SDK 回归通过。
- 最终 `make -j1 c-test cpp-test go-test csharp-test doc-check structure-check abi-check` 全部通过。包含原有多语言集成与五语言示例相关检查、Go 所有权/极值故障测试、C# smoke/边界/CLI、C/C++ 注释门禁、Rust 严格文档/doctest；ABI 检查重新构建 release rnet-ffi 并核对导出。
- `go test -a -p 1 -count=1 -race -tags rnet_test_faults ./go/rnet ./examples/go/game_quickstart` 通过。该结果覆盖 Go 侧并发检查，不是对 Rust/C 内部的线程检测。Go 格式、`git diff --check`、发布脚本 `bash -n` 通过；审查后没有遗留本轮已确认的行为缺陷。
- 临时证据：`/tmp/netlib-contract-audit-baseline.log`、`/tmp/netlib-clock-duration-red.log`、`/tmp/netlib-clock-duration-green.log`、`/tmp/netlib-contract-audit-final.log`、`/tmp/netlib-contract-audit-race.log`。均为当前脏工作树的本机证据，不转记为干净发布提交资格。
- 构建串行并使用 15 GiB 可用内存守护；没有重跑 Rust workspace/严格 Clippy、ASan/Miri/Loom、独立审计、完整发布包/远端 CI 或目标环境长稳。已有跨阶段未提交修改保留；后续获授权提交时宜按阶段组织，不把巨大累计 diff 当作单一小修复。
