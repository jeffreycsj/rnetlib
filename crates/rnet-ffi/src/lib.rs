//! Stable C ABI for RNet.

#![deny(unsafe_op_in_unsafe_fn)]

mod abi;
mod abi_config;
mod endpoint;
mod error;
mod events;
mod game;
mod game_abi;
mod game_events;
mod game_range;
mod game_registry;
mod observe;
mod registry;
mod runtime;
mod session;

pub use abi::*;
pub use abi_config::*;
pub use endpoint::{
    rnet_client_connect_v2, rnet_client_join, rnet_endpoint_close, rnet_endpoint_local_port,
    rnet_endpoint_open, rnet_keypair_from_private, rnet_keypair_generate, rnet_listener_open,
    rnet_server_open_v2,
};
pub use error::rnet_last_error_message;
pub use events::{rnet_buffer_release, rnet_poll_events, rnet_poll_events_ex};
pub use game::*;
pub use game_abi::*;
pub use game_range::*;
pub use observe::{
    rnet_latency_snapshot, rnet_latency_snapshot_v2, rnet_metrics_log_interval_set,
    rnet_metrics_snapshot, rnet_metrics_snapshot_v2, rnet_metrics_snapshot_v3,
};
pub use runtime::{
    rnet_abi_version, rnet_config_init, rnet_config_v3_init, rnet_config_v4_init,
    rnet_config_v5_init, rnet_runtime_create, rnet_runtime_create_v2, rnet_runtime_create_v3,
    rnet_runtime_create_v4, rnet_runtime_create_v5, rnet_runtime_destroy, rnet_runtime_stop,
};
pub use session::{
    rnet_send, rnet_session_auth_decide, rnet_session_close, rnet_session_rekey,
    rnet_session_security_set, rnet_session_send, rnet_session_send_ex,
};
