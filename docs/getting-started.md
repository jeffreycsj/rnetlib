# RNet getting started

## 游戏 API 的代码注释与可执行示例

游戏接入首先使用 `rnet-game`，不要将下面底层传输示例中的 `SessionOpened`
当作游戏层的 `SessionReady`。完整游戏收发循环见
[`game_quickstart.rs`](../crates/rnet-game/examples/game_quickstart.rs)。

`cargo doc -p rnet-game --no-deps --open` 可查看 Rust 游戏接口的类型、成员和方法说明，
包括参数单位/零值、句柄有效期、缓冲区所有权、错误、线程约束和异步完成事件。
crate 首页直接包含同一份可执行示例，避免文档和示例维护两份不同代码。

### 按语言找到完整游戏流程

所有语言都使用同一顺序：配置身份与传输 → listen/connect → 持续正容量 poll →
服务端授权 → SessionReady → send/Message → 处理背压和断线 → 关闭与销毁。
以下区分独立示例与回归测试，后面的底层传输片段不能代替这些游戏流程。

| 语言 | 可执行入口 | 本地命令 |
| --- | --- | --- |
| Rust | [有界重试的双端游戏回显](../crates/rnet-game/examples/game_quickstart.rs) | `cargo run -p rnet-game --example game_quickstart -- tcp --one-slot` |
| C11 | [双端游戏回显](../examples/c/game_echo.c)、[失败路径释放](../examples/c/game_poll.h) | `make c-test`，然后 `./target/rnet-c-game-echo tcp`（可换 udp/kcp） |
| C++11 | [有界重试的双端回显](../examples/cpp/game_quickstart.cpp)、[恢复回归](../examples/cpp/game_echo_smoke.cpp) | `make cpp-test`，然后 `./target/rnet-cpp-game-quickstart tcp --one-slot` |
| Go | [有界重试的双端游戏回显](../examples/go/game_quickstart/runtime.go)、[命令行入口](../examples/go/game_quickstart/main.go) | `make go-test`，然后 `./target/rnet-go-game-quickstart tcp --one-slot` |
| C# | [双端游戏回显](../csharp/RNet.Quickstart/GameExample.cs)、[接入说明与运行命令](csharp.md) | `make csharp-test`，然后按 C# 指南运行 `RNet.Quickstart` |

C、Rust、C++11、Go 和 C# 示例演示二进制/空消息、日志注入、收到背压后保留副本且继续 poll、授权拒绝和清理。
其 `--deny` 模式预期失败；示例票据不能用于生产。其他片段不代表已经实现完整业务重试队列。
所有语言的发送成功都只表示本地入队，不能当作送达；UDP 业务仍可能丢失/重复/乱序。

Rust 示例可选 tcp/udp/kcp（无参数默认 KCP），`--one-slot` 使用真实单消息预算演示背压，
`--deny` 必须以 AuthRejected 失败，不能当作运行成功。`cargo test -p rnet-game --example game_quickstart`
执行边界和联网回归；该测试目标已加入 workspace 测试，不只在文档编译时检查。

### 文档防回退检查

`make doc-check` 已加入 `make check` 和 Linux CI，包含：

- Rust core/protocol/security/observe/transport/game 的缺失注释、严格 rustdoc 与文档链接检查，
  以及 doctest（含真实客户端/服务端握手、授权与回显）。
- `make native-doc-check`：以 C11/C++11 编译公开头文件，通过 Clang AST 检查 C 的 `rnet_` 声明、
  `RNET_` 枚举项，以及 C++ `rnet` 命名空间中的类型、字段、枚举和公开/受保护方法。
  构造/析构、删除重载和回调 typedef 也需要文档；类型注释不能替代成员注释。

原生门禁需要 Python 3（仅标准库）和支持 JSON AST 的 Clang/Clang++；本机验证版本为 Clang 17。
可用 `make native-doc-check CLANG=/path/to/clang CLANGXX=/path/to/clang++ PYTHON=/path/to/python3`
指定工具。编译器缺失、语法/文档参数错误、未支持的公开声明形态或零覆盖都会失败。
检查器自身的测试使用真实编译器，并验证删除注释后确实被拒绝，不只验证完整头文件通过。

Go 导出项的 AST 注释检查随 `make go-test` 执行；C# 的缺失 XML 文档检查随 `make csharp-test`
和 SDK 构建执行，生成的 `RNet.xml` 随库发布。这两项不包含在单独的 `make doc-check` 中。
原生宏和命名空间摘要仍需人工核对；任何“存在注释”检查都不能证明参数、返回值或并发语义写得正确，
也不代表生成了 C/C++ HTML 文档或完成目标环境长稳验证。

## 跨语言接入契约与差异

不同语言封装同一协议，但内存所有权和数值类型并不相同；不能直接复制另一语言的清理/单位处理。
以下是本轮核对的游戏 API 路径，不覆盖所有底层传输 API 或平台。

| 契约 | Rust | C | C++11 | Go | C# |
| --- | --- | --- | --- | --- | --- |
| 收到的消息 | 拥有 Bytes，可跨后续 poll 保留 | 借用导出字节；每个非零 token 在所属 runtime 释放 | 复制到 vector，包装层释放 token | 复制到 slice，包装层释放 token | 复制到 byte[]，finally 释放 token |
| poll 部分失败 | Rust 无 C 导出 token | 失败也须释放 out_count 前缀的主/辅助 token | 异常仍清理已导出前缀 | error 路径清理已导出前缀 | 异常仍清理已导出前缀 |
| 无时钟样本 | None | available=0 | available=false | Available=false | null |
| 时钟偏移单位 | i64 微秒 | int64_t 微秒 | int64_t 微秒 | time.Duration 纳秒，越界限幅至有符号整微秒边界 | long 微秒 |
| 发送过期禁用值 | None；Some(0) 则立即过期 | expiry_ms=0 | expiry_ms=0 | Expiry=0；正值向上取整到毫秒 | ExpiryMs=0 |
| 检查关闭错误 | stop 返回 Result | stop/release/destroy 分别检查返回码 | 显式 close；析构仅尽力清理 | Close 返回 error，失败保留句柄以便处理/重试 | 显式 Stop 观察错误；Dispose 不报告 SafeHandle 内部 native 清理失败 |

共同要求：所有句柄只属于各自 runtime；先授权并等到游戏就绪，再发送 opaque payload。
发送成功仅代表本地接受；Send 的错误和后续失败事件都须处理。Poll 失败/转换失败时，已消费事件不保证重放。
时钟查询要求 game-ready session，未就绪或失效句柄应报错，不能当成“尚未采样”；None/available/null
也不能当作测得零偏移。范围极值的限幅不证明对端时钟可信。

零容量 poll 只做有界维护，不能长期替代正容量 poll；积压达到预算会暂停读取后续传输事件，定时器仍推进。
Go 的容量范围是 0..65536，C# 是 0..4096；C/C++ 的调用方/封装会按请求容量准备数组，避免盲目使用巨大容量。
关闭前先停止其他业务调用者，日志回调不得关闭 runtime。C++ 析构和 C# Dispose 的尽力清理边界不能当作
与 C 的逐项返回码检查等价；C# 的在途 lease 还可能延后 native 清理和回调结束。

## One endpoint API, three transports

Servers call `listen`; clients call `connect`. Set `transport` to TCP, UDP, or KCP in the configuration. The selected transport is immutable for the endpoint and session. `send` therefore needs only a session and opaque payload; business message types belong inside that payload.

Every unified connection performs an application handshake before `SessionOpened`. Client identity and server trust belong to the runtime. The client never chooses whether a connection is plaintext or encrypted. The server selects the initial business-data mode and may later switch it; cryptographic control traffic remains authenticated in both modes.

Use TCP for reliable ordered byte-stream delivery, UDP when loss and reordering are acceptable, and KCP when reliable ordered delivery over UDP is required.

## Rust

```rust
use rnet_core::{EventType, Transport};
use rnet_security::Keypair;
use rnet_transport::{
    ClientSecurity, HostClientConfig, NetworkRuntime, RuntimeConfig, SecurityChange,
    SecurityMode, ServerConfig,
};

let server_key = Keypair::generate()?;
let client_key = Keypair::generate()?;
let trust = ClientSecurity::pinned(client_key, server_key.public.clone());
let runtime = NetworkRuntime::new_with_client_security(
    RuntimeConfig::production(), Some(trust),
)?;

let listener = runtime.listen(ServerConfig {
    transport: Transport::Tcp,
    bind_addr: "127.0.0.1:0".parse()?,
    local_key: server_key,
    initial_security: SecurityMode::Encrypted,
})?;
let port = runtime.endpoint_local_addr(listener)?.port();
let client = runtime.connect_host(HostClientConfig {
    transport: Transport::Tcp,
    host: "localhost".to_owned(), // numeric IPv4/IPv6 and DNS names are accepted
    port,
    join_payload: b"login-ticket".to_vec(),
})?;

for event in runtime.poll_events(32, std::time::Duration::from_millis(50)) {
    match event.event_type {
        EventType::AuthRequest => runtime.auth_decide(event.session, true)?,
        EventType::SessionOpened if event.endpoint == client => {
            runtime.send(event.session, b"hello")?;
        }
        _ => {}
    }
}
```

`RuntimeConfig::default()` and `RuntimeConfig::production()` now use the same secure defaults: no legacy unauthenticated endpoints, no server-selected plaintext business data, and automatic rekey enabled. There is no `compatibility()` preset.

Only a server-side session may initiate changes. To allow the plaintext transition below, explicitly set `config.security_policy.allow_plaintext_business_data = true` **before creating the server runtime**. This permits plaintext business bytes, not unauthenticated setup or controls; it is unsuitable for untrusted network paths. The client does not need this flag and automatically follows the authenticated server policy.

```rust
runtime.set_security_mode(server_session, SecurityMode::Plaintext)?;
runtime.set_security_mode(server_session, SecurityMode::Encrypted)?;
runtime.rekey_session(server_session)?;
```

Wait for `SecurityChanged` before starting another change. Sends accepted during a transition remain ordered and are emitted after its barrier completes.
Decode the event with `SecurityChange::from_event(&event)` to obtain the typed operation,
active mode, and committed epoch.

## C

Before calling any initializer, require `rnet_abi_version() == RNET_ABI_VERSION`; do not mix headers, SDKs or native libraries from different builds. Check every returned status in production.

```c
rnet_config_t config;
rnet_config_init(&config);

rnet_client_security_t trust = {0};
trust.struct_size = sizeof(trust);
trust.abi_version = RNET_ABI_VERSION;
trust.local_private_key = (rnet_slice_t){client_private, 32};
trust.expected_server_public_key = (rnet_slice_t){server_public, 32};

rnet_runtime_t runtime = 0;
rnet_runtime_create(&config, &trust, &runtime);

rnet_server_config_t server = {0};
server.struct_size = sizeof(server);
server.abi_version = RNET_ABI_VERSION;
server.transport = RNET_TRANSPORT_UDP;
server.initial_security = RNET_SECURITY_ENCRYPTED;
server.bind_host = (rnet_slice_t){(const uint8_t *)"127.0.0.1", 9};
server.bind_port = 7000;
server.local_private_key = (rnet_slice_t){server_private, 32};
rnet_server_listen(runtime, &server, &listener);

rnet_client_config_t client = {0};
client.struct_size = sizeof(client);
client.abi_version = RNET_ABI_VERSION;
client.transport = RNET_TRANSPORT_UDP;
client.remote_host = (rnet_slice_t){(const uint8_t *)"game.example.com", 16};
client.remote_port = 7000;
client.join_payload = (rnet_slice_t){ticket, ticket_len};
rnet_client_connect(runtime, &client, &client_endpoint);
```

Alternatively set `trust.verify_server` to a thread-safe callback. Its function and `user_data` must remain valid until `rnet_runtime_destroy` returns. Poll `RNET_EVENT_AUTH_REQUEST`, validate `client_public_key[32] || join_payload`, and call `rnet_session_auth_decide`. Release every nonzero event `buffer_token` exactly once.

Use `rnet_session_send`, `rnet_session_security_set`, and `rnet_session_rekey`. Stop the runtime, release all event buffers, then destroy it.
If a synchronous call fails, copy `rnet_last_error_message()` before the next RNet call for a
human-readable diagnostic.

## C++11

```cpp
rnet::ServerOptions server;
server.transport = rnet::Transport::Kcp;
server.initial_security = rnet::SecurityMode::Encrypted;

std::array<uint8_t, 32> expected;
std::copy(server.keypair.public_key(), server.keypair.public_key() + 32,
          expected.begin());
rnet::Keypair client_key;
rnet_config_t config = {};
rnet::check(rnet_config_init(&config));
rnet::Runtime runtime(config, client_key, expected);

rnet_endpoint_t listener = runtime.listen(server);
rnet::ClientOptions client;
client.transport = rnet::Transport::Kcp;
client.port = runtime.local_port(listener);
client.join_payload = "login-ticket";
rnet_endpoint_t endpoint = runtime.connect(client);
```

Poll events, call `auth_decide`, then use `send`. Server-side sessions can call `set_security(session, mode)` and `rekey(session)`. The wrapper is C++11-only and owns the runtime through RAII.

## Go

游戏接入使用 `GameRuntime`，完整代码见 [runtime.go](../examples/go/game_quickstart/runtime.go)。
监听与客户端各有独立 runtime，必须同时持续正容量 `Poll`；服务端 `AuthDecide` 后，
客户端仍需等待 `GameSessionReady` 才能 `Send(session, payload)`，消息通过 `GameMessage` 接收。

```sh
make go-test
./target/rnet-go-game-quickstart tcp --one-slot
./target/rnet-go-game-quickstart udp
./target/rnet-go-game-quickstart kcp --deny  # 预期授权失败，退出码 1
```

示例 [pending.go](../examples/go/game_quickstart/pending.go) 独立说明应用重试队列：
每端最多两条 128 字节副本，只在发送成功后出队，背压时继续 poll，不循环忙等。
三传输测试都验证二进制和空消息的实际回显；`--one-slot` 必须出现真实背压。
清理会尝试双端关闭，通过 `errors.Join` 保留原始和清理错误，不能忽略 `Close()` 返回值。

日志注入、发布包编译、参数默认值和运行边界见 [Go 游戏接入说明](game-networking.md#go)。
默认保持加密；运行中调策略用服务端会话的 `SetSecurity` / `Rekey`，相关回归在
[game_test.go](../go/rnet/game_test.go)，不是本示例的收发前置条件。

## Production sizing and load probe

Rust deployments can tune `max_sessions_per_endpoint`, `handshake_timeout`,
`datagram_idle_timeout`, event capacity, write capacity, payload limits, worker threads, and KCP
MTU through `RuntimeConfig`. TCP defaults to `TCP_NODELAY`; optional send/receive buffer requests are
available in Rust `RuntimeConfig`, C/C++ `rnet_config_t`, and Go `Config`. Runtime-wide endpoint and pending-handshake limits complement per-listener capacity so unauthenticated traffic cannot
bypass admission control. Size queues from an explicit memory budget; increasing every limit is not
a substitute for backpressure in the application.

Host names and textual IPv4/IPv6 addresses are supported. TCP, UDP, and KCP try resolved candidates
under a single deadline. UDP/KCP automatically rebind their client socket when retrying a candidate
from another address family.

Run the encrypted local probe before deployment, then repeat across real hosts with representative
loss, RTT, payloads, connection counts, and application event processing:

```sh
cargo run --release -p rnet-transport --example load_probe -- tcp 1000000 4096
cargo run --release -p rnet-transport --example load_probe -- kcp 100000 4096
cargo run --release -p rnet-transport --example load_probe -- udp 100000 1024
./scripts/run-soak.sh tcp 86400 4096
```

The probe reports CPU, RSS, message rate, MiB/s, queue/error counters, and P50/P90/P95/P99/P99.9/max latency samples. Enterprise readiness is
an environment-specific capacity result: define an SLO, run soak and fault tests at or above peak
load, and alert on `events_dropped`, `send_would_block`, protocol errors, queue percentiles, and KCP
update delay.

## Observability and shutdown

Latency snapshots expose sample count, P50, P90, P95, P99, P99.9, and max in microseconds for connect, cryptographic handshake, authorization wait, send queue, event queue, KCP RTT/update delay, and logger callback latency. Go `LatencyMetrics` reads cumulative values and `DrainLatencyMetrics` rotates the interval; C++ passes `true` to `latency_metrics`. Periodic log summaries are enabled with `rnet_metrics_log_interval_set` or its SDK wrapper. Go can set `Config.Logger` to receive `LogRecord`; callbacks must be thread-safe and return quickly.

Close sessions/endpoints when useful, stop the runtime with a drain timeout, release borrowed C event buffers, then destroy/close the runtime. `WOULD_BLOCK` means a bounded queue or transition slot is full; poll progress and retry according to application backpressure policy.
