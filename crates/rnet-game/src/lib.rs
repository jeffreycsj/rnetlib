//! Game-oriented session and messaging facade over RNet's transport runtime.

mod config;
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
mod runtime;

pub use config::{
    GameClientConfig, GameHostClientConfig, GameProtocol, GameRuntimeConfig, GameServerConfig,
};
pub use event::{GameEvent, GameMessage, NetworkQuality, QualityBasis, QualityGrade};
pub use observe::HeartbeatMetricsSnapshot;
pub use quality::{QualityPolicy, UdpLossSnapshot};
pub use rnet_transport::KcpRetransmissionSnapshot;
pub use runtime::{GameRuntime, GameSendOptions};

#[cfg(test)]
mod envelope_tests;
#[cfg(test)]
mod heartbeat_tests;
#[cfg(test)]
mod join_tests;
#[cfg(test)]
mod quality_tests;
