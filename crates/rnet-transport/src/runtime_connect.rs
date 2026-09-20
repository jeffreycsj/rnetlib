//! Host-name resolution and multi-address client connection entry points.

use crate::address::resolve_host;
use crate::config::EndpointConfig;
use crate::runtime::NetworkRuntime;
use rnet_core::{ErrorCode, Handle, Result, RnetError, Transport};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

impl NetworkRuntime {
    /// Resolves a DNS host name or textual IP and starts a transport-neutral client connection.
    pub fn connect_host(&self, config: crate::config::HostClientConfig) -> Result<Handle> {
        if config.host.is_empty() || config.port == 0 {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "client host and port must be set",
            ));
        }
        self.validate_join_payload(&config.join_payload)?;
        let operation_started = Instant::now();
        let addresses = resolve_host(&config.host, config.port, self.shared.config.dns_timeout)?;

        if config.transport != Transport::Tcp {
            let first = addresses[0];
            let remaining = addresses.iter().copied().skip(1).collect();
            let endpoint_config: EndpointConfig = crate::config::ClientConfig {
                transport: config.transport,
                bind_addr: Some(SocketAddr::new(
                    if first.is_ipv4() {
                        std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
                    } else {
                        std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
                    },
                    0,
                )),
                remote_addr: first,
                join_payload: config.join_payload,
            }
            .into();
            let security = endpoint_config
                .security
                .clone()
                .expect("client security mode");
            return self.open_adaptive_datagram_candidates(endpoint_config, security, remaining);
        }

        let total_timeout = self
            .shared
            .config
            .dns_timeout
            .saturating_add(self.shared.config.connect_timeout);
        self.connect_tcp_candidates(
            addresses,
            &config.join_payload,
            operation_started,
            total_timeout,
        )
    }

    /// Connects using caller-resolved addresses in order, retrying datagram handshakes in place.
    pub fn connect_resolved(&self, config: crate::config::ResolvedClientConfig) -> Result<Handle> {
        self.validate_join_payload(&config.join_payload)?;
        let first = *config.remote_addrs.first().ok_or_else(|| {
            RnetError::new(
                ErrorCode::InvalidArgument,
                "at least one remote address is required",
            )
        })?;
        if config.transport == Transport::Tcp {
            return self.connect_tcp_candidates(
                config.remote_addrs,
                &config.join_payload,
                Instant::now(),
                self.shared.config.connect_timeout,
            );
        }
        let remaining = config.remote_addrs.into_iter().skip(1).collect();
        let endpoint_config: EndpointConfig = crate::config::ClientConfig {
            transport: config.transport,
            bind_addr: Some(SocketAddr::new(
                if first.is_ipv4() {
                    std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
                } else {
                    std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
                },
                0,
            )),
            remote_addr: first,
            join_payload: config.join_payload,
        }
        .into();
        let security = endpoint_config
            .security
            .clone()
            .expect("client security mode");
        self.open_adaptive_datagram_candidates(endpoint_config, security, remaining)
    }

    fn validate_join_payload(&self, payload: &[u8]) -> Result<()> {
        if payload.len() > self.shared.config.max_body_len.min(60 * 1024) {
            return Err(RnetError::new(
                ErrorCode::MessageTooLarge,
                "join payload exceeds the configured handshake limit",
            ));
        }
        Ok(())
    }

    fn connect_tcp_candidates(
        &self,
        addresses: Vec<SocketAddr>,
        join_payload: &[u8],
        started: Instant,
        total_timeout: Duration,
    ) -> Result<Handle> {
        let mut last_error = None;
        for remote_addr in addresses {
            let remaining = total_timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(RnetError::new(
                    ErrorCode::Timeout,
                    "TCP multi-address connection deadline expired",
                ));
            }
            let attempt_timeout = remaining.min(self.shared.config.connect_timeout);
            let endpoint_config: EndpointConfig = crate::config::ClientConfig {
                transport: Transport::Tcp,
                bind_addr: None,
                remote_addr,
                join_payload: join_payload.to_vec(),
            }
            .into();
            match self.open_adaptive_tcp_client_with_timeout(
                endpoint_config,
                join_payload.to_vec(),
                attempt_timeout,
            ) {
                Ok(endpoint) => return Ok(endpoint),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error
            .unwrap_or_else(|| RnetError::new(ErrorCode::IoError, "all resolved addresses failed")))
    }
}
