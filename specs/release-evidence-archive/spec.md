# 人工批准的 Release 证据归档

状态：2026-11-24，离线归档、GitHub 来源核验和受审批发布的代码及本地回归已实现。
工作流默认关闭；管理员配置和隔离仓库演练仍未完成，未变更远端权限、tag 或 Release。

## 目标与边界

将同一干净提交、同一次完整资格运行的证据和候选 SDK 保存为 GitHub Release
附件，不再把会过期的 Actions artifact 当长期记录。归档不等于生产认证；目标环境
弱网、最终长稳和独立审计仍单独验收。本功能不创建 runner，不运行生产部署。

## 行为规格

- 当维护者手动输入资格运行 ID、attempt、完整 commit SHA 和既有 release tag 时，
  系统只接受本仓库受信 `release-qualification` 手动运行产生的成功结果。
- 当全部现行资格门禁通过、源码干净且日志、环境、SDK、manifest 和源码摘要一致时，
  系统形成只读验证报告；缺失、过期、跳过、混合提交或摘要不匹配必须失败。
- 当 tag 不存在或解析到其他提交时，应失败，不自动创建或移动 tag。
- 当指定审核者尚未批准时，不创建 Release 或上传附件；手动触发不等同审批。
- 批准后复核输入及摘要，创建 draft，上传完整附件并核对远端摘要，最后发布为
  candidate/prerelease，说明仍未完成的生产资格项。失败保留 draft 供排查，不公开
  半套资产，不覆盖、删除或复用其他 Release 的同名附件。
- 同一 tag 的发布串行。重试只接受同一运行身份及完全相同内容；存在冲突时失败。

## 技术设计

新增手动归档工作流，分为只读验证和受审批发布两个 job。只读 job 使用
`contents: read` / `actions: read`；仅发布 job 获得 `contents: write`，绑定预先配置的
`rnet-release-archive` environment。必须配置 required reviewers、禁止发起者自批、
限制可发布 ref；未核实保护规则有效之前，不启用自动发布。

下载后只把 artifact 当数据处理：使用受信工作流版本的校验器，不执行下载内容中的
脚本或 SDK；限制解压路径、大小及文件类型。复用现有证据与包校验规则，增加离线
bundle 校验入口，不能通过改写历史 report 让它符合当前校验器。

附件至少包括原始 SDK archive 及其 checksum、完整 evidence bundle 及其 checksum、
archive manifest。manifest 关联仓库、tag、完整 SHA、资格 workflow/run/attempt、
source digest、lockfile hash、SDK/bundle SHA-256、验证器版本及归档工作流 URL。
bundle 保留 summary、逐门禁 report、command log 和 environment 记录；不采集完整
环境变量或凭据。发布前检查日志泄密风险。

Release 附件按项目政策长期保留；不设自动删除任务。Release 仍不是不可删除的独立
备份，不能承诺永久不可篡改。immutable releases 或离站备份作为后续单独配置，不在
本次自动启用。artifact 到期前未归档则必须重跑资格，不得补造证据。

## 外部前置条件

仓库管理员需确认仓库套餐支持所需审批保护、指定审核人、配置 environment 和受保护
tag。required-reviewer 可用性依套餐及仓库可见性而异，不能仅写一个 `environment:`
字段便认为人工审批已强制执行。参见
[GitHub environment 保护规则](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments)。
采用 draft 完整上传后发布的顺序，亦符合
[GitHub Release 管理流程](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository)。

## 实现与验收

- [x] 确认本设计；不将方案确认当作实际发布授权。
- [ ] 仓库管理员确认审核人、environment 和受保护 tag 配置。
- [x] TDD 实现离线归档校验：混合运行、脏源码、缺失 gate、日志篡改、声明的 tag SHA
      不一致、非法解压路径、重复文件和资源跳过都拒绝。实际远端 tag 留待来源核验。
- [x] 实现 `scripts/release_archive.py prepare/verify`，原始证据不改写，附件可移机复核；
      可重复生成且不覆盖已有输出，manifest 不宣称来源认证或生产认证。
- [x] 离线阶段对抗性回归：JSON 非有限数/深度、gzip CRC、伪造认证字段及外层摘要
      重算后的 SDK 内部损坏。后续流程仍需完整审查。
- [x] 实现 GitHub workflow/run/attempt、仓库、tag、artifact 来源与审批保护的在线核验逻辑；使用合成托管响应做回归，实际运行验收见下项。
- [x] 实现只读验证、人工审批、最小权限发布以及冲突安全重试（默认禁用，未实际发布）。
- [ ] 用隔离测试仓库验证：未批准/拒绝均无 Release 写入，上传失败只有 draft，
      下载附件可独立复核并追溯原始运行。
- [ ] 对抗性审查通过，文档记录实际配置和演练结果后再启用。

## 离线阶段交付边界

现有 9 项门禁的清单和命令来自受信校验器，不能由 bundle 自行缩减。重新核对原始
summary、每项 report、日志与工具链文件、源码和原始 SDK archive；旧绝对路径只作
元数据，绝不据此打开文件。归档读取、资格验证、交付入口分为三个模块。

`release_archive.py` 的 `--identity` 必须独立提供 repository、commit、tag、tag_commit、run_id、attempt
和 validator_revision；该离线入口仍不联网、不证明来源。新增 `github_archive.py` 仅在
受信 GitHub Actions 上读取真实托管元数据，下载后复用离线验证器，批准后才写 Release。

## 托管阶段的具体契约

- `release-archive.yml` 只能手动触发且必须从默认分支运行；两阶段 checkout 同一个不可变 `github.workflow_sha`，不执行资格附件里的代码。资格运行也必须来自本仓库默认分支，成功且为指定的最新 attempt，候选提交必须是验证器提交的祖先。
- `verify` 只有 contents/actions read；输出六份附件内容绑定的 SHA256。`publish` 在 `rnet-release-archive` environment 审批后获得 contents write，重新下载、验证原始内容并比对只读阶段的摘要。两个阶段都受启动 19 GiB、运行中 15 GiB 内存保护。
- 审批环境使用个体 User 审核人（不要求组织成员读取权限），开启 prevent_self_review，唯一 deployment branch policy 是默认分支的精确名称。脚本独立核对审批历史、environment ID 和批准人，不能用管理员 bypass 代替独立审核。
- 审批 API 不携带 attempt 身份，因此归档工作流只接受第 1 次 attempt；失败后新建 workflow_dispatch 并重新审批。资格运行本身可以是任意指定的成功最新 attempt。
- tag 规则要求 active 的 update/deletion 限制，匹配候选 tag 精确名称或 ~ALL，exclude 为空。不解释任意通配符；发布前复查 tag。管理员仍是信任根，规则 bypass 信息在只读 token 下可能不可见，不宣称抵御有特权者并发修改策略/tag。
- 保留原有五附件离线 manifest 格式，另附 `hosting-origin.json` 记录资格运行 URL、GitHub artifact ID/ZIP 摘要及验证器身份。Release 正文绑定六附件摘要，记录最初归档和最终公开的 workflow URL；避免把尚未发生的人工审批写进可重复生成的离线 manifest。
- 先创建 prerelease draft，再上传并核对远端完整文件集合、大小和 digest。只对内容完全相同且为本流程创建的草稿补齐缺失附件，不覆盖或删除冲突内容。公开前再次核验审批、来源、tag、草稿元数据和附件完整性。
- 最终公开请求若响应丢失，Release 可能已经完整公开；新触发根据远端实际状态核验，不能把网络失败当成肯定未发布。同名 starter/损坏附件拒绝自动清理，需管理员检查；验证器更新或源身份变化则使用新的候选 tag。
- 工作流由仓库变量 `RNET_RELEASE_ARCHIVE_ENABLED=true` 启用；必须先完成管理员配置和隔离仓库演练。本轮不设置该变量、不创建 runner，也不运行外部发布。


## 2026-11-24 本地对抗性复审与验证

本轮只修改归档工具、工作流及说明，不改变网络库运行时或 SDK ABI。

| 已修复问题 | 回归证据 |
| --- | --- |
| 普通 EOCD 字段可掩盖实际 ZIP64 中央目录大小，绕过对象分配前的上限检查 | 构造 ZIP64/普通尾记录冲突，在进入 ZipFile 前必须拒绝 |
| 不同 Python 版本对 ZIP64 扩展尾记录的定位不同 | 只允许固定长度尾记录；正常 ZIP64 目录仍可解压，扩展尾记录提前拒绝 |
| 上传期间 Release 名称、候选标志或作者变化未被完整复核 | 元数据漂移阻止最终公开；已有附件不删除、不覆盖 |
| 新 dispatch 补齐草稿后缺少实际成功发布运行的记录 | 正文保留初始运行及成功发布运行 URL，新运行必须重新审批 |
| 空用户名形式的下载地址绕过用户信息检查 | 拒绝 `https://@api.github.com/`，令牌仅发送到精确 GitHub API 主机 |

验证：`make native-doc-check` 通过，包含 101 项脚本测试、生成头文件一致性及
原生接口注释检查；actionlint 1.7.7 校验归档和资格工作流通过；`git diff --check`
通过。脚本测试使用合成托管响应和实际 ZIP/离线校验流程，不是线上 GitHub 验收。

未完成的验证：真实审批环境读取曾返回 403，无法确认远端配置，不能据此判定环境
不存在；未执行真实上传、隔离仓库演练、目标环境弱网/长稳或本提交的完整发布资格。
工作流保持默认关闭，管理员配置和演练完成后才可启用。
