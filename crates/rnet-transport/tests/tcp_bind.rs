use rnet_core::{ErrorCode, Transport};
use rnet_security::Keypair;
use rnet_transport::{ClientConfig, ClientSecurity, NetworkRuntime, RuntimeConfig};
use std::net::{SocketAddr, TcpListener};
use std::time::Duration;

fn runtime() -> NetworkRuntime {
    let key = Keypair::generate().unwrap();
    NetworkRuntime::new_with_client_security(
        RuntimeConfig {
            worker_threads: 1,
            ..RuntimeConfig::production()
        },
        Some(ClientSecurity::pinned(key.clone(), key.public.clone())),
    )
    .unwrap()
}

fn config(remote_addr: SocketAddr, bind_addr: SocketAddr) -> ClientConfig {
    ClientConfig {
        transport: Transport::Tcp,
        bind_addr: Some(bind_addr),
        remote_addr,
        join_payload: Vec::new(),
    }
}

#[test]
fn tcp_explicit_local_port_is_used_for_ipv4_and_ipv6() {
    for address in ["127.0.0.1:0", "[::1]:0"] {
        let server = TcpListener::bind(address).unwrap();
        let reservation = TcpListener::bind(address).unwrap();
        let local = reservation.local_addr().unwrap();
        let runtime = runtime();
        // Discover an OS-chosen free port; the connector must claim that port, not choose another.
        drop(reservation);
        let endpoint = runtime
            .connect(config(server.local_addr().unwrap(), local))
            .unwrap();
        assert_eq!(runtime.endpoint_local_addr(endpoint).unwrap(), local);
        let (_, observed_peer) = server.accept().unwrap();
        assert_eq!(
            observed_peer, local,
            "verify the actual socket, not only registry metadata"
        );
        runtime.stop(Duration::ZERO).unwrap();
    }
}

#[test]
fn tcp_busy_local_port_is_rejected_without_fallback_or_endpoint_leak() {
    for address in ["127.0.0.1:0", "[::1]:0"] {
        let server = TcpListener::bind(address).unwrap();
        server.set_nonblocking(true).unwrap();
        let occupied = TcpListener::bind(address).unwrap();
        let runtime = runtime();
        let error = runtime
            .connect(config(
                server.local_addr().unwrap(),
                occupied.local_addr().unwrap(),
            ))
            .expect_err("binding failure must not silently use an ephemeral local port");
        assert_eq!(error.code(), ErrorCode::IoError);
        let metrics = runtime.metrics_snapshot();
        assert_eq!(metrics.current_endpoints, 0);
        assert_eq!(metrics.current_sessions, 0);
        assert_eq!(metrics.pending_handshakes, 0);
        assert_eq!(
            server.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

#[test]
fn tcp_mismatched_local_address_family_is_rejected_before_connect() {
    for (remote, local) in [("127.0.0.1:0", "[::1]:0"), ("[::1]:0", "127.0.0.1:0")] {
        let server = TcpListener::bind(remote).unwrap();
        server.set_nonblocking(true).unwrap();
        let runtime = runtime();
        assert_eq!(
            runtime
                .connect(config(server.local_addr().unwrap(), local.parse().unwrap(),))
                .unwrap_err()
                .code(),
            ErrorCode::InvalidArgument
        );
        assert_eq!(runtime.metrics_snapshot().current_endpoints, 0);
        assert_eq!(
            server.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
