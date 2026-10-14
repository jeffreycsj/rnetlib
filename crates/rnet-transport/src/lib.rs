//! Tokio TCP, UDP, and KCP transport runtime.

const MAX_HANDSHAKE_RECORD: usize = 65_535;

mod adaptive_codec;
mod adaptive_datagram;
mod adaptive_datagram_maintenance;
mod adaptive_datagram_process;
mod adaptive_datagram_send;
mod adaptive_datagram_session;
mod adaptive_kcp_preflight;
mod adaptive_peer;
mod adaptive_tcp;
mod adaptive_tcp_session;
mod adaptive_wire;
mod address;
mod admission;
mod auto_rekey;
mod config;
mod cookie;
mod endpoint_failure;
mod event;
#[cfg(feature = "fuzzing")]
pub mod fuzz_support;
mod kcp;
mod kcp_preflight;
mod latest;
mod lifecycle;
mod metrics;
mod record_reader;
mod runtime;
mod runtime_adaptive;
mod runtime_connect;
mod runtime_observe;
mod runtime_registry;
mod session;
mod session_close;
mod state;
mod tcp_socket;

pub use config::{
    ClientConfig, ClientSecurity, HostClientConfig, PeerVerifier, ResolvedClientConfig,
    RuntimeConfig, SecurityPolicy, ServerConfig,
};
pub use event::{AuthRequest, SecurityChange, SecurityOperation};
pub use kcp::{KcpEngine, KcpRetransmissionSnapshot, RustKcpEngine};
pub use latest::{LatestSendOutcome, LatestTransportSnapshot};
pub use metrics::{
    AdmissionRejectReason, LatencyKind, LatencyMetricSnapshot, MetricsSnapshot,
    ADMISSION_REJECT_REASON_COUNT, LATENCY_KIND_COUNT,
};
pub use rnet_protocol::control::SecurityMode;
pub use runtime::NetworkRuntime;
pub use session::SendOptions;
