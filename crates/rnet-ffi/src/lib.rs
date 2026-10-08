//! Stable C ABI for RNet.

#![deny(unsafe_op_in_unsafe_fn)]

mod abi;
mod abi_config;
mod endpoint;
mod error;
mod events;
mod game;
mod game_abi;
mod game_config_abi;
mod game_control;
mod game_events;
mod game_observe;
mod game_poll;
mod game_profile;
mod game_queue_abi;
mod game_range;
mod game_range_abi;
mod game_registry;
mod observe;
mod registry;
mod runtime;
mod session;

pub use abi::*;
pub use abi_config::*;
pub use endpoint::{
    rnet_client_connect, rnet_client_join, rnet_endpoint_close, rnet_endpoint_local_port,
    rnet_endpoint_open, rnet_keypair_from_private, rnet_keypair_generate, rnet_listener_open,
    rnet_server_listen,
};
pub use error::rnet_last_error_message;
pub use events::{rnet_buffer_release, rnet_poll_events};
pub use game::*;
pub use game_abi::*;
pub use game_config_abi::*;
pub use game_control::*;
pub use game_observe::*;
pub use game_poll::*;
pub use game_profile::*;
pub use game_queue_abi::*;
pub use game_range::*;
pub use game_range_abi::*;
pub use observe::{rnet_latency_snapshot, rnet_metrics_log_interval_set, rnet_metrics_snapshot};
pub use runtime::{
    rnet_abi_version, rnet_config_init, rnet_runtime_create, rnet_runtime_destroy,
    rnet_runtime_stop,
};
pub use session::{
    rnet_session_auth_decide, rnet_session_close, rnet_session_rekey, rnet_session_security_set,
    rnet_session_send, rnet_session_send_ex,
};
