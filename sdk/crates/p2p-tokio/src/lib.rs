//! Synchronous I/O implementations for standalone Rust applications.
//!
//! Uses std::net for UDP, reqwest::blocking for HTTP, tungstenite for WebSocket.
//! All trait methods are synchronous — the caller drives the event loop.

pub mod udp;
pub mod http;
pub mod ws;
pub mod platform;

pub use udp::SyncUdpTransport;
pub use http::SyncHttpTransport;
pub use ws::SyncSignalingTransport;
pub use platform::StdPlatform;

use p2p_io::traits::{HttpTransport, IoError, IoProvider, Platform, UdpTransport};

/// Synchronous I/O provider backed by std::net / reqwest / StdPlatform.
pub struct SyncIoProvider;

impl IoProvider for SyncIoProvider {
    fn create_udp(&self, port: u16) -> Result<Box<dyn UdpTransport>, IoError> {
        SyncUdpTransport::bind_any(port).map(|u| Box::new(u) as Box<dyn UdpTransport>)
    }

    fn create_udp_v6(&self, port: u16) -> Result<Box<dyn UdpTransport>, IoError> {
        SyncUdpTransport::bind_any_v6(port).map(|u| Box::new(u) as Box<dyn UdpTransport>)
    }

    fn create_http(&self) -> Box<dyn HttpTransport> {
        Box::new(SyncHttpTransport::new())
    }

    fn get_local_addresses(&self) -> Vec<String> {
        StdPlatform::new().get_local_addresses()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use p2p_io::traits::{UdpTransport, SignalingTransport, Platform};

    #[test]
    fn test_udp_bind_any() {
        let udp = SyncUdpTransport::bind_any(0);
        assert!(udp.is_ok());
        let udp = udp.unwrap();
        let (_, port) = udp.local_addr().unwrap();
        assert!(port > 0);
    }

    #[test]
    fn test_udp_send_recv_loopback() {
        let server = SyncUdpTransport::bind_any(0).unwrap();
        let (_, server_port) = server.local_addr().unwrap();

        let client = SyncUdpTransport::bind_any(0).unwrap();
        let (_client_ip, client_port) = client.local_addr().unwrap();

        // Client sends to server
        client.send_to(b"hello", "127.0.0.1", server_port).unwrap();

        // Server receives
        let (data, from_ip, from_port) = server.recv_from(2000).unwrap();
        assert_eq!(&data, b"hello");
        // from_ip may be "127.0.0.1" even though client binds to 0.0.0.0
        assert_eq!(from_port, client_port);

        // Server responds
        server.send_to(b"world", &from_ip, from_port).unwrap();

        // Client receives
        let (data, _, _) = client.recv_from(2000).unwrap();
        assert_eq!(&data, b"world");
    }

    #[test]
    fn test_udp_recv_timeout() {
        let udp = SyncUdpTransport::bind_any(0).unwrap();
        let result = udp.recv_from(100);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), p2p_io::traits::IoError::Timeout));
    }

    #[test]
    fn test_http_new() {
        let _http = SyncHttpTransport::new();
    }

    #[test]
    fn test_ws_new() {
        let _ws = SyncSignalingTransport::new();
        assert!(!_ws.is_connected());
    }

    #[test]
    fn test_platform_local_addresses() {
        let platform = StdPlatform::new();
        let addrs = platform.get_local_addresses();
        assert!(!addrs.is_empty());
    }

    #[test]
    fn test_platform_random_bytes() {
        let platform = StdPlatform::new();
        let bytes1 = platform.random_bytes(32);
        let bytes2 = platform.random_bytes(32);
        assert_eq!(bytes1.len(), 32);
        assert_eq!(bytes2.len(), 32);
        assert_ne!(bytes1, bytes2);
    }

    #[test]
    fn test_platform_now_ms() {
        let platform = StdPlatform::new();
        let t1 = platform.now_ms();
        assert!(t1 > 0);
    }
}
