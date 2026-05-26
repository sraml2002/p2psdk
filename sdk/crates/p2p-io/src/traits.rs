//! Platform I/O abstraction traits.
//!
//! All platform-specific I/O is abstracted through these traits, enabling:
//! - ArkTS/NAPI bridge: inject callbacks from HarmonyOS platform APIs
//! - Tokio: real network I/O for standalone Rust applications

use std::result::Result;

/// Error type for I/O operations.
#[derive(Debug, thiserror::Error)]
pub enum IoError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("timeout")]
    Timeout,
    #[error("connection closed")]
    Closed,
    #[error("HTTP error {0}: {1}")]
    Http(u16, String),
    #[error("{0}")]
    Other(String),
}

/// UDP datagram transport abstraction.
pub trait UdpTransport: Send + Sync {
    /// Send a datagram to the specified address.
    fn send_to(&self, data: &[u8], addr: &str, port: u16) -> Result<(), IoError>;

    /// Receive a datagram with timeout in milliseconds.
    /// Returns the received data and the sender's address (ip, port).
    fn recv_from(&self, timeout_ms: u64) -> Result<(Vec<u8>, String, u16), IoError>;

    /// Get the local address this socket is bound to.
    fn local_addr(&self) -> Result<(String, u16), IoError>;

    /// Close the socket.
    fn close(&self);
}

/// Result of a DTLS handshake step (buffer-based I/O, no real socket).
#[derive(Debug)]
pub enum DtlsStepResult {
    /// Data is available to send to the peer.
    DataToSend(Vec<u8>),
    /// Waiting for more data from the peer.
    NeedsMoreData,
    /// Handshake completed successfully.
    Complete,
}

/// DTLS session abstraction with buffer-based I/O.
///
/// Mirrors the dtls_client.h C API:
/// - `handshake_step(incoming)` → DtlsStepResult
/// - `encrypt(plaintext)` → ciphertext
/// - `decrypt(ciphertext)` → plaintext
pub trait DtlsTransport: Send + Sync {
    /// Perform one handshake step.
    /// `incoming` is the received UDP data (None on first step or retry).
    fn handshake_step(&mut self, incoming: Option<&[u8]>) -> Result<DtlsStepResult, IoError>;

    /// Encrypt application data (after handshake complete).
    fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, IoError>;

    /// Decrypt received data (after handshake complete).
    fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, IoError>;

    /// Destroy the session and free resources.
    fn destroy(&mut self);
}

/// WebSocket signaling transport abstraction.
pub trait SignalingTransport: Send + Sync {
    /// Connect to the signaling server.
    fn connect(&mut self, url: &str) -> Result<(), IoError>;

    /// Send a text message.
    fn send(&self, message: &str) -> Result<(), IoError>;

    /// Try to receive a message (non-blocking).
    fn try_recv(&mut self) -> Option<String>;

    /// Close the connection.
    fn close(&mut self);

    /// Whether currently connected.
    fn is_connected(&self) -> bool;
}

/// HTTP client abstraction.
pub trait HttpTransport: Send + Sync {
    /// Send a POST request. Returns (status_code, response_body).
    fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Result<(u16, String), IoError>;

    /// Send a GET request. Returns (status_code, response_body).
    fn get(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<(u16, String), IoError>;
}

/// Platform-specific operations that cannot be abstracted by network traits.
pub trait Platform: Send + Sync {
    /// Get local network IP addresses.
    fn get_local_addresses(&self) -> Vec<String>;

    /// Generate cryptographically random bytes.
    fn random_bytes(&self, len: usize) -> Vec<u8>;

    /// Log a message.
    fn log(&self, tag: &str, msg: &str);

    /// Get current time in milliseconds since epoch.
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

/// Factory trait for creating platform I/O resources.
///
/// Injected internally by `P2pClient::init` so the SDK can create
/// UDP sockets and HTTP clients internally without depending on a
/// specific platform implementation.
pub trait IoProvider: Send + Sync {
    /// Create a UDP socket bound to `0.0.0.0:<port>` (IPv4).
    fn create_udp(&self, port: u16) -> Result<Box<dyn UdpTransport>, IoError>;

    /// Create a UDP socket bound to `[::]:<port>` (IPv6).
    fn create_udp_v6(&self, port: u16) -> Result<Box<dyn UdpTransport>, IoError>;

    /// Create an HTTP client.
    fn create_http(&self) -> Box<dyn HttpTransport>;

    /// Get local network IP addresses.
    fn get_local_addresses(&self) -> Vec<String>;
}
