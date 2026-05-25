//! Persistent TURN session with DTLS transport.
//!
//! Manages a long-lived DTLS connection to a TURN server, keeping the
//! underlying UDP socket alive across multiple Allocate / CreatePermission
//! operations. I/O is injected via closures.

use crate::dtls::session::{DtlsError, DtlsSession};
use crate::stun::client::{dtls_handshake, dtls_stun_exchange, StunClientError, STUN_RESPONSE_TIMEOUT_MS};
use crate::stun::codec::generate_transaction_id;
use crate::stun::message;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum TurnError {
    Client(StunClientError),
    NotOpen,
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(e) => write!(f, "TURN client: {e}"),
            Self::NotOpen => write!(f, "session not open"),
        }
    }
}

impl From<StunClientError> for TurnError {
    fn from(e: StunClientError) -> Self { Self::Client(e) }
}

impl From<DtlsError> for TurnError {
    fn from(e: DtlsError) -> Self { Self::Client(StunClientError::Dtls(e)) }
}

// ---------------------------------------------------------------------------
// TurnSession
// ---------------------------------------------------------------------------

/// Persistent DTLS session to a TURN server.
///
/// Created via `TurnSession::allocate()`. The underlying DTLS connection
/// and UDP state are kept alive for subsequent `create_permission()` calls.
pub struct TurnSession {
    dtls: DtlsSession,
    turn_ip: String,
    turn_port: u16,
    p2p_token: String,
    relay_ip: String,
    relay_port: u16,
    mapped_ip: String,
    mapped_port: u16,
    closed: bool,
}

impl TurnSession {
    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn relay_ip(&self) -> &str { &self.relay_ip }
    pub fn relay_port(&self) -> u16 { self.relay_port }
    pub fn mapped_ip(&self) -> &str { &self.mapped_ip }
    pub fn mapped_port(&self) -> u16 { self.mapped_port }

    // ── Allocate ──────────────────────────────────────────────────────────────

    /// Allocate a TURN relay address.
    ///
    /// Creates a DTLS session, performs the handshake, sends an Allocate
    /// request, and stores the relay + mapped addresses.
    pub fn allocate(
        send: &mut dyn FnMut(&[u8]),
        recv: &mut dyn FnMut(u64) -> Option<Vec<u8>>,
        turn_ip: &str,
        turn_port: u16,
        p2p_token: &str,
        family: u8,
    ) -> Result<Self, TurnError> {
        let mut dtls = DtlsSession::new()?;

        // DTLS handshake
        dtls_handshake(&mut dtls, send, recv)?;

        // Build and send Allocate request
        let req = message::build_allocate_request(p2p_token, family);
        let encrypted = dtls.encrypt(&req.data)?;
        send(&encrypted);

        // Receive and parse response
        let response = recv(STUN_RESPONSE_TIMEOUT_MS)
            .ok_or_else(|| TurnError::Client(StunClientError::Timeout))?;
        let decrypted = dtls.decrypt(&response)?;
        let result = message::parse_allocate_response(&decrypted, &req.transaction_id)
            .map_err(|e| TurnError::Client(StunClientError::ResponseParse(e.to_string())))?;

        Ok(Self {
            dtls,
            turn_ip: turn_ip.into(),
            turn_port,
            p2p_token: p2p_token.into(),
            relay_ip: result.relay_ip,
            relay_port: result.relay_port,
            mapped_ip: result.mapped_ip,
            mapped_port: result.mapped_port,
            closed: false,
        })
    }

    // ── CreatePermission ──────────────────────────────────────────────────────

    /// Send a CreatePermission request for the given peer addresses.
    ///
    /// `peers` — array of `"ip:port"` strings.
    pub fn create_permission(
        &mut self,
        send: &mut dyn FnMut(&[u8]),
        recv: &mut dyn FnMut(u64) -> Option<Vec<u8>>,
        peers: &[&str],
    ) -> Result<(), TurnError> {
        if self.closed {
            return Err(TurnError::NotOpen);
        }
        if peers.is_empty() {
            return Ok(());
        }

        let tid = generate_transaction_id();
        let req = message::build_create_permission_request(peers, &self.p2p_token, &tid);
        let response = dtls_stun_exchange(&mut self.dtls, send, recv, &req.data)?;

        // Validate response type
        if response.len() < 2 {
            return Err(TurnError::Client(StunClientError::ResponseParse(
                "response too short".into(),
            )));
        }

        let msg_type = u16::from_be_bytes([response[0], response[1]]);
        let success_type = crate::types::STUN_CREATE_PERMISSION_SUCCESS;
        let error_type = crate::types::STUN_CREATE_PERMISSION_ERROR;

        if msg_type == success_type {
            return Ok(());
        }

        if msg_type == error_type {
            // Parse ERROR-CODE attribute
            let err = parse_error_code(&response);
            return Err(TurnError::Client(StunClientError::StunError(
                err.unwrap_or_else(|| format!("TURN CreatePermission error (type 0x{msg_type:04x})")),
            )));
        }

        Err(TurnError::Client(StunClientError::StunError(
            format!("unexpected response type 0x{msg_type:04x}"),
        )))
    }

    // ── Close ─────────────────────────────────────────────────────────────────

    pub fn close(&mut self) {
        if !self.closed {
            self.dtls.close();
            self.closed = true;
        }
    }
}

impl Drop for TurnSession {
    fn drop(&mut self) {
        self.close();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_error_code(response: &[u8]) -> Option<String> {
    if response.len() < 20 {
        return None;
    }
    let msg_len = u16::from_be_bytes([response[2], response[3]]) as usize;
    let mut offset = 20;
    while offset + 4 <= 20 + msg_len {
        let attr_type = u16::from_be_bytes([response[offset], response[offset + 1]]);
        let attr_len = u16::from_be_bytes([response[offset + 2], response[offset + 3]]) as usize;
        if attr_type == crate::types::ATTR_ERROR_CODE && offset + 8 <= response.len() {
            let class = (response[offset + 4] & 0x07) as u16;
            let number = u16::from_be_bytes([response[offset + 6], response[offset + 7]]);
            let code = class * 100 + number;
            // Reason phrase is the rest of the attribute value
            let reason_start = offset + 8;
            let reason_end = (reason_start + attr_len.saturating_sub(4)).min(response.len());
            let reason = String::from_utf8_lossy(&response[reason_start..reason_end]);
            return Some(format!("{code} {reason}"));
        }
        offset += 4 + ((attr_len + 3) & !3);
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let e = TurnError::NotOpen;
        assert_eq!(format!("{e}"), "session not open");
    }

    #[test]
    fn test_parse_error_code_too_short() {
        assert!(parse_error_code(&[0; 10]).is_none());
    }

    #[test]
    fn test_parse_error_code_no_error_attr() {
        // Valid STUN header with SOFTWARE attribute only
        let mut buf = vec![0u8; 28];
        buf[2] = 0; // msg_len = 8
        buf[3] = 8;
        // SOFTWARE attr (0x8022), not ERROR-CODE
        buf[20] = 0x80;
        buf[21] = 0x22;
        assert!(parse_error_code(&buf).is_none());
    }
}
