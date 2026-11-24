# 普通 GitHub CI 独立资源预算

2026-11-24：用户已确认普通 CI 单独设置资源预算，本地及发布资格规则不变。

## 行为契约

- 普通 `ci` 工作流的 Linux SDK job 显式选择 `RNET_RESOURCE_PROFILE=github-ci`：
  启动要求 6 GiB 可用内存，probe/soak 运行时保留 2 GiB。串行构建、单测试线程。
- 低预算只在 `GITHUB_ACTIONS=true`、`GITHUB_WORKFLOW=ci`、显式选择配置，且不存在
  `RNET_EVIDENCE_RUN_ID` 时生效；配置缺失、未知值、本地或资格上下文均使用严格规则。
- 本地 probe 仍为启动 19 GiB、运行保留 15 GiB；本地 soak 原有 15 GiB 门槛不变。
  发布资格执行器自身仍为 19/15 GiB，并强制给子进程传入 strict，防止继承普通 CI 配置。
- TCP/UDP/KCP、坏 pin/凭据、故障路径和已打包 probe 测试不减少、不跳过；资源不足
  仍使普通 CI 失败，失败不得上传 SDK。未知内存不视为达标。
- 不新增网络库公开 API，不修改生产运行时；预算只用于开发 probe 和测试工具。
- 该预算不是容量认证；不触发长稳、不设置远端 runner，不自动提交或推送。

## 实现

在二进制支持目录共享 Rust 预算选择函数，probe/soak 和测试前置检查共用；
CI 的 Python 前置检查采用明确的 6 GiB 阈值，不修改发布资格常量。
发布执行器在创建子进程时强制 strict，打包测试继承调用方环境。

## 验收

- [x] 原有 CI、发布门禁与 probe/soak 测试基线通过。
- [x] 预算选择的环境组合、低内存和边界值回归先失败再通过。
- [x] 发布执行器在继承 github-ci 时仍使用严格子进程环境。
- [x] 严格配置及普通 CI 配置均实际运行 TCP/UDP/KCP probe/soak 测试。
- [x] 生成头文件/脚本回归、Rust Clippy、工作流语法与差异检查通过。
- [x] 复审确认无静默跳过、无本地/发布预算降级；记录验证边界。


## 验证结果与复审

- 原有 CI 3 项、发布资格 18 项、probe/soak 13 项基线通过。
- 新增回归先复现：14.71 GiB 被旧 CI 门槛拒绝、发布子进程继承 github-ci；修复后通过。
- strict 与 github-ci 两组各 17 项 Rust 回归通过，无跳过；含 TCP/UDP/KCP、多进程、
  错 pin/凭据、断流/坏 echo/缺 ACK、资源选择和 soak 完成计数。
- `make native-doc-check` 通过（102 项脚本测试、头文件一致性、原生接口注释）。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`、structure-check、
  actionlint、fmt 和 diff 检查通过。

复审重点：入口与子进程预算必须一致，资源不足不得变成成功 skip，CI 配置不得泄漏
到本地或发布资格。发布环境继承问题已由实际子进程测试覆盖；共享 Rust 策略保持
未知配置严格回退。没有新增 SDK API，也没有更改传输/加密逻辑。

验证边界：14.71 GiB 由前置检查的注入内存读数覆盖；网络回归在本地充足内存机器
模拟 CI 环境执行，外层监督始终保留本机 15 GiB。尚未提交/推送或重跑远端 Actions，
未重新打包 SDK、执行目标环境长稳或进行新的完整发布资格认证。
