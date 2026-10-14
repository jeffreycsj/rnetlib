//! Game-oriented session and messaging facade over RNet's transport runtime.
//!
//! Start with [`GameRuntimeConfig::production`], then [`GameRuntime::listen`] on the server
//! and [`GameRuntime::connect`] or [`GameRuntime::connect_host`] on a client configured with
//! server trust. Select transport once in endpoint configuration, never on each message.
//! Both sides must keep calling [`GameRuntime::poll`]. The server answers authorization events;
//! each side waits for [`GameEvent::SessionReady`] before sending opaque application payloads.
//!
//! # Contracts to keep in mind
//!
//! - Successful send means bounded local admission, not peer delivery; handle backpressure.
//! - Snapshot replacement and priority/expiry operate locally, not on already transmitted data.
//! - Only the server chooses business encryption; plaintext business data has no integrity.
//! - Resume tickets are one-use and server-runtime-local, never cross-process recovery.
//! - RTT/clock estimates include queueing effects and do not constitute an anti-cheat clock.
//!
//! # SDK compatibility
//!
//! One inclusive application version configuration serves every connection.
//! Older SDK source APIs are deliberately rejected:
//! ```compile_fail
//! use rnet_game::GameProtocolRange;
//! ```
//! ```compile_fail
//! let _ = rnet_game::GameProtocol::new(42, 3);
//! ```
//! ```compile_fail
//! use rnet_game::{GameRuntime, GameServerConfig};
//! fn obsolete(runtime: &GameRuntime, config: GameServerConfig) {
//!     let _ = runtime.listen_range(config);
//! }
//! ```

#![deny(missing_docs)]
#![doc = concat!(
    "\n# Executable server/client example\n\n",
    "This example runs separate server and client runtimes in one process. Its fixed login ",
    "credential is only a demonstration; production authorization must validate real credentials.\n\n",
    "```rust\n",
    include_str!("../examples/game_quickstart.rs"),
    "\n```\n"
)]

mod admission;
mod clock_sync;
mod clock_sync_runtime;
mod config;
mod diagnostics;
mod diagnostics_summary;
#[cfg(test)]
mod early_data_policy_tests;
mod endpoint_lifecycle;
#[cfg(test)]
mod endpoint_lifecycle_tests;
mod envelope;
mod event;
#[cfg(feature = "fuzzing")]
pub mod fuzz_support;
mod heartbeat;
mod heartbeat_runtime;
mod join;
mod lifecycle;
mod message_runtime;
mod observe;
mod poll_runtime;
mod quality;
mod quality_runtime;
mod range_api;
mod range_runtime;
mod range_state;
mod realtime;
mod realtime_runtime;
mod resume;
mod resume_runtime;
mod runtime;
mod scheduler;
mod scheduler_runtime;

pub use clock_sync::ClockSyncSample;
pub use clock_sync_runtime::ClockSyncMetricsSnapshot;
pub use config::{
    GameClientConfig, GameHostClientConfig, GameProfile, GameProtocol, GameRuntimeConfig,
    GameServerConfig,
};
pub use diagnostics::GameLoggerSnapshot;
pub use event::{
    GameEvent, GameMessage, NetworkQuality, QualityBasis, QualityGrade, ResumeTicket,
    SensitiveBytes,
};
pub use observe::{HeartbeatMetricsSnapshot, ProtocolMetricsSnapshot, ResumeMetricsSnapshot};
pub use quality::{QualityPolicy, UdpLossSnapshot};
pub use range_state::RangeBufferSnapshot;
pub use realtime::{RealtimeQueueConfig, RealtimeQueueSnapshot};
pub use rnet_observe::{BoundedLogger, LogLevel, LogRecord, LoggerConfig};
pub use rnet_transport::{KcpRetransmissionSnapshot, LatestTransportSnapshot};
pub use runtime::{GameRuntime, GameSendOptions};
pub use scheduler::{GamePriority, ScheduledQueueConfig, ScheduledQueueSnapshot};

#[cfg(test)]
mod clock_sync_tests;
#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod envelope_tests;
#[cfg(test)]
mod heartbeat_tests;
#[cfg(test)]
mod join_tests;
#[cfg(test)]
mod plaintext_policy_tests;
#[cfg(test)]
mod quality_tests;
#[cfg(test)]
mod realtime_tests;
#[cfg(test)]
mod resume_tests;
#[cfg(test)]
mod scheduler_tests;
