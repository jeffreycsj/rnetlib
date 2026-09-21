//! Game-oriented session and messaging facade over RNet's transport runtime.

mod clock_sync;
mod clock_sync_runtime;
mod config;
mod diagnostics;
mod envelope;
mod event;
#[cfg(feature = "fuzzing")]
pub mod fuzz_support;
mod heartbeat;
mod heartbeat_runtime;
mod join;
mod lifecycle;
mod observe;
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

pub use clock_sync::ClockSyncSample;
pub use clock_sync_runtime::ClockSyncMetricsSnapshot;
pub use config::{
    GameClientConfig, GameHostClientConfig, GameProtocol, GameProtocolRange, GameRangeClientConfig,
    GameRangeHostClientConfig, GameRangeServerConfig, GameRuntimeConfig, GameServerConfig,
};
pub use diagnostics::GameLoggerSnapshot;
pub use event::{
    GameEvent, GameMessage, NetworkQuality, QualityBasis, QualityGrade, ResumeTicket,
    SensitiveBytes,
};
pub use observe::{HeartbeatMetricsSnapshot, ResumeMetricsSnapshot};
pub use quality::{QualityPolicy, UdpLossSnapshot};
pub use range_state::RangeBufferSnapshot;
pub use realtime::{RealtimeQueueConfig, RealtimeQueueSnapshot};
pub use rnet_observe::{BoundedLogger, LogLevel, LogRecord, LoggerConfig};
pub use rnet_transport::{KcpRetransmissionSnapshot, LatestTransportSnapshot};
pub use runtime::{GameRuntime, GameSendOptions};

#[cfg(test)]
mod clock_sync_tests;
#[cfg(test)]
mod envelope_tests;
#[cfg(test)]
mod heartbeat_tests;
#[cfg(test)]
mod join_tests;
#[cfg(test)]
mod quality_tests;
#[cfg(test)]
mod realtime_tests;
#[cfg(test)]
mod resume_tests;
