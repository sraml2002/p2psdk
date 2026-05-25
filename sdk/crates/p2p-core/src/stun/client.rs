//! STUN/TURN client functions using DTLS-encrypted transport.
//!
//! Provides `get_external_address()` and `get_turn_relay_address()` that
//! perform a full DTLS handshake followed by an encrypted STUN/TURN exchange
//! with the server. I/O is injected via closures so the protocol logic stays
//! platform-independent.

use std::time::Instant;

use crate::dtls::session::{DtlsError, DtlsSession, HandshakeStep};
use crate::stun::codec::{StunResult, TurnResult};
use crate::stun::message;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum StunClientError {
    Dtls(DtlsError),
    Timeout,
    ResponseParse(String),
    StunError(String),
}

impl std::fmt::Display for StunClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dtls(e) => write!(f, "DTLS: {e}"),
            Self::Timeout => write!(f, "timeout"),
            Self::ResponseParse(msg) => write!(f, "parse: {msg}"),
            Self::StunError(msg) => write!(f, "STUN error: {msg}"),
        }
    }
}

impl From<DtlsError> for StunClientError {
    fn from(e: DtlsError) -> Self { Self::Dtls(e) }
}

// ---------------------------------------------------------------------------
// Timeouts (matching StunClient.ets)
// ---------------------------------------------------------------------------

const DTLS_HANDSHAKE_TIMEOUT_MS: u64 = 15_000;
const HANDSHAKE_STEP_TIMEOUT_MS: u64 = 1_000;
pub(crate) const STUN_RESPONSE_TIMEOUT_MS: u64 = 5_000;

// ---------------------------------------------------------------------------
// Core: DTLS handshake + encrypted exchange
// ---------------------------------------------------------------------------

/// Perform DTLS handshake then exchange one encrypted STUN/TURN message.
///
/// - `send`: sends raw bytes to the STUN/TURN server
/// - `recv`: receives with timeout (ms); returns `None` on timeout
/// - `request`: plaintext STUN/TURN message to send
///
/// Returns the decrypted response bytes.
pub fn dtls_stun_exchange(
    dtls: &mut DtlsSession,
    send: &mut dyn FnMut(&[u8]),
    recv: &mut dyn FnMut(u64) -> Option<Vec<u8>>,
    request: &[u8],
) -> Result<Vec<u8>, StunClientError> {
    // Phase 1: DTLS handshake
    dtls_handshake(dtls, send, recv)?;

    // Phase 2: Encrypt request and send
    let encrypted = dtls.encrypt(request)?;
    send(&encrypted);

    // Phase 3: Receive and decrypt response
    let response = recv(STUN_RESPONSE_TIMEOUT_MS).ok_or(StunClientError::Timeout)?;
    let decrypted = dtls.decrypt(&response)?;

    Ok(decrypted)
}

/// Drive a DTLS handshake to completion using the given I/O closures.
pub fn dtls_handshake(
    dtls: &mut DtlsSession,
    send: &mut dyn FnMut(&[u8]),
    recv: &mut dyn FnMut(u64) -> Option<Vec<u8>>,
) -> Result<(), StunClientError> {
    let start = Instant::now();
    let mut incoming: Option<Vec<u8>> = None;

    loop {
        if start.elapsed().as_millis() as u64 > DTLS_HANDSHAKE_TIMEOUT_MS {
            return Err(StunClientError::Timeout);
        }

        let step = dtls.handshake_step(incoming.as_deref())?;
        incoming = None;

        match step {
            HandshakeStep::Complete => return Ok(()),
            HandshakeStep::DataToSend(data) => {
                send(&data);
                if let Some(resp) = recv(HANDSHAKE_STEP_TIMEOUT_MS) {
                    incoming = Some(resp);
                }
            }
            HandshakeStep::Waiting => {
                if let Some(resp) = recv(HANDSHAKE_STEP_TIMEOUT_MS) {
                    incoming = Some(resp);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// High-level client functions
// ---------------------------------------------------------------------------

/// Discover the external (mapped) IP:port via DTLS-encrypted STUN Binding.
///
/// Creates a DTLS session, performs handshake, sends a STUN Binding Request
/// with the P2P_TOKEN attribute, and parses the XOR-MAPPED-ADDRESS from
/// the response.
pub fn get_external_address(
    send: &mut dyn FnMut(&[u8]),
    recv: &mut dyn FnMut(u64) -> Option<Vec<u8>>,
    _stun_ip: &str,
    _stun_port: u16,
    p2p_token: &str,
) -> Result<StunResult, StunClientError> {
    let mut dtls = DtlsSession::new()?;

    // Build STUN Binding Request
    let req = message::build_binding_request(p2p_token);
    let response = dtls_stun_exchange(&mut dtls, send, recv, &req.data)?;

    // Parse response
    message::parse_binding_response(&response, &req.transaction_id).map_err(Into::into)
}

/// Obtain a TURN relay address via DTLS-encrypted TURN Allocate.
///
/// Returns both the relay address and the mapped address from the
/// Allocate success response.
pub fn get_turn_relay_address(
    send: &mut dyn FnMut(&[u8]),
    recv: &mut dyn FnMut(u64) -> Option<Vec<u8>>,
    _turn_ip: &str,
    _turn_port: u16,
    p2p_token: &str,
    family: u8,
) -> Result<TurnResult, StunClientError> {
    let mut dtls = DtlsSession::new()?;

    let req = message::build_allocate_request(p2p_token, family);
    let response = dtls_stun_exchange(&mut dtls, send, recv, &req.data)?;

    message::parse_allocate_response(&response, &req.transaction_id).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let e = StunClientError::Timeout;
        assert_eq!(format!("{e}"), "timeout");

        let e = StunClientError::StunError("401 Unauthorized".into());
        assert!(format!("{e}").contains("401"));
    }

    #[test]
    fn test_dtls_handshake_starts() {
        let mut client_session = DtlsSession::new().unwrap();
        let step = client_session.handshake_step(None).unwrap();
        assert!(matches!(step, HandshakeStep::DataToSend(_)));
    }
}

use crate::stun::message::StunError;

impl From<StunError> for StunClientError {
    fn from(e: StunError) -> Self { Self::ResponseParse(e.to_string()) }
}
