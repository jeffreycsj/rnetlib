use crate::{
    config::{EndpointConfig, EndpointSecurity, RuntimeConfig},
    metrics::{Latencies, Metrics},
    state::{ByteBudget, Shared},
};
use rnet_core::{
    ErrorCode, Event, EventQueue, EventType, Handle, HandleTable, Lifecycle, Result, RnetError,
    RuntimeState, Transport,
};
use std::sync::{Arc, Mutex};
use tokio::runtime::{Builder, Runtime};

pub struct NetworkRuntime {
    pub(crate) runtime: Runtime,
    pub(crate) shared: Arc<Shared>,
}

impl NetworkRuntime {
    /// Creates a runtime without unified client credentials.
    pub fn new(config: RuntimeConfig) -> Result<Self> {
        Self::new_with_client_security(config, None)
    }

    /// Creates a runtime with identity and trust policy for unified client connections.
    pub fn new_with_client_security(
        config: RuntimeConfig,
        client_security: Option<crate::config::ClientSecurity>,
    ) -> Result<Self> {
        config.validate()?;
        let events =
            EventQueue::new_with_limits(config.event_queue_capacity, config.max_event_bytes)?;
        let runtime = Builder::new_multi_thread()
            .worker_threads(config.worker_threads)
            .enable_all()
            .thread_name("rnet-io")
            .build()
            .map_err(RnetError::from)?;
        let state = RuntimeState::created();
        state.start()?;
        let send_budget = ByteBudget::new(config.max_runtime_queued_bytes);
        let shared = Arc::new(Shared {
            config,
            client_security,
            state,
            events,
            endpoints: Mutex::new(HandleTable::new()),
            sessions: Mutex::new(HandleTable::new()),
            metrics: Metrics::default(),
            latencies: Latencies::default(),
            kcp_telemetry: Arc::new(crate::kcp::KcpTelemetryRegistry::default()),
            send_budget,
            latest: crate::latest::LatestRegistry::default(),
            stopped_event_emitted: Mutex::new(false),
        });
        let _ = shared
            .events
            .try_push(Event::simple(EventType::RuntimeStarted));
        Ok(Self { runtime, shared })
    }

    fn open_endpoint(&self, config: EndpointConfig) -> Result<Handle> {
        if self.shared.state.load() != Lifecycle::Running {
            return Err(RnetError::new(
                ErrorCode::InvalidState,
                "runtime is not accepting endpoints",
            ));
        }
        if let EndpointSecurity::AdaptiveClient { join_payload } = &config.security {
            let maximum = self.shared.config.max_body_len.min(60 * 1024);
            if join_payload.len() > maximum {
                return Err(RnetError::new(
                    ErrorCode::MessageTooLarge,
                    "join payload exceeds the configured handshake limit",
                ));
            }
        }
        if matches!(
            config.security,
            EndpointSecurity::AdaptiveServer {
                initial_mode: rnet_protocol::control::SecurityMode::Plaintext,
                ..
            }
        ) && !self
            .shared
            .config
            .security_policy
            .allow_plaintext_business_data
        {
            return Err(RnetError::new(
                ErrorCode::NotSupported,
                "plaintext business data is disabled by security policy",
            ));
        }
        match (config.transport, config.security.clone()) {
            (
                Transport::Tcp,
                EndpointSecurity::AdaptiveServer {
                    local_key,
                    initial_mode,
                },
            ) => self.open_adaptive_tcp_listener(config, local_key, initial_mode),
            (Transport::Tcp, EndpointSecurity::AdaptiveClient { join_payload }) => {
                self.open_adaptive_tcp_client(config, join_payload)
            }
            (Transport::Udp | Transport::Kcp, security) => {
                self.open_adaptive_datagram(config, security)
            }
        }
    }

    /// Starts a server listener using the transport selected in `config`.
    /// Every accepted session requires a Noise handshake and explicit business authorization,
    /// including when the server permits plaintext business records.
    ///
    /// The former unauthenticated endpoint constructor is not a public API:
    /// ```compile_fail
    /// use rnet_transport::EndpointConfig;
    /// let _ = EndpointConfig::tcp_listener("127.0.0.1:0".parse().unwrap());
    /// ```
    pub fn listen(&self, config: crate::config::ServerConfig) -> Result<Handle> {
        self.open_endpoint(config.into())
    }

    /// Starts a client connection using the transport selected in `config`.
    ///
    /// The caller does not select plaintext or encrypted operation. The authenticated server
    /// policy controls the initial mode and every later security transition.
    pub fn connect(&self, config: crate::config::ClientConfig) -> Result<Handle> {
        self.open_endpoint(config.into())
    }
}
