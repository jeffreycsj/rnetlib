//! Game-oriented session and messaging facade over RNet's transport runtime.

mod config;
mod envelope;
mod event;
#[cfg(feature = "fuzzing")]
pub mod fuzz_support;
mod join;
mod lifecycle;
mod observe;
mod runtime;

pub use config::{
    GameClientConfig, GameHostClientConfig, GameProtocol, GameRuntimeConfig, GameServerConfig,
};
pub use event::{GameEvent, GameMessage};
pub use runtime::{GameRuntime, GameSendOptions};

#[cfg(test)]
mod envelope_tests;
#[cfg(test)]
mod join_tests;
