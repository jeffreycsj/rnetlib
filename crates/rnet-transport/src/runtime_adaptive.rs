//! Endpoint construction for the unified server/client security protocol.

use crate::adaptive_datagram::run_adaptive_datagram;
use crate::adaptive_tcp::{run_adaptive_tcp_client, run_adaptive_tcp_listener};
use crate::config::{EndpointConfig, EndpointSecurity};
use crate::metrics::LatencyKind;
use crate::runtime::NetworkRuntime;
use crate::state::SessionTarget;
use crate::tcp_socket::configure_std_tcp;
use rnet_core::{ErrorCode, Handle, Result, RnetError, Transport};
use rnet_security::Keypair;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc;

impl NetworkRuntime {
    pub(crate) fn open_adaptive_datagram(
        &self,
        config: EndpointConfig,
        security: EndpointSecurity,
    ) -> Result<Handle> {
        self.open_adaptive_datagram_candidates(config, security, Vec::new())
    }

    pub(crate) fn open_adaptive_datagram_candidates(
        &self,
        config: EndpointConfig,
        security: EndpointSecurity,
        remote_candidates: Vec<std::net::SocketAddr>,
    ) -> Result<Handle> {
        let bind_addr = config.bind_addr.ok_or_else(|| {
            RnetError::new(
                ErrorCode::InvalidArgument,
                "datagram endpoint requires bind_addr",
            )
        })?;
        let socket = Arc::new(self.runtime.block_on(UdpSocket::bind(bind_addr))?);
        let endpoint = self.insert_endpoint(socket.local_addr()?, config.transport)?;
        let (sender, receiver) = mpsc::channel(self.shared.config.write_queue_capacity);
        let (cleanup_sender, cleanup_receiver) = mpsc::unbounded_channel();
        let initial_session = if matches!(security, EndpointSecurity::AdaptiveClient { .. }) {
            let peer = config.remote_addr.ok_or_else(|| {
                RnetError::new(ErrorCode::InvalidArgument, "client requires remote_addr")
            })?;
            let target = match config.transport {
                Transport::Kcp => SessionTarget::Kcp {
                    sender: sender.clone(),
                    peer,
                    cleanup: Some(cleanup_sender.clone()),
                },
                _ => SessionTarget::Udp {
                    sender: sender.clone(),
                    peer,
                    max_frame_len: self
                        .shared
                        .config
                        .max_datagram_size
                        .saturating_sub(crate::adaptive_codec::ENCRYPTED_UDP_WIRE_OVERHEAD),
                    cleanup: Some(cleanup_sender.clone()),
                },
            };
            match self.insert_pending_session(endpoint, target, None, true) {
                Ok(session) => Some(session),
                Err(error) => {
                    self.discard_endpoint(endpoint);
                    return Err(error);
                }
            }
        } else {
            None
        };
        if let Err(error) = self.push_endpoint_opened(endpoint) {
            if let Some(session) = initial_session {
                self.discard_session(session);
            }
            self.discard_endpoint(endpoint);
            return Err(error);
        }
        let shared = Arc::clone(&self.shared);
        let client_security = shared.client_security.clone();
        let transport = config.transport;
        let remote = config.remote_addr;
        let connect_deadline =
            matches!(security, EndpointSecurity::AdaptiveClient { .. }).then(|| {
                Instant::now()
                    .checked_add(shared.config.connect_timeout)
                    .expect("validated connection timeout")
            });
        let task = self.runtime.spawn(async move {
            run_adaptive_datagram(
                shared,
                endpoint,
                socket,
                sender,
                receiver,
                cleanup_sender,
                cleanup_receiver,
                remote,
                initial_session,
                security,
                client_security,
                transport,
                remote_candidates,
                connect_deadline,
            )
            .await;
        });
        self.set_endpoint_abort(endpoint, task.abort_handle())?;
        Ok(endpoint)
    }

    pub(crate) fn open_adaptive_tcp_listener(
        &self,
        config: EndpointConfig,
        local_key: Keypair,
        initial_mode: rnet_protocol::control::SecurityMode,
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
            run_adaptive_tcp_listener(shared, endpoint, listener, local_key, initial_mode).await;
        });
        self.set_endpoint_abort(endpoint, task.abort_handle())?;
        Ok(endpoint)
    }

    pub(crate) fn open_adaptive_tcp_client(
        &self,
        config: EndpointConfig,
        join_payload: Vec<u8>,
    ) -> Result<Handle> {
        self.open_adaptive_tcp_client_with_timeout(
            config,
            join_payload,
            self.shared.config.connect_timeout,
        )
    }

    pub(crate) fn open_adaptive_tcp_client_with_timeout(
        &self,
        config: EndpointConfig,
        join_payload: Vec<u8>,
        connect_timeout: std::time::Duration,
    ) -> Result<Handle> {
        let client_security = self.shared.client_security.clone().ok_or_else(|| {
            RnetError::new(
                ErrorCode::InvalidState,
                "unified client connection requires runtime client security",
            )
        })?;
        let remote_addr = config.remote_addr.ok_or_else(|| {
            RnetError::new(
                ErrorCode::InvalidArgument,
                "TCP client requires remote_addr",
            )
        })?;
        let connect_started = Instant::now();
        let stream = std::net::TcpStream::connect_timeout(&remote_addr, connect_timeout)?;
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
        let session =
            match self.insert_pending_session(endpoint, SessionTarget::Tcp(sender), None, true) {
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
            run_adaptive_tcp_client(
                shared,
                endpoint,
                session,
                stream,
                receiver,
                client_security,
                join_payload,
            )
            .await;
        });
        self.set_endpoint_abort(endpoint, task.abort_handle())?;
        Ok(endpoint)
    }
}
