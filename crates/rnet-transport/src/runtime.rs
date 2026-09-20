use crate::{
    config::{EndpointConfig, EndpointSecurity, RuntimeConfig},
    metrics::{Latencies, LatencyKind, Metrics},
    secure_datagram::run_secure_udp_endpoint,
    secure_kcp::run_secure_kcp_endpoint,
    secure_tcp::{run_secure_tcp_client, run_secure_tcp_listener},
    state::{push_tcp_event, session_event, ByteBudget, SessionTarget, Shared},
    tcp::{run_tcp_listener, run_tcp_session},
    tcp_socket::configure_std_tcp,
    udp::run_udp_endpoint,
};
use rnet_core::{
    ErrorCode, Event, EventQueue, EventType, Handle, HandleTable, Lifecycle, Result, RnetError,
    RuntimeState, Transport,
};
use rnet_security::Keypair;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::{
    net::{TcpListener, TcpStream, UdpSocket},
    runtime::{Builder, Runtime},
    sync::mpsc,
};

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
            send_budget,
            stopped_event_emitted: Mutex::new(false),
        });
        let _ = shared
            .events
            .try_push(Event::simple(EventType::RuntimeStarted));
        Ok(Self { runtime, shared })
    }

    pub fn open_endpoint(&self, config: EndpointConfig) -> Result<Handle> {
        if self.shared.state.load() != Lifecycle::Running {
            return Err(RnetError::new(
                ErrorCode::InvalidState,
                "runtime is not accepting endpoints",
            ));
        }
        if let Some(
            EndpointSecurity::Client { join_payload, .. }
            | EndpointSecurity::AdaptiveClient { join_payload },
        ) = &config.security
        {
            let maximum = self.shared.config.max_body_len.min(60 * 1024);
            if join_payload.len() > maximum {
                return Err(RnetError::new(
                    ErrorCode::MessageTooLarge,
                    "join payload exceeds the configured handshake limit",
                ));
            }
        }
        if config.security.is_none()
            && !self
                .shared
                .config
                .security_policy
                .allow_legacy_unauthenticated_endpoints
        {
            return Err(RnetError::new(
                ErrorCode::NotSupported,
                "legacy unauthenticated endpoints are disabled by security policy",
            ));
        }
        if matches!(
            config.security,
            Some(EndpointSecurity::AdaptiveServer {
                initial_mode: rnet_protocol::control::SecurityMode::Plaintext,
                ..
            })
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
        match (config.transport, config.listen, config.security.clone()) {
            (
                Transport::Tcp,
                true,
                Some(EndpointSecurity::AdaptiveServer {
                    local_key,
                    initial_mode,
                }),
            ) => self.open_adaptive_tcp_listener(config, local_key, initial_mode),
            (Transport::Tcp, false, Some(EndpointSecurity::AdaptiveClient { join_payload })) => {
                self.open_adaptive_tcp_client(config, join_payload)
            }
            (
                Transport::Udp | Transport::Kcp,
                _,
                Some(
                    security @ (EndpointSecurity::AdaptiveServer { .. }
                    | EndpointSecurity::AdaptiveClient { .. }),
                ),
            ) => self.open_adaptive_datagram(config, security),
            (Transport::Tcp, true, Some(EndpointSecurity::Server { local_key })) => {
                self.open_secure_tcp_listener(config, local_key)
            }
            (
                Transport::Tcp,
                false,
                Some(EndpointSecurity::Client {
                    local_key,
                    expected_server_public,
                    join_payload,
                }),
            ) => {
                self.open_secure_tcp_client(config, local_key, expected_server_public, join_payload)
            }
            (Transport::Tcp, true, None) => self.open_tcp_listener(config),
            (Transport::Tcp, false, None) => self.open_tcp_client(config),
            (Transport::Udp, _, None) => self.open_udp(config),
            (Transport::Udp, true, Some(security @ EndpointSecurity::Server { .. })) => {
                self.open_secure_udp(config, security)
            }
            (Transport::Udp, false, Some(security @ EndpointSecurity::Client { .. })) => {
                self.open_secure_udp(config, security)
            }
            (Transport::Kcp, true, Some(security @ EndpointSecurity::Server { .. })) => {
                self.open_secure_kcp(config, security)
            }
            (Transport::Kcp, false, Some(security @ EndpointSecurity::Client { .. })) => {
                self.open_secure_kcp(config, security)
            }
            (Transport::Kcp, _, _) => Err(RnetError::new(
                ErrorCode::NotSupported,
                "KCP requires the authenticated listener/join API",
            )),
            _ => Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "security role is incompatible with endpoint mode",
            )),
        }
    }

    /// Starts a server listener using the transport selected in `config`.
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

    fn open_secure_tcp_listener(
        &self,
        config: EndpointConfig,
        local_key: Keypair,
    ) -> Result<Handle> {
        let bind_addr = config.bind_addr.ok_or_else(|| {
            RnetError::new(
                ErrorCode::InvalidArgument,
                "TCP listener requires bind_addr",
            )
        })?;
        let listener = self.runtime.block_on(TcpListener::bind(bind_addr))?;
        let endpoint = self.insert_endpoint(listener.local_addr()?, config.transport)?;
        if let Err(error) = self.push_endpoint_opened(endpoint) {
            self.discard_endpoint(endpoint);
            return Err(error);
        }
        let shared = Arc::clone(&self.shared);
        let task = self.runtime.spawn(async move {
            run_secure_tcp_listener(shared, endpoint, listener, local_key).await;
        });
        self.set_endpoint_abort(endpoint, task.abort_handle())?;
        Ok(endpoint)
    }

    fn open_secure_tcp_client(
        &self,
        config: EndpointConfig,
        local_key: Keypair,
        expected_server_public: Vec<u8>,
        join_payload: Vec<u8>,
    ) -> Result<Handle> {
        let remote_addr = config.remote_addr.ok_or_else(|| {
            RnetError::new(
                ErrorCode::InvalidArgument,
                "TCP client requires remote_addr",
            )
        })?;
        let connect_started = Instant::now();
        let stream =
            std::net::TcpStream::connect_timeout(&remote_addr, self.shared.config.connect_timeout)?;
        configure_std_tcp(&stream, &self.shared.config)?;
        stream.set_nonblocking(true)?;
        let stream = {
            let _entered = self.runtime.enter();
            TcpStream::from_std(stream)?
        };
        self.shared
            .latencies
            .record(LatencyKind::Connect, connect_started.elapsed());
        let endpoint = self.insert_endpoint(stream.local_addr()?, config.transport)?;
        let (sender, receiver) = mpsc::channel(self.shared.config.write_queue_capacity);
        let session = match self.insert_pending_session(endpoint, SessionTarget::Tcp(sender), None)
        {
            Ok(session) => session,
            Err(error) => {
                self.discard_endpoint(endpoint);
                return Err(error);
            }
        };
        if let Err(error) = self.push_endpoint_opened(endpoint) {
            self.discard_session(session);
            self.discard_endpoint(endpoint);
            return Err(error);
        }
        let shared = Arc::clone(&self.shared);
        let task = self.runtime.spawn(async move {
            run_secure_tcp_client(
                shared,
                endpoint,
                session,
                stream,
                receiver,
                local_key,
                expected_server_public,
                join_payload,
            )
            .await;
        });
        self.set_endpoint_abort(endpoint, task.abort_handle())?;
        Ok(endpoint)
    }

    fn open_secure_udp(
        &self,
        config: EndpointConfig,
        security: EndpointSecurity,
    ) -> Result<Handle> {
        let bind_addr = config.bind_addr.ok_or_else(|| {
            RnetError::new(ErrorCode::InvalidArgument, "secure UDP requires bind_addr")
        })?;
        let socket = Arc::new(self.runtime.block_on(UdpSocket::bind(bind_addr))?);
        let endpoint = self.insert_endpoint(socket.local_addr()?, config.transport)?;
        let (sender, receiver) = mpsc::channel(self.shared.config.write_queue_capacity);
        let initial_session = match &security {
            EndpointSecurity::Client { .. } => {
                let peer = config.remote_addr.ok_or_else(|| {
                    RnetError::new(
                        ErrorCode::InvalidArgument,
                        "secure UDP client requires peer",
                    )
                })?;
                let session = self.insert_pending_session(
                    endpoint,
                    SessionTarget::Udp {
                        sender: sender.clone(),
                        peer,
                        max_frame_len: self.shared.config.max_datagram_size.saturating_sub(25),
                        cleanup: None,
                    },
                    None,
                );
                match session {
                    Ok(session) => Some(session),
                    Err(error) => {
                        self.discard_endpoint(endpoint);
                        return Err(error);
                    }
                }
            }
            EndpointSecurity::Server { .. } => None,
            _ => {
                return Err(RnetError::new(
                    ErrorCode::InvalidArgument,
                    "invalid UDP security role",
                ))
            }
        };
        if let Err(error) = self.push_endpoint_opened(endpoint) {
            if let Some(session) = initial_session {
                self.discard_session(session);
            }
            self.discard_endpoint(endpoint);
            return Err(error);
        }
        let shared = Arc::clone(&self.shared);
        let task = self.runtime.spawn(async move {
            run_secure_udp_endpoint(
                shared,
                endpoint,
                socket,
                sender,
                receiver,
                config.remote_addr,
                initial_session,
                security,
            )
            .await;
        });
        self.set_endpoint_abort(endpoint, task.abort_handle())?;
        Ok(endpoint)
    }

    fn open_secure_kcp(
        &self,
        config: EndpointConfig,
        security: EndpointSecurity,
    ) -> Result<Handle> {
        let bind_addr = config.bind_addr.ok_or_else(|| {
            RnetError::new(ErrorCode::InvalidArgument, "secure KCP requires bind_addr")
        })?;
        let socket = Arc::new(self.runtime.block_on(UdpSocket::bind(bind_addr))?);
        let endpoint = self.insert_endpoint(socket.local_addr()?, config.transport)?;
        let (sender, receiver) = mpsc::channel(self.shared.config.write_queue_capacity);
        let initial_session = match &security {
            EndpointSecurity::Client { .. } => {
                let peer = config.remote_addr.ok_or_else(|| {
                    RnetError::new(
                        ErrorCode::InvalidArgument,
                        "secure KCP client requires peer",
                    )
                })?;
                let session = self.insert_pending_session(
                    endpoint,
                    SessionTarget::Kcp {
                        sender: sender.clone(),
                        peer,
                        cleanup: None,
                    },
                    None,
                );
                match session {
                    Ok(session) => Some(session),
                    Err(error) => {
                        self.discard_endpoint(endpoint);
                        return Err(error);
                    }
                }
            }
            EndpointSecurity::Server { .. } => None,
            _ => {
                return Err(RnetError::new(
                    ErrorCode::InvalidArgument,
                    "invalid KCP security role",
                ))
            }
        };
        if let Err(error) = self.push_endpoint_opened(endpoint) {
            if let Some(session) = initial_session {
                self.discard_session(session);
            }
            self.discard_endpoint(endpoint);
            return Err(error);
        }
        let shared = Arc::clone(&self.shared);
        let task = self.runtime.spawn(async move {
            run_secure_kcp_endpoint(
                shared,
                endpoint,
                socket,
                sender,
                receiver,
                config.remote_addr,
                initial_session,
                security,
            )
            .await;
        });
        self.set_endpoint_abort(endpoint, task.abort_handle())?;
        Ok(endpoint)
    }

    fn open_tcp_listener(&self, config: EndpointConfig) -> Result<Handle> {
        let bind_addr = config.bind_addr.ok_or_else(|| {
            RnetError::new(
                ErrorCode::InvalidArgument,
                "TCP listener requires bind_addr",
            )
        })?;
        let listener = self.runtime.block_on(TcpListener::bind(bind_addr))?;
        let local_addr = listener.local_addr()?;
        let endpoint = self.insert_endpoint(local_addr, config.transport)?;
        if let Err(error) = self.push_endpoint_opened(endpoint) {
            self.discard_endpoint(endpoint);
            return Err(error);
        }
        let shared = Arc::clone(&self.shared);
        let task = self.runtime.spawn(async move {
            run_tcp_listener(shared, endpoint, listener).await;
        });
        self.set_endpoint_abort(endpoint, task.abort_handle())?;
        Ok(endpoint)
    }

    fn open_tcp_client(&self, config: EndpointConfig) -> Result<Handle> {
        let remote_addr = config.remote_addr.ok_or_else(|| {
            RnetError::new(
                ErrorCode::InvalidArgument,
                "TCP client requires remote_addr",
            )
        })?;
        let connect_started = Instant::now();
        let stream =
            std::net::TcpStream::connect_timeout(&remote_addr, self.shared.config.connect_timeout)?;
        configure_std_tcp(&stream, &self.shared.config)?;
        stream.set_nonblocking(true)?;
        let stream = {
            let _entered = self.runtime.enter();
            TcpStream::from_std(stream)?
        };
        self.shared
            .latencies
            .record(LatencyKind::Connect, connect_started.elapsed());
        let local_addr = stream.local_addr()?;
        let endpoint = self.insert_endpoint(local_addr, config.transport)?;
        if let Err(error) = self.push_endpoint_opened(endpoint) {
            self.discard_endpoint(endpoint);
            return Err(error);
        }
        let (sender, receiver) = mpsc::channel(self.shared.config.write_queue_capacity);
        let session = self.insert_session(endpoint, SessionTarget::Tcp(sender));
        let shared = Arc::clone(&self.shared);
        let task = self.runtime.spawn(async move {
            push_tcp_event(
                &shared,
                session_event(EventType::SessionOpened, endpoint, session),
            )
            .await;
            if shared.state.load() == Lifecycle::Running {
                run_tcp_session(shared, endpoint, session, stream, receiver).await;
            }
        });
        self.set_endpoint_abort(endpoint, task.abort_handle())?;
        Ok(endpoint)
    }

    fn open_udp(&self, config: EndpointConfig) -> Result<Handle> {
        let bind_addr = config.bind_addr.ok_or_else(|| {
            RnetError::new(
                ErrorCode::InvalidArgument,
                "UDP endpoint requires bind_addr",
            )
        })?;
        let socket = Arc::new(self.runtime.block_on(UdpSocket::bind(bind_addr))?);
        let local_addr = socket.local_addr()?;
        let endpoint = self.insert_endpoint(local_addr, config.transport)?;
        if let Err(error) = self.push_endpoint_opened(endpoint) {
            self.discard_endpoint(endpoint);
            return Err(error);
        }
        let (sender, receiver) = mpsc::channel(self.shared.config.write_queue_capacity);
        let peers = Arc::new(Mutex::new(HashMap::new()));
        let initial_session = if let Some(peer) = config.remote_addr {
            let session = self.insert_session(
                endpoint,
                SessionTarget::Udp {
                    sender: sender.clone(),
                    peer,
                    max_frame_len: self.shared.config.max_datagram_size,
                    cleanup: None,
                },
            );
            peers
                .lock()
                .expect("UDP peer map poisoned")
                .insert(peer, session);
            Some(session)
        } else {
            None
        };
        let shared = Arc::clone(&self.shared);
        let task = self.runtime.spawn(async move {
            run_udp_endpoint(
                shared,
                endpoint,
                socket,
                sender,
                receiver,
                peers,
                initial_session,
            )
            .await;
        });
        self.set_endpoint_abort(endpoint, task.abort_handle())?;
        Ok(endpoint)
    }
}
