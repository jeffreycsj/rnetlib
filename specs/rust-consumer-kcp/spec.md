# Rust 外部接入的 KCP 运算契约

状态：2026-10-28 已完成实现及本轮本地验证；仍为未提交候选，不是生产认证。

## 需求与已确认问题

外部 Rust 应用直接依赖 netlib 时，不应为了 KCP 正确性另配 Cargo profile、
关闭溢出检查或复制仓库级配置。保持当前 SDK API、ABI 和 wire 不变。

Cargo 只使用调用方 workspace 根清单的 profile，忽略依赖库的 profile：
[Cargo 官方说明](https://doc.rust-lang.org/cargo/reference/profiles.html)。
当前 kcp 0.6.0 的时钟差值及部分序号、定时器更新依赖隐式整数回绕；
netlib 和 fuzz 根清单中的局部例外无法约束外部调用方的构建。

本地对照实验：

- 仓库已有 `kcp_engine` 五项测试通过。
- 独立 workspace 只声明 `rnet-transport` 路径依赖，默认 test profile。
  原有序号边界用例通过，但下面的时钟用例失败于 kcp 的整数减法溢出。
- 只在临时调用方加 `[profile.test.package.kcp] overflow-checks = false`，
  两项用例均通过。该对照证明构建条件差异，不作为最终修复。

```rust
let mut engine = RustKcpEngine::new(1)?;
engine.update(0x7fff_fff0, &mut |_| {});
engine.update(0x8000_0010, &mut |_| {});
engine.send(b"still usable")?;
```

复现位于 `/tmp/rnet-downstream-kcp.S2tOxg/`：`clock-repro.log` 为失败，
`profile-control.log` 为对照通过；临时目录不是永久发布证据。
当前影响已证实为外部 Rust 的受检运算构建，不推断已打包 release SDK 必然同样失败。

## 行为规格

- 当调用方使用默认 debug/test 或开启溢出检查的 release 时，KCP 应保持一致行为，
  无需调用方额外配置 profile、根级 patch 或编译器选项。
- 当时钟跨 i32 符号边界、u32 回绕以及序号跨界时，更新、重传、ACK 和收发不能
  因协议规定的回绕而 panic；序号间距仍遵守 KCP 的半空间比较规则。
- 当长度、分片数、缓存或非协议计数超限时，不能一律改成回绕以掩盖错误；
  保留现有输入校验、预算和可观测的失败语义。
- TCP/UDP、Noise、业务版本协商及对外接口不变，不增加旧版本兼容层。

## 建议技术方案与代价

1. 从当前锁定的 kcp 0.6.0 建立隔离的本地修正版 crate，保留上游 MIT 许可、
   来源版本、源包校验值和补丁说明。使用独立包名和直接依赖，内部可沿用 `kcp`
   别名；不使用只有调用方根目录才生效的 `[patch]` 修复。
2. 逐类检查时钟、序号、窗口和重传算术：协议模运算显式使用 wrapping，
   容量/长度运算保留检查或现有有界不变量，不机械替换全部加减。
3. 验证通过后删除主 workspace 与 fuzz 对 KCP 的 overflow-checks 例外。
   维护成本是需要跟踪上游更新并复审补丁；本地改名不能替代上游漏洞跟踪。
4. 增加真正独立于 netlib workspace 的 Rust 消费者测试，接入检查流程。
   用确定性时钟和状态边界注入测试回绕，不以长稳代替边界测试。

不选择只补接入文档、捕获 panic 或只在本库根清单打 patch：这些不能保证
外部受检构建下 KCP 的正确性。此方案不重写 KCP 协议，也不扩展平台承诺。

## 任务与验收

- [x] 固化外部消费者 RED：默认 test 的真实分片流量触发减法溢出；修复后默认 test 和受检 release 通过。
- [x] 引入有来源记录的隔离修正版；六项内部边界测试先 RED 后 GREEN，保留适配层校验和预算。
- [x] 覆盖分片、丢包/乱序、重传、ACK 边界和溢出错误路径，移除 profile 依赖；含可选异步路径共十项测试。
- [x] 接入 CI/本地外部消费者回归，核对主库/fuzz 锁文件及许可证；额外审计上游来源身份。
- [x] 运行 Rust 1.85、严格 Clippy、workspace、六目标 ASan、ABI 和多语言回归。
- [x] 按对抗性审查技能复核算术语义、测试独立性和边界；重建候选验证证据。

审查、证据范围与剩余限制见 [review.md](review.md)。本规格完成状态在验证后更新，
不属于门禁记录中的源指纹；代码、测试和构建配置在九项门禁之间没有变化。

全程串行高内存任务并保留 15 GiB 可用内存；不提交、推送、部署或启动目标长稳。
