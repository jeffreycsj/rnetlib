# 七项生产游戏网络库审查与补齐

沿用已确认的 wire v3/v4、服务端权威安全切换、单实例恢复语义，落实用户本轮七项要求。

## 行为与边界

- C# 使用 C 游戏 ABI，提供 Listen/Connect/Send/Poll、认证、关闭、动态加密、参数配置和指标；不重新实现协议。面向 .NET 8+，首先验收 Linux x86_64；Windows/Unity/AOT 需单独验证。
- 收包复制为托管数组，finally 释放两枚 native token；回调保持存活直到 destroy 完成，异常不越过 native 边界。Dispose 与使用协调，回调禁止 Stop/Dispose、允许只读指标查询。
- 游戏日志默认每 30 秒输出累计延迟分位数，可设置间隔或用 0 禁用，由 poll 驱动，含样本数、P90/P95/P99/P99.9/max、队列及 logger 丢弃。
- 异步错误日志保留具体库内校验原因、句柄和 correlation，不输出凭据或 payload；只有错误码时不得伪装为具体 OS 错误。
- 长稳与目标弱网矩阵仍为最后验收，短时回环不代表容量或安全认证。

## 任务

- [x] 现有游戏日志与 C ABI 基线测试通过。
- [x] 全库七项证据审查与残余风险记录。
- [x] 日志摘要、错误原因及 Rust/C/C++/Go 接口和回归。
- [x] 分模块 C# 门面、三传输/回调/GC/释放/错误路径测试。
- [x] CI、SDK 打包、示例与支持范围同步。
- [x] 串行全库及跨语言验证。

本轮确认并修复 native 注册表和 Go wrapper 的 logger 销毁死锁，以及 Go TLS 诊断文本跨线程读取问题。全库 Clippy/串行测试、C++11、Go/race、C#、Loom 和 ABI 校验均通过。C# 使用发布包的托管 DLL/native 动态库通过三传输流程，不依赖重新构建 SDK；详情与剩余生产门禁见 `docs/game-library-review.md`。本轮未执行目标环境长稳。
