//! Centralized TCP socket tuning shared by compatibility and authenticated paths.

use crate::config::RuntimeConfig;
use rnet_core::Result;
use socket2::SockRef;

pub(crate) fn configure_std_tcp(
    stream: &std::net::TcpStream,
    config: &RuntimeConfig,
) -> Result<()> {
    stream.set_nodelay(config.tcp_nodelay)?;
    configure_buffers(SockRef::from(stream), config)
}

pub(crate) fn configure_tokio_tcp(
    stream: &tokio::net::TcpStream,
    config: &RuntimeConfig,
) -> Result<()> {
    stream.set_nodelay(config.tcp_nodelay)?;
    configure_buffers(SockRef::from(stream), config)
}

fn configure_buffers(socket: SockRef<'_>, config: &RuntimeConfig) -> Result<()> {
    if let Some(bytes) = config.tcp_send_buffer_bytes {
        socket.set_send_buffer_size(bytes)?;
    }
    if let Some(bytes) = config.tcp_recv_buffer_bytes {
        socket.set_recv_buffer_size(bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::configure_std_tcp;
    use crate::config::RuntimeConfig;
    use socket2::SockRef;
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn applies_nodelay_and_requested_kernel_buffers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_server, _) = listener.accept().unwrap();
        let config = RuntimeConfig {
            tcp_nodelay: true,
            tcp_send_buffer_bytes: Some(64 * 1024),
            tcp_recv_buffer_bytes: Some(64 * 1024),
            ..RuntimeConfig::default()
        };

        configure_std_tcp(&client, &config).unwrap();

        let socket = SockRef::from(&client);
        assert!(client.nodelay().unwrap());
        assert!(socket.send_buffer_size().unwrap() >= 64 * 1024);
        assert!(socket.recv_buffer_size().unwrap() >= 64 * 1024);
    }
}
