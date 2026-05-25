//! ICE STUN Binding Request/Response encoding/decoding (plain UDP).
//!
//! Implements RFC 8445 / RFC 5389 ICE connectivity check STUN messages.

use crate::crypto::{hmac_sha1, stun_fingerprint};
use crate::stun::codec::{generate_transaction_id, pad_to_4, parse_xor_address};
use crate::types::{
    ATTR_FINGERPRINT, ATTR_ICE_CONTROLLED, ATTR_ICE_CONTROLLING, ATTR_MESSAGE_INTEGRITY,
    ATTR_PRIORITY, ATTR_USE_CANDIDATE, ATTR_USERNAME, ATTR_ERROR_CODE, ATTR_XOR_MAPPED_ADDRESS,
    STUN_BINDING_ERROR, STUN_BINDING_REQUEST, STUN_BINDING_SUCCESS, STUN_MAGIC_COOKIE,
};

// ── Data types ──────────────────────────────────────────────────────────────

/// Result of a STUN connectivity check.
#[derive(Debug, Clone)]
pub struct StunCheckResult {
    pub success: bool,
    pub mapped_ip: String,
    pub mapped_port: u16,
    pub role_conflict: bool,
    pub use_candidate: bool,
    pub error_code: u16,
    pub error_reason: String,
}

impl Default for StunCheckResult {
    fn default() -> Self {
        Self {
            success: false,
            mapped_ip: String::new(),
            mapped_port: 0,
            role_conflict: false,
            use_candidate: false,
            error_code: 0,
            error_reason: String::new(),
        }
    }
}

/// Result of building an ICE Binding Request.
#[derive(Debug, Clone)]
pub struct IceBindingRequestResult {
    pub data: Vec<u8>,
    pub transaction_id: [u8; 12],
}

/// Parsed ICE STUN message with all relevant attributes extracted.
#[derive(Debug, Clone)]
pub struct ParsedStunMessage {
    pub is_stun: bool,
    pub is_request: bool,
    pub is_response: bool,
    pub is_error_response: bool,
    pub transaction_id: Vec<u8>,
    pub username: String,
    pub priority: u32,
    /// `Some(true)` = controlling, `Some(false)` = controlled, `None` = not present
    pub controlling: Option<bool>,
    pub use_candidate: bool,
    pub error_code: u16,
    pub mapped_ip: String,
    pub mapped_port: u16,
}

impl Default for ParsedStunMessage {
    fn default() -> Self {
        Self {
            is_stun: false,
            is_request: false,
            is_response: false,
            is_error_response: false,
            transaction_id: Vec::new(),
            username: String::new(),
            priority: 0,
            controlling: None,
            use_candidate: false,
            error_code: 0,
            mapped_ip: String::new(),
            mapped_port: 0,
        }
    }
}

// ── Constants ───────────────────────────────────────────────────────────────

/// STUN header size: type(2) + length(2) + magic_cookie(4) + transaction_id(12)
const STUN_HEADER_LEN: usize = 20;

// ── Functions ───────────────────────────────────────────────────────────────

/// Build an ICE STUN Binding Request (type 0x0001).
///
/// Attribute order: USERNAME -> PRIORITY -> ICE-CONTROLLING/CONTROLLED ->
/// USE-CANDIDATE -> MESSAGE-INTEGRITY -> FINGERPRINT.
///
/// When `remote_pwd` is provided, MESSAGE-INTEGRITY (HMAC-SHA1) is computed
/// and included before FINGERPRINT.
pub fn build_ice_binding_request(
    username: &str,
    priority: u32,
    controlling: bool,
    tie_breaker: u64,
    use_candidate: bool,
    remote_pwd: Option<&str>,
) -> IceBindingRequestResult {
    let user_bytes = username.as_bytes();

    // Calculate attribute sizes (4-byte header + padded value)
    let username_attr_len = pad_to_4(4 + user_bytes.len());
    let priority_attr_len = pad_to_4(4 + 4);
    let role_attr_len = pad_to_4(4 + 8);
    let use_candidate_attr_len = pad_to_4(4 + 0);
    let mi_attr_len = if remote_pwd.is_some() {
        pad_to_4(4 + 20) // MESSAGE-INTEGRITY: 4B header + 20B HMAC
    } else {
        0
    };
    let fingerprint_attr_len = pad_to_4(4 + 4);

    let mut attrs_len = username_attr_len + priority_attr_len + role_attr_len + mi_attr_len + fingerprint_attr_len;
    if use_candidate {
        attrs_len += use_candidate_attr_len;
    }

    let total_len = STUN_HEADER_LEN + attrs_len;
    let mut buf = vec![0u8; total_len];

    // STUN header
    write_u16(&mut buf, 0, STUN_BINDING_REQUEST);
    write_u16(&mut buf, 2, attrs_len as u16);
    write_u32(&mut buf, 4, STUN_MAGIC_COOKIE);

    let transaction_id = generate_transaction_id();
    buf[8..20].copy_from_slice(&transaction_id);

    let mut offset = STUN_HEADER_LEN;

    // USERNAME
    write_u16(&mut buf, offset, ATTR_USERNAME);
    write_u16(&mut buf, offset + 2, user_bytes.len() as u16);
    buf[offset + 4..offset + 4 + user_bytes.len()].copy_from_slice(user_bytes);
    offset += username_attr_len;

    // PRIORITY
    write_u16(&mut buf, offset, ATTR_PRIORITY);
    write_u16(&mut buf, offset + 2, 4);
    write_u32(&mut buf, offset + 4, priority);
    offset += priority_attr_len;

    // ICE-CONTROLLING / ICE-CONTROLLED
    let role_attr = if controlling {
        ATTR_ICE_CONTROLLING
    } else {
        ATTR_ICE_CONTROLLED
    };
    write_u16(&mut buf, offset, role_attr);
    write_u16(&mut buf, offset + 2, 8);
    let tb_high = (tie_breaker >> 32) as u32;
    let tb_low = (tie_breaker & 0xFFFFFFFF) as u32;
    write_u32(&mut buf, offset + 4, tb_high);
    write_u32(&mut buf, offset + 8, tb_low);
    offset += role_attr_len;

    // USE-CANDIDATE
    if use_candidate {
        write_u16(&mut buf, offset, ATTR_USE_CANDIDATE);
        write_u16(&mut buf, offset + 2, 0);
        offset += use_candidate_attr_len;
    }

    // MESSAGE-INTEGRITY (HMAC-SHA1)
    if let Some(pwd) = remote_pwd {
        let mi_start = offset;

        // Write MI attribute header with placeholder HMAC
        write_u16(&mut buf, offset, ATTR_MESSAGE_INTEGRITY);
        write_u16(&mut buf, offset + 2, 20);
        // offset + 4 .. offset + 24 already zeroed

        // RFC 5389 Section 15.4:
        //   Length field MUST include MI attribute for HMAC computation
        let adjusted_length = ((mi_start - STUN_HEADER_LEN) + mi_attr_len) as u16;
        write_u16(&mut buf, 2, adjusted_length);

        // Compute HMAC-SHA1 over bytes[0..mi_start]
        let hmac = hmac_sha1(pwd.as_bytes(), &buf[..mi_start]);
        buf[mi_start + 4..mi_start + 4 + 20].copy_from_slice(&hmac);

        // Restore STUN header length to full attrs_len
        write_u16(&mut buf, 2, attrs_len as u16);

        offset += mi_attr_len;
    }

    // FINGERPRINT
    write_u16(&mut buf, offset, ATTR_FINGERPRINT);
    write_u16(&mut buf, offset + 2, 4);
    let fp = stun_fingerprint(&buf[..total_len]);
    write_u32(&mut buf, offset + 4, fp);

    IceBindingRequestResult {
        data: buf,
        transaction_id,
    }
}

/// Build an ICE STUN Binding Success Response (type 0x0101, no attributes).
///
/// Copies the transaction ID from the request.
/// Returns `None` if the request is too short or not a valid STUN message.
pub fn build_ice_binding_response(request_data: &[u8]) -> Option<Vec<u8>> {
    if request_data.len() < STUN_HEADER_LEN {
        return None;
    }
    // Check first 2 bits are 0 (STUN)
    if request_data[0] & 0xC0 != 0 {
        return None;
    }
    let req_type = read_u16(request_data, 0);
    // Accept Binding Request (0x0001) — mask out class bits for robustness
    if (req_type & 0x3EFF) != STUN_BINDING_REQUEST {
        return None;
    }

    let transaction_id = &request_data[8..20];
    let mut resp = vec![0u8; STUN_HEADER_LEN];

    write_u16(&mut resp, 0, STUN_BINDING_SUCCESS);
    write_u16(&mut resp, 2, 0); // no attributes
    write_u32(&mut resp, 4, STUN_MAGIC_COOKIE);
    resp[8..20].copy_from_slice(transaction_id);

    Some(resp)
}

/// Build an ICE STUN Binding Error Response (type 0x0111) with ERROR-CODE + FINGERPRINT.
///
/// Returns `None` if the request is too short.
pub fn build_ice_error_response(
    request_data: &[u8],
    error_code: u16,
    reason: &str,
) -> Option<Vec<u8>> {
    if request_data.len() < STUN_HEADER_LEN {
        return None;
    }

    let transaction_id = &request_data[8..20];
    let reason_bytes = reason.as_bytes();

    let error_attr_len = pad_to_4(4 + 4 + reason_bytes.len()); // 4B header + 4B error-code header + reason
    let fingerprint_attr_len = pad_to_4(4 + 4);
    let attrs_len = error_attr_len + fingerprint_attr_len;
    let total_len = STUN_HEADER_LEN + attrs_len;

    let mut buf = vec![0u8; total_len];

    // STUN header
    write_u16(&mut buf, 0, STUN_BINDING_ERROR);
    write_u16(&mut buf, 2, attrs_len as u16);
    write_u32(&mut buf, 4, STUN_MAGIC_COOKIE);
    buf[8..20].copy_from_slice(transaction_id);

    let mut offset = STUN_HEADER_LEN;

    // ERROR-CODE attribute
    write_u16(&mut buf, offset, ATTR_ERROR_CODE);
    write_u16(&mut buf, offset + 2, (4 + reason_bytes.len()) as u16);
    buf[offset + 4] = 0; // reserved
    buf[offset + 5] = (error_code / 100) as u8;
    buf[offset + 6] = (error_code % 100) as u8;
    buf[offset + 7..offset + 7 + reason_bytes.len()].copy_from_slice(reason_bytes);
    offset += error_attr_len;

    // FINGERPRINT
    write_u16(&mut buf, offset, ATTR_FINGERPRINT);
    write_u16(&mut buf, offset + 2, 4);
    let fp = stun_fingerprint(&buf[..total_len]);
    write_u32(&mut buf, offset + 4, fp);

    Some(buf)
}

/// Parse an incoming ICE STUN message.
///
/// Extracts all ICE-relevant attributes: USERNAME, PRIORITY, USE-CANDIDATE,
/// ICE-CONTROLLING/CONTROLLED, XOR-MAPPED-ADDRESS, ERROR-CODE.
pub fn parse_ice_stun_message(data: &[u8]) -> ParsedStunMessage {
    let mut result = ParsedStunMessage::default();

    if data.len() < STUN_HEADER_LEN {
        return result;
    }

    // Check first 2 bits are 0 (STUN)
    if data[0] & 0xC0 != 0 {
        return result;
    }

    let msg_type = read_u16(data, 0);
    let magic_cookie = read_u32(data, 4);

    if magic_cookie != STUN_MAGIC_COOKIE {
        return result;
    }

    result.is_stun = true;
    result.transaction_id = data[8..20].to_vec();

    match msg_type {
        STUN_BINDING_REQUEST => {
            result.is_request = true;
        }
        STUN_BINDING_SUCCESS => {
            result.is_response = true;
        }
        STUN_BINDING_ERROR => {
            result.is_error_response = true;
        }
        _ => {
            return result;
        }
    }

    let msg_length = read_u16(data, 2) as usize;
    let end = std::cmp::min(STUN_HEADER_LEN + msg_length, data.len());
    let mut offset = STUN_HEADER_LEN;

    while offset + 4 <= end {
        let attr_type = read_u16(data, offset);
        let attr_len = read_u16(data, offset + 2) as usize;

        match attr_type {
            ATTR_USERNAME if attr_len > 0 => {
                let val_start = offset + 4;
                if val_start + attr_len <= data.len() {
                    result.username = String::from_utf8_lossy(&data[val_start..val_start + attr_len]).into_owned();
                }
            }
            ATTR_PRIORITY if attr_len >= 4 => {
                result.priority = read_u32(data, offset + 4);
            }
            ATTR_USE_CANDIDATE => {
                result.use_candidate = true;
            }
            ATTR_ICE_CONTROLLING if attr_len >= 8 => {
                result.controlling = Some(true);
            }
            ATTR_ICE_CONTROLLED if attr_len >= 8 => {
                result.controlling = Some(false);
            }
            ATTR_XOR_MAPPED_ADDRESS => {
                if let Ok((ip, port)) = parse_xor_address(data, offset + 4, attr_len, &result.transaction_id) {
                    result.mapped_ip = ip;
                    result.mapped_port = port;
                }
            }
            ATTR_ERROR_CODE if attr_len >= 4 => {
                // ERROR-CODE value layout (RFC 5389 §15.6):
                //   byte 0: reserved (all zero)
                //   byte 1: high 5 bits reserved, low 3 bits = class
                //   byte 2: number (0-99)
                //   byte 3+: reason phrase
                let class_num = (data[offset + 5] & 0x07) as u16;
                let number = data[offset + 6] as u16;
                result.error_code = class_num * 100 + number;
            }
            _ => {}
        }

        offset += 4 + pad_to_4(attr_len);
    }

    result
}

// ── Helpers ─────────────────────────────────────────────────────────────────

#[inline]
fn write_u16(buf: &mut [u8], offset: usize, val: u16) {
    buf[offset..offset + 2].copy_from_slice(&val.to_be_bytes());
}

#[inline]
fn write_u32(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_be_bytes());
}

#[inline]
fn read_u16(buf: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([buf[offset], buf[offset + 1]])
}

#[inline]
fn read_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ATTR_MESSAGE_INTEGRITY;

    #[test]
    fn test_build_ice_binding_request() {
        let result = build_ice_binding_request(
            "user1:user2",
            12345,
            true,           // controlling
            0x1122334455667788,
            false,          // no use_candidate
            None,           // no remote_pwd
        );

        // STUN header checks
        assert_eq!(read_u16(&result.data, 0), STUN_BINDING_REQUEST);
        assert_eq!(read_u32(&result.data, 4), STUN_MAGIC_COOKIE);
        assert_eq!(&result.data[8..20], &result.transaction_id);

        // Verify attributes are present
        let msg_len = read_u16(&result.data, 2) as usize;
        let end = STUN_HEADER_LEN + msg_len;
        let mut found_username = false;
        let mut found_priority = false;
        let mut found_controlling = false;
        let mut found_fingerprint = false;
        let mut found_mi = false;

        let mut offset = STUN_HEADER_LEN;
        while offset + 4 <= end {
            let attr_type = read_u16(&result.data, offset);
            let attr_len = read_u16(&result.data, offset + 2) as usize;

            match attr_type {
                ATTR_USERNAME => found_username = true,
                ATTR_PRIORITY => {
                    found_priority = true;
                    assert_eq!(read_u32(&result.data, offset + 4), 12345);
                }
                ATTR_ICE_CONTROLLING => found_controlling = true,
                ATTR_FINGERPRINT => found_fingerprint = true,
                ATTR_MESSAGE_INTEGRITY => found_mi = true,
                _ => {}
            }

            offset += 4 + pad_to_4(attr_len);
        }

        assert!(found_username, "USERNAME attribute not found");
        assert!(found_priority, "PRIORITY attribute not found");
        assert!(found_controlling, "ICE-CONTROLLING attribute not found");
        assert!(found_fingerprint, "FINGERPRINT attribute not found");
        assert!(!found_mi, "MESSAGE-INTEGRITY should not be present without remote_pwd");

        // Verify fingerprint: clear the FP value, recompute, and compare
        let fp_offset = end - pad_to_4(4 + 4);
        let fp_val = read_u32(&result.data, fp_offset + 4);
        let mut verify_buf = result.data[..end].to_vec();
        write_u32(&mut verify_buf, fp_offset + 4, 0); // clear fingerprint value
        let expected_fp = stun_fingerprint(&verify_buf);
        assert_eq!(fp_val, expected_fp);
    }

    #[test]
    fn test_build_ice_binding_request_with_integrity() {
        let remote_pwd = "test_password";
        let result = build_ice_binding_request(
            "alice:bob",
            50000,
            false,  // controlled
            0xAABBCCDDEEFF0011,
            true,   // use_candidate
            Some(remote_pwd),
        );

        // Parse and verify MESSAGE-INTEGRITY is present
        let msg_len = read_u16(&result.data, 2) as usize;
        let end = STUN_HEADER_LEN + msg_len;
        let mut found_mi = false;
        let mut mi_offset = 0;

        let mut offset = STUN_HEADER_LEN;
        while offset + 4 <= end {
            let attr_type = read_u16(&result.data, offset);
            let attr_len = read_u16(&result.data, offset + 2) as usize;

            if attr_type == ATTR_MESSAGE_INTEGRITY {
                found_mi = true;
                mi_offset = offset;
            }
            offset += 4 + pad_to_4(attr_len);
        }

        assert!(found_mi, "MESSAGE-INTEGRITY should be present");

        // Re-derive the HMAC to verify correctness
        // The HMAC is computed over the message with adjusted length = attrs up to and including MI
        let mi_attr_len = pad_to_4(4 + 20);
        let adjusted_length = ((mi_offset - STUN_HEADER_LEN) + mi_attr_len) as u16;

        // Build the temp buffer with adjusted length for HMAC verification
        let mut temp = result.data.clone();
        write_u16(&mut temp, 2, adjusted_length);
        // Zero out the HMAC value
        temp[mi_offset + 4..mi_offset + 4 + 20].copy_from_slice(&[0u8; 20]);

        // The actual HMAC should be over bytes[0..mi_offset] with adjusted header length
        let mut verify_buf = result.data[..mi_offset].to_vec();
        // Temporarily set the length to adjusted_length
        write_u16(&mut verify_buf, 2, adjusted_length);

        let expected_hmac = hmac_sha1(remote_pwd.as_bytes(), &verify_buf);
        let actual_hmac = &result.data[mi_offset + 4..mi_offset + 4 + 20];
        assert_eq!(&expected_hmac[..], actual_hmac, "HMAC-SHA1 mismatch");
    }

    #[test]
    fn test_build_and_parse_roundtrip() {
        let username = "local_ufrag:remote_ufrag";
        let priority = 7890;
        let tie_breaker = 0xDEADBEEFCAFEBABE;

        let result = build_ice_binding_request(
            username,
            priority,
            true,       // controlling
            tie_breaker,
            true,       // use_candidate
            None,
        );

        let parsed = parse_ice_stun_message(&result.data);

        assert!(parsed.is_stun);
        assert!(parsed.is_request);
        assert!(!parsed.is_response);
        assert!(!parsed.is_error_response);
        assert_eq!(parsed.transaction_id, result.transaction_id);
        assert_eq!(parsed.username, username);
        assert_eq!(parsed.priority, priority);
        assert_eq!(parsed.controlling, Some(true));
        assert!(parsed.use_candidate);
    }

    #[test]
    fn test_build_error_response() {
        // Build a dummy request to extract transaction_id
        let req = build_ice_binding_request("a:b", 1, true, 0, false, None);

        let error_resp = build_ice_error_response(&req.data, 487, "Role Conflict")
            .expect("should build error response");

        // Verify header
        assert_eq!(read_u16(&error_resp, 0), STUN_BINDING_ERROR);
        assert_eq!(read_u32(&error_resp, 4), STUN_MAGIC_COOKIE);
        assert_eq!(&error_resp[8..20], &req.transaction_id);

        // Parse and verify
        let parsed = parse_ice_stun_message(&error_resp);
        assert!(parsed.is_stun);
        assert!(parsed.is_error_response);
        assert_eq!(parsed.error_code, 487);
        assert_eq!(parsed.transaction_id, req.transaction_id);

        // Verify fingerprint: clear the FP value, recompute, and compare
        let msg_len = read_u16(&error_resp, 2) as usize;
        let total = STUN_HEADER_LEN + msg_len;
        let fp_offset = total - pad_to_4(4 + 4);
        let fp_val = read_u32(&error_resp, fp_offset + 4);
        let mut verify_buf = error_resp[..total].to_vec();
        write_u32(&mut verify_buf, fp_offset + 4, 0); // clear fingerprint value
        assert_eq!(fp_val, stun_fingerprint(&verify_buf));
    }

    #[test]
    fn test_parse_response() {
        // Build a request to get a valid transaction_id
        let req = build_ice_binding_request("x:y", 100, false, 0, false, None);

        // Manually construct a success response with XOR-MAPPED-ADDRESS
        let cookie_bytes = STUN_MAGIC_COOKIE.to_be_bytes();
        // XOR-MAPPED-ADDRESS attribute: 4B header + 8B value = 12B total
        let xor_mapped_attr_total: usize = 4 + 8;
        let fp_attr_total = pad_to_4(4 + 4);
        let attrs_len = xor_mapped_attr_total + fp_attr_total;
        let total_len = STUN_HEADER_LEN + attrs_len;

        let mut resp = vec![0u8; total_len];

        // Header
        write_u16(&mut resp, 0, STUN_BINDING_SUCCESS);
        write_u16(&mut resp, 2, attrs_len as u16);
        write_u32(&mut resp, 4, STUN_MAGIC_COOKIE);
        resp[8..20].copy_from_slice(&req.transaction_id);

        // XOR-MAPPED-ADDRESS attribute
        let mut offset = STUN_HEADER_LEN;
        write_u16(&mut resp, offset, ATTR_XOR_MAPPED_ADDRESS);
        write_u16(&mut resp, offset + 2, 8); // value length
        // Value: reserved(1) + family(1) + x_port(2) + x_ip(4)
        resp[offset + 4] = 0x00; // reserved
        resp[offset + 5] = 0x01; // IPv4

        // Port 5000 XOR with high 16 bits of magic cookie
        let x_port = 5000u16 ^ u16::from_be_bytes([cookie_bytes[0], cookie_bytes[1]]);
        resp[offset + 6..offset + 8].copy_from_slice(&x_port.to_be_bytes());

        // IP 10.0.0.1 XOR with magic cookie bytes
        let ip: [u8; 4] = [10, 0, 0, 1];
        for i in 0..4 {
            resp[offset + 8 + i] = ip[i] ^ cookie_bytes[i];
        }
        offset += xor_mapped_attr_total;

        // FINGERPRINT
        write_u16(&mut resp, offset, ATTR_FINGERPRINT);
        write_u16(&mut resp, offset + 2, 4);
        let fp = stun_fingerprint(&resp[..total_len]);
        write_u32(&mut resp, offset + 4, fp);

        // Parse
        let parsed = parse_ice_stun_message(&resp);

        assert!(parsed.is_stun);
        assert!(parsed.is_response);
        assert!(!parsed.is_request);
        assert!(!parsed.is_error_response);
        assert_eq!(parsed.transaction_id, req.transaction_id);
        assert_eq!(parsed.mapped_ip, "10.0.0.1");
        assert_eq!(parsed.mapped_port, 5000);
    }

    #[test]
    fn test_build_ice_binding_response() {
        let req = build_ice_binding_request("a:b", 1, true, 0, false, None);
        let resp = build_ice_binding_response(&req.data).expect("should build response");

        assert_eq!(read_u16(&resp, 0), STUN_BINDING_SUCCESS);
        assert_eq!(read_u16(&resp, 2), 0); // no attributes
        assert_eq!(read_u32(&resp, 4), STUN_MAGIC_COOKIE);
        assert_eq!(&resp[8..20], &req.transaction_id);
        assert_eq!(resp.len(), STUN_HEADER_LEN);
    }

    #[test]
    fn test_parse_too_short() {
        let parsed = parse_ice_stun_message(&[0x00, 0x01, 0x00]);
        assert!(!parsed.is_stun);
    }

    #[test]
    fn test_parse_non_stun() {
        // First 2 bits not zero
        let mut data = vec![0u8; 20];
        data[0] = 0x80; // bit 0 of first byte set
        let parsed = parse_ice_stun_message(&data);
        assert!(!parsed.is_stun);
    }

    #[test]
    fn test_controlled_role_roundtrip() {
        let result = build_ice_binding_request(
            "u1:u2",
            999,
            false, // controlled
            0x123456789ABCDEF0,
            false,
            None,
        );

        let parsed = parse_ice_stun_message(&result.data);
        assert_eq!(parsed.controlling, Some(false));
    }
}
