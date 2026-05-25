//! DTLS 1.2 client session with buffer-based I/O.
//!
//! Wraps `dimpl` (Sans-IO DTLS) to replicate the buffer-based model from
//! `dtls_client.c`: no real sockets, caller feeds UDP datagrams in and
//! retrieves outgoing datagrams out.
//!
//! - DTLS 1.2 only (Huawei STUN server compatibility).
//! - Client mode, no certificate verification.

use std::sync::Arc;
use std::time::Instant;

use dimpl::{certificate, Config, Dtls, Output};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from DTLS session operations.
#[derive(Debug)]
pub enum DtlsError {
    HandshakeFailed(String),
    EncryptFailed(String),
    DecryptFailed(String),
    NotConnected,
    Destroyed,
    Internal(String),
}

impl std::fmt::Display for DtlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HandshakeFailed(msg) => write!(f, "DTLS handshake failed: {msg}"),
            Self::EncryptFailed(msg) => write!(f, "DTLS encrypt failed: {msg}"),
            Self::DecryptFailed(msg) => write!(f, "DTLS decrypt failed: {msg}"),
            Self::NotConnected => write!(f, "DTLS session not connected"),
            Self::Destroyed => write!(f, "DTLS session destroyed"),
            Self::Internal(msg) => write!(f, "DTLS internal error: {msg}"),
        }
    }
}

impl std::error::Error for DtlsError {}

// ---------------------------------------------------------------------------
// Handshake step result
// ---------------------------------------------------------------------------

/// Result of a single DTLS handshake step.
#[derive(Debug)]
pub enum HandshakeStep {
    /// Encrypted data that must be sent to the peer over UDP.
    DataToSend(Vec<u8>),
    /// No data to send; waiting for an incoming packet or timer expiry.
    Waiting,
    /// Handshake completed successfully.
    Complete,
}

// ---------------------------------------------------------------------------
// DtlsSession
// ---------------------------------------------------------------------------

const OUTPUT_BUF_SIZE: usize = 65536;

/// DTLS 1.2 client session with buffer-based I/O.
pub struct DtlsSession {
    dtls: Option<Dtls>,
    connected: bool,
    start_instant: Instant,
}

impl DtlsSession {
    /// Create a new DTLS 1.2 client session.
    ///
    /// Generates a self-signed certificate internally and configures DTLS 1.2
    /// client mode with no peer certificate verification.
    pub fn new() -> Result<Self, DtlsError> {
        let cert = certificate::generate_self_signed_certificate()
            .map_err(|e| DtlsError::Internal(format!("cert generation: {e}")))?;

        let config = Arc::new(
            Config::builder()
                .build()
                .map_err(|e| DtlsError::Internal(format!("config: {e}")))?,
        );

        let start = Instant::now();
        let mut dtls = Dtls::new_12(config, cert, start);
        dtls.set_active(true);

        Ok(Self {
            dtls: Some(dtls),
            connected: false,
            start_instant: start,
        })
    }

    /// Perform one handshake step.
    ///
    /// - `incoming`: UDP datagram received from the peer (`None` on first call
    ///   or timer expiry).
    pub fn handshake_step(&mut self, incoming: Option<&[u8]>) -> Result<HandshakeStep, DtlsError> {
        let dtls = self.dtls.as_mut().ok_or(DtlsError::Destroyed)?;

        if let Some(data) = incoming {
            dtls.handle_packet(data)
                .map_err(|e| DtlsError::HandshakeFailed(e.to_string()))?;
        }

        let mut data_to_send = Vec::new();
        let mut out_buf = [0u8; OUTPUT_BUF_SIZE];
        let mut timeout_fired = false;

        loop {
            match dtls.poll_output(&mut out_buf) {
                Output::Packet(p) => data_to_send.extend_from_slice(p),
                Output::Connected => {
                    self.connected = true;
                }
                Output::PeerCert(_) => {}
                Output::Timeout(deadline) => {
                    // If we have no output yet and haven't fired a timeout,
                    // advance the clock to trigger the initial flight
                    // (e.g. ClientHello).
                    if data_to_send.is_empty() && !self.connected && !timeout_fired {
                        let _ = dtls.handle_timeout(deadline);
                        timeout_fired = true;
                        continue;
                    }
                    break;
                }
                Output::CloseNotify | _ => break,
            }
        }

        if self.connected {
            if data_to_send.is_empty() {
                Ok(HandshakeStep::Complete)
            } else {
                // Final flight still needs sending; caller sends it then
                // calls handshake_step(None) to get Complete.
                Ok(HandshakeStep::DataToSend(data_to_send))
            }
        } else if !data_to_send.is_empty() {
            Ok(HandshakeStep::DataToSend(data_to_send))
        } else {
            Ok(HandshakeStep::Waiting)
        }
    }

    /// Notify the session that a retransmission timer has fired.
    pub fn handle_timeout(&mut self) -> Result<(), DtlsError> {
        let dtls = self.dtls.as_mut().ok_or(DtlsError::Destroyed)?;
        let now = Instant::now();
        dtls.handle_timeout(now)
            .map_err(|e| DtlsError::HandshakeFailed(e.to_string()))?;
        Ok(())
    }

    /// Encrypt application data (must be called after handshake completes).
    ///
    /// Returns the encrypted DTLS record ready to send over UDP.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, DtlsError> {
        let dtls = self.dtls.as_mut().ok_or(DtlsError::Destroyed)?;

        if !self.connected {
            return Err(DtlsError::NotConnected);
        }

        dtls.send_application_data(plaintext)
            .map_err(|e| DtlsError::EncryptFailed(e.to_string()))?;

        let mut out_buf = [0u8; OUTPUT_BUF_SIZE];
        match dtls.poll_output(&mut out_buf) {
            Output::Packet(p) => Ok(p.to_vec()),
            _ => Err(DtlsError::EncryptFailed("no output after send".into())),
        }
    }

    /// Decrypt a received DTLS record (must be called after handshake completes).
    ///
    /// Returns the decrypted application data.
    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, DtlsError> {
        let dtls = self.dtls.as_mut().ok_or(DtlsError::Destroyed)?;

        if !self.connected {
            return Err(DtlsError::NotConnected);
        }

        dtls.handle_packet(ciphertext)
            .map_err(|e| DtlsError::DecryptFailed(e.to_string()))?;

        let mut out_buf = [0u8; OUTPUT_BUF_SIZE];
        match dtls.poll_output(&mut out_buf) {
            Output::ApplicationData(p) => Ok(p.to_vec()),
            _ => Err(DtlsError::DecryptFailed("no application data".into())),
        }
    }

    /// Send a DTLS close_notify alert.
    pub fn close(&mut self) {
        if let Some(dtls) = self.dtls.as_mut() {
            let _ = dtls.close();
        }
        self.connected = false;
    }

    /// Whether the DTLS handshake has completed.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Elapsed time since session creation.
    pub fn elapsed(&self) -> std::time::Duration {
        self.start_instant.elapsed()
    }
}

impl Drop for DtlsSession {
    fn drop(&mut self) {
        self.close();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_create() {
        let session = DtlsSession::new();
        assert!(session.is_ok(), "DtlsSession::new() should succeed");
        let session = session.unwrap();
        assert!(!session.is_connected());
    }

    #[test]
    fn test_handshake_first_step_produces_client_hello() {
        let mut client = DtlsSession::new().unwrap();
        // First step with no incoming data should produce ClientHello
        let result = client.handshake_step(None).unwrap();
        match result {
            HandshakeStep::DataToSend(data) => {
                assert!(!data.is_empty(), "ClientHello should not be empty");
                // DTLS 1.2 record: content_type(1) + version(2) + epoch(2) + seq(6) + len(2) + ...
                assert!(data.len() > 13, "DTLS record should have header");
            }
            other => panic!("Expected DataToSend, got {:?}", other),
        }
    }

    #[test]
    fn test_encrypt_before_connected_fails() {
        let mut session = DtlsSession::new().unwrap();
        let result = session.encrypt(b"hello");
        assert!(result.is_err());
        match result.unwrap_err() {
            DtlsError::NotConnected => {}
            e => panic!("Expected NotConnected, got {}", e),
        }
    }

    #[test]
    fn test_decrypt_before_connected_fails() {
        let mut session = DtlsSession::new().unwrap();
        let result = session.decrypt(b"\x00\x00");
        assert!(result.is_err());
        match result.unwrap_err() {
            DtlsError::NotConnected => {}
            e => panic!("Expected NotConnected, got {}", e),
        }
    }

    #[test]
    fn test_close() {
        let mut session = DtlsSession::new().unwrap();
        assert!(!session.is_connected());
        session.close();
        assert!(!session.is_connected());
    }

    /// Simulate a full DTLS handshake between client and server.
    #[test]
    fn test_full_handshake_loopback() {
        let cert = certificate::generate_self_signed_certificate().unwrap();
        let config = Arc::new(Config::builder().build().unwrap());

        // Client
        let start = Instant::now();
        let mut client_dtls = Dtls::new_12(config.clone(), cert.clone(), start);
        client_dtls.set_active(true);

        // Server
        let mut server_dtls = Dtls::new_12(config, cert, start);

        let mut client_connected = false;
        let mut server_connected = false;

        // Drive the handshake by exchanging packets
        for _ in 0..20 {
            if client_connected && server_connected {
                break;
            }

            let mut out_buf = [0u8; OUTPUT_BUF_SIZE];

            // Client → Server
            loop {
                match client_dtls.poll_output(&mut out_buf) {
                    Output::Packet(p) => {
                        server_dtls.handle_packet(p).unwrap();
                    }
                    Output::Connected => {
                        client_connected = true;
                    }
                    Output::PeerCert(_) => {}
                    _ => break,
                }
            }

            // Server → Client
            loop {
                match server_dtls.poll_output(&mut out_buf) {
                    Output::Packet(p) => {
                        client_dtls.handle_packet(p).unwrap();
                    }
                    Output::Connected => {
                        server_connected = true;
                    }
                    Output::PeerCert(_) => {}
                    _ => break,
                }
            }

            // Advance time for retransmission
            let now = start + std::time::Duration::from_millis(100);
            let _ = client_dtls.handle_timeout(now);
            let _ = server_dtls.handle_timeout(now);
        }

        assert!(client_connected, "client should be connected");
        assert!(server_connected, "server should be connected");
    }

    /// Simulate DTLS handshake + encrypt/decrypt through the DtlsSession API
    /// (the same path used by stun/client.rs dtls_stun_exchange).
    #[test]
    fn test_dtls_session_encrypt_decrypt_loopback() {
        let cert = certificate::generate_self_signed_certificate().unwrap();
        let config = Arc::new(Config::builder().build().unwrap());

        // Client (DtlsSession — the real API used in STUN queries)
        let mut client = DtlsSession::new().unwrap();

        // Server (raw dimpl, simulates STUN server DTLS endpoint)
        let start = Instant::now();
        let mut server_dtls = Dtls::new_12(config, cert, start);

        // dimpl requires handle_timeout to initialize server state (random etc.)
        let init_now = start + std::time::Duration::from_millis(50);
        let _ = server_dtls.handle_timeout(init_now);

        let mut out_buf = [0u8; OUTPUT_BUF_SIZE];
        let mut server_connected = false;

        // Drive handshake: collect packets from one side, feed to the other
        for _ in 0..30 {
            if client.is_connected() && server_connected {
                break;
            }

            // Collect all client output, feed to server
            let mut client_to_server = Vec::new();
            let step = client.handshake_step(None).unwrap();
            if let HandshakeStep::DataToSend(data) = step {
                client_to_server = data;
            }
            if !client_to_server.is_empty() {
                server_dtls.handle_packet(&client_to_server).unwrap();
            }

            // Collect all server output
            let mut server_to_client: Vec<Vec<u8>> = Vec::new();
            loop {
                match server_dtls.poll_output(&mut out_buf) {
                    Output::Packet(p) => server_to_client.push(p.to_vec()),
                    Output::Connected => server_connected = true,
                    Output::PeerCert(_) => {}
                    _ => break,
                }
            }

            // Feed server output to client
            for pkt in &server_to_client {
                let step = client.handshake_step(Some(pkt)).unwrap();
                // If client generates response, feed it back to server
                if let HandshakeStep::DataToSend(data) = step {
                    server_dtls.handle_packet(&data).unwrap();
                    // Drain server output from this new input
                    loop {
                        match server_dtls.poll_output(&mut out_buf) {
                            Output::Packet(p) => {
                                let s2 = client.handshake_step(Some(p)).unwrap();
                                if let HandshakeStep::DataToSend(d) = s2 {
                                    server_dtls.handle_packet(&d).unwrap();
                                }
                            }
                            Output::Connected => server_connected = true,
                            _ => break,
                        }
                    }
                }
            }

            // Advance server time for retransmission
            let now = start + std::time::Duration::from_millis(100);
            let _ = server_dtls.handle_timeout(now);
        }

        assert!(client.is_connected(), "client should be connected");
        assert!(server_connected, "server should be connected");

        // Test encrypt → send to server → server decrypts
        let plaintext = b"Hello STUN server!";
        let encrypted = client.encrypt(plaintext).unwrap();
        assert_ne!(&encrypted[..], plaintext, "ciphertext should differ from plaintext");

        server_dtls.handle_packet(&encrypted).unwrap();
        match server_dtls.poll_output(&mut out_buf) {
            Output::ApplicationData(p) => {
                assert_eq!(p, plaintext, "decrypted data should match original");
            }
            other => panic!("Expected ApplicationData, got {:?}", other),
        }

        // Test server encrypt → client decrypt
        server_dtls.send_application_data(b"STUN response").unwrap();
        let server_encrypted = match server_dtls.poll_output(&mut out_buf) {
            Output::Packet(p) => p.to_vec(),
            other => panic!("Expected Packet, got {:?}", other),
        };

        let decrypted = client.decrypt(&server_encrypted).unwrap();
        assert_eq!(decrypted, b"STUN response");
    }
}
