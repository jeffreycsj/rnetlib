# 人工批准的 Release 证据归档

状态：设计待确认，尚未实现、未发布，也未变更仓库权限。

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

- [ ] 确认本设计、审核人和 environment 配置；不将方案确认当作实际发布授权。
- [ ] TDD 实现离线归档校验：混合运行、脏源码、缺失 gate、日志篡改、错误 tag、
      非法解压路径、重复文件和资源跳过都拒绝。
- [ ] 实现只读验证、人工审批、最小权限发布以及冲突安全的重试。
- [ ] 用隔离测试仓库验证：未批准/拒绝均无 Release 写入，上传失败只有 draft，
      下载附件可独立复核并追溯原始运行。
- [ ] 对抗性审查通过，文档记录实际配置和演练结果后再启用。
