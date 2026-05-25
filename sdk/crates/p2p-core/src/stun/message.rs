//! STUN message building and parsing (Binding, Allocate, CreatePermission).

use super::codec::{
    generate_transaction_id, ipv6_to_bytes, pad_to_4, parse_xor_address, StunResult, TurnResult,
};
use crate::types::{
    AF_INET, AF_INET6, ATTR_ERROR_CODE, ATTR_P2P_TOKEN, ATTR_REQUESTED_ADDRESS_FAMILY,
    ATTR_REQUESTED_TRANSPORT, ATTR_SOFTWARE, ATTR_XOR_MAPPED_ADDRESS, ATTR_XOR_PEER_ADDRESS,
    ATTR_XOR_RELAYED_ADDRESS, STUN_ALLOCATE_ERROR, STUN_ALLOCATE_REQUEST, STUN_ALLOCATE_SUCCESS,
    STUN_BINDING_REQUEST, STUN_BINDING_SUCCESS, STUN_CREATE_PERMISSION_REQUEST,
    STUN_MAGIC_COOKIE, TURN_TRANSPORT_UDP,
};

/// A STUN request consisting of the serialized message bytes and the transaction ID.
#[derive(Debug, Clone)]
pub struct StunRequest {
    pub data: Vec<u8>,
    pub transaction_id: [u8; 12],
}

/// Errors that can occur during STUN message parsing.
#[derive(Debug, thiserror::Error)]
pub enum StunError {
    #[error("response too short: {0} bytes")]
    ResponseTooShort(usize),
    #[error("invalid STUN response: first two bits not 00")]
    InvalidFirstBits,
    #[error("unexpected message type: 0x{0:04x}")]
    UnexpectedMessageType(u16),
    #[error("invalid Magic Cookie: 0x{0:08x}")]
    InvalidMagicCookie(u32),
    #[error("transaction ID mismatch")]
    TransactionIdMismatch,
    #[error("XOR-MAPPED-ADDRESS attribute not found")]
    XorMappedAddressNotFound,
    #[error("XOR-RELAYED-ADDRESS not found in Allocate response")]
    XorRelayedAddressNotFound,
    #[error("allocate error {code}: {reason}")]
    AllocateError { code: u16, reason: String },
    #[error("allocate error (unknown error code)")]
    AllocateErrorUnknown,
    #[error("unsupported address family: {0}")]
    UnsupportedAddressFamily(u8),
    #[error("XOR-MAPPED-ADDRESS too short")]
    XorMappedAddressTooShort,
    #[error("invalid peer address format: {0}")]
    InvalidPeerAddress(String),
}

// ── Build helpers ────────────────────────────────────────────────────────────

/// Write a big-endian u16 at the given offset.
fn write_u16(buf: &mut [u8], offset: usize, val: u16) {
    buf[offset..offset + 2].copy_from_slice(&val.to_be_bytes());
}

/// Write a big-endian u32 at the given offset.
fn write_u32(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_be_bytes());
}

/// Write the STUN header into the first 20 bytes of `buf`.
/// `attrs_len` is the total length of all attributes (message length field).
fn write_stun_header(buf: &mut [u8], msg_type: u16, attrs_len: u16, transaction_id: &[u8; 12]) {
    write_u16(buf, 0, msg_type);
    write_u16(buf, 2, attrs_len);
    write_u32(buf, 4, STUN_MAGIC_COOKIE);
    buf[8..20].copy_from_slice(transaction_id);
}

/// Parse an "ip:port" string, returning (ip_str, port, is_ipv6).
/// Supports both IPv4 "1.2.3.4:5678" and IPv6 "2001:db8::1:5678" (last colon splits port).
fn parse_peer_addr(peer: &str) -> Result<(&str, u16, bool), StunError> {
    let last_colon = peer.rfind(':').ok_or_else(|| {
        StunError::InvalidPeerAddress(peer.to_string())
    })?;
    let ip_part = &peer[..last_colon];
    let port_part = &peer[last_colon + 1..];
    let port: u16 = port_part.parse().map_err(|_| {
        StunError::InvalidPeerAddress(peer.to_string())
    })?;
    let is_ipv6 = ip_part.contains(':');
    Ok((ip_part, port, is_ipv6))
}

// ── Public build functions ───────────────────────────────────────────────────

/// Build a STUN Binding Request with P2P-TOKEN and SOFTWARE attributes.
///
/// - Message type: 0x0001 (Binding Request)
/// - Attributes: P2P-TOKEN (0x0081), SOFTWARE (0x8022, "MyApp")
pub fn build_binding_request(p2p_token: &str) -> StunRequest {
    let token_bytes = p2p_token.as_bytes();
    let sw_bytes = b"MyApp";

    let p2p_attr_size = pad_to_4(4 + token_bytes.len());
    let sw_attr_size = pad_to_4(4 + sw_bytes.len());
    let attrs_len = (p2p_attr_size + sw_attr_size) as u16;
    let total_len = 20 + attrs_len as usize;

    let mut buf = vec![0u8; total_len];
    let transaction_id = generate_transaction_id();

    write_stun_header(&mut buf, STUN_BINDING_REQUEST, attrs_len, &transaction_id);

    // P2P-TOKEN (0x0081)
    let mut offset = 20;
    write_u16(&mut buf, offset, ATTR_P2P_TOKEN);
    write_u16(&mut buf, offset + 2, token_bytes.len() as u16);
    buf[offset + 4..offset + 4 + token_bytes.len()].copy_from_slice(token_bytes);

    // SOFTWARE (0x8022)
    offset += p2p_attr_size;
    write_u16(&mut buf, offset, ATTR_SOFTWARE);
    write_u16(&mut buf, offset + 2, sw_bytes.len() as u16);
    buf[offset + 4..offset + 4 + sw_bytes.len()].copy_from_slice(sw_bytes);

    StunRequest {
        data: buf,
        transaction_id,
    }
}

/// Build a STUN Allocate Request with REQUESTED-TRANSPORT, REQUESTED-ADDRESS-FAMILY,
/// P2P-TOKEN, and SOFTWARE attributes.
///
/// - Message type: 0x0003 (Allocate Request)
/// - `family`: address family (1 = IPv4, 2 = IPv6)
pub fn build_allocate_request(p2p_token: &str, family: u8) -> StunRequest {
    let token_bytes = p2p_token.as_bytes();
    let sw_bytes = b"MyApp";

    let rt_size = pad_to_4(4 + 4); // REQUESTED-TRANSPORT: 4 bytes value
    let raf_size = pad_to_4(4 + 4); // REQUESTED-ADDRESS-FAMILY: 4 bytes value
    let p2p_size = pad_to_4(4 + token_bytes.len());
    let sw_size = pad_to_4(4 + sw_bytes.len());
    let attrs_len = (rt_size + raf_size + p2p_size + sw_size) as u16;
    let total_len = 20 + attrs_len as usize;

    let mut buf = vec![0u8; total_len];
    let transaction_id = generate_transaction_id();

    write_stun_header(&mut buf, STUN_ALLOCATE_REQUEST, attrs_len, &transaction_id);

    // REQUESTED-TRANSPORT (0x0019): proto=17 (UDP) + 3 bytes RFFU
    let mut offset = 20;
    write_u16(&mut buf, offset, ATTR_REQUESTED_TRANSPORT);
    write_u16(&mut buf, offset + 2, 4);
    buf[offset + 4] = TURN_TRANSPORT_UDP;
    buf[offset + 5] = 0;
    buf[offset + 6] = 0;
    buf[offset + 7] = 0;

    // REQUESTED-ADDRESS-FAMILY (0x0017)
    offset += rt_size;
    write_u16(&mut buf, offset, ATTR_REQUESTED_ADDRESS_FAMILY);
    write_u16(&mut buf, offset + 2, 4);
    buf[offset + 4] = family;
    buf[offset + 5] = 0;
    buf[offset + 6] = 0;
    buf[offset + 7] = 0;

    // P2P-TOKEN (0x0081)
    offset += raf_size;
    write_u16(&mut buf, offset, ATTR_P2P_TOKEN);
    write_u16(&mut buf, offset + 2, token_bytes.len() as u16);
    buf[offset + 4..offset + 4 + token_bytes.len()].copy_from_slice(token_bytes);

    // SOFTWARE (0x8022)
    offset += p2p_size;
    write_u16(&mut buf, offset, ATTR_SOFTWARE);
    write_u16(&mut buf, offset + 2, sw_bytes.len() as u16);
    buf[offset + 4..offset + 4 + sw_bytes.len()].copy_from_slice(sw_bytes);

    StunRequest {
        data: buf,
        transaction_id,
    }
}

/// Build a STUN CreatePermission Request with P2P-TOKEN and XOR-PEER-ADDRESS attributes.
///
/// - Message type: 0x0008 (CreatePermission)
/// - `peers`: slice of "ip:port" strings (supports IPv4 and IPv6)
/// - `transaction_id`: caller-supplied transaction ID (for correlation)
pub fn build_create_permission_request(
    peers: &[&str],
    p2p_token: &str,
    transaction_id: &[u8; 12],
) -> StunRequest {
    let token_bytes = p2p_token.as_bytes();

    // Pre-calculate total attribute size
    let p2p_size = pad_to_4(4 + token_bytes.len());
    let mut peer_attrs_size = 0;
    for peer in peers {
        let (_, _, is_ipv6) = parse_peer_addr(peer).unwrap_or(("", 0, false));
        let addr_len = if is_ipv6 { 20 } else { 8 };
        peer_attrs_size += pad_to_4(4 + addr_len);
    }
    let attrs_len = (p2p_size + peer_attrs_size) as u16;
    let total_len = 20 + attrs_len as usize;

    let mut buf = vec![0u8; total_len];

    write_stun_header(&mut buf, STUN_CREATE_PERMISSION_REQUEST, attrs_len, transaction_id);

    // P2P-TOKEN (0x0081)
    let mut offset = 20;
    write_u16(&mut buf, offset, ATTR_P2P_TOKEN);
    write_u16(&mut buf, offset + 2, token_bytes.len() as u16);
    buf[offset + 4..offset + 4 + token_bytes.len()].copy_from_slice(token_bytes);
    offset += p2p_size;

    // XOR-PEER-ADDRESS attributes (one per peer)
    let cookie_bytes = STUN_MAGIC_COOKIE.to_be_bytes();
    for peer in peers {
        let (ip_part, port, is_ipv6) = parse_peer_addr(peer).unwrap();
        let family = if is_ipv6 { AF_INET6 } else { AF_INET };
        let addr_len: u16 = if is_ipv6 { 20 } else { 8 };
        let attr_total = pad_to_4(4 + addr_len as usize);

        write_u16(&mut buf, offset, ATTR_XOR_PEER_ADDRESS);
        write_u16(&mut buf, offset + 2, addr_len);
        buf[offset + 4] = 0; // reserved
        buf[offset + 5] = family;

        // XOR port with upper 16 bits of magic cookie
        let x_port = port ^ u16::from_be_bytes([cookie_bytes[0], cookie_bytes[1]]);
        write_u16(&mut buf, offset + 6, x_port);

        if is_ipv6 {
            let ip_bytes = ipv6_to_bytes(ip_part).unwrap_or([0u8; 16]);
            // XOR address with magic_cookie (first 4 bytes) + transaction_id (remaining 12 bytes)
            for i in 0..4 {
                buf[offset + 8 + i] = ip_bytes[i] ^ cookie_bytes[i];
            }
            for i in 4..16 {
                buf[offset + 8 + i] = ip_bytes[i] ^ transaction_id[i - 4];
            }
        } else {
            let octets: Vec<u8> = ip_part
                .split('.')
                .map(|s| s.parse::<u8>().unwrap_or(0))
                .collect();
            buf[offset + 8] = octets[0] ^ cookie_bytes[0];
            buf[offset + 9] = octets[1] ^ cookie_bytes[1];
            buf[offset + 10] = octets[2] ^ cookie_bytes[2];
            buf[offset + 11] = octets[3] ^ cookie_bytes[3];
        }

        offset += attr_total;
    }

    StunRequest {
        data: buf,
        transaction_id: *transaction_id,
    }
}

// ── Public parse functions ───────────────────────────────────────────────────

/// Parse a STUN Binding Response, extracting the XOR-MAPPED-ADDRESS.
///
/// Validates the STUN header, magic cookie, and transaction ID.
pub fn parse_binding_response(
    data: &[u8],
    transaction_id: &[u8; 12],
) -> Result<StunResult, StunError> {
    if data.len() < 20 {
        return Err(StunError::ResponseTooShort(data.len()));
    }

    // Validate first two bits are 00
    if (data[0] & 0xC0) != 0 {
        return Err(StunError::InvalidFirstBits);
    }

    // Validate message type = Binding Success Response (0x0101)
    let msg_type = u16::from_be_bytes([data[0], data[1]]);
    if msg_type != STUN_BINDING_SUCCESS {
        return Err(StunError::UnexpectedMessageType(msg_type));
    }

    // Validate Magic Cookie
    let cookie = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    if cookie != STUN_MAGIC_COOKIE {
        return Err(StunError::InvalidMagicCookie(cookie));
    }

    // Validate Transaction ID
    if data[8..20] != transaction_id[..] {
        return Err(StunError::TransactionIdMismatch);
    }

    // Parse attributes
    let msg_length = u16::from_be_bytes([data[2], data[3]]) as usize;
    let end = 20 + msg_length;
    let mut offset = 20;

    while offset + 4 <= end {
        let attr_type = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let attr_len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]);

        if attr_type == ATTR_XOR_MAPPED_ADDRESS {
            let family = data.get(offset + 5).copied().unwrap_or(0);
            if family == AF_INET || family == AF_INET6 {
                let (ip, port) = parse_xor_address(data, offset + 4, attr_len as usize, &data[8..20])
                    .map_err(|_| StunError::XorMappedAddressTooShort)?;
                return Ok(StunResult { ip, port });
            } else {
                return Err(StunError::UnsupportedAddressFamily(family));
            }
        }

        offset += 4 + pad_to_4(attr_len as usize);
    }

    Err(StunError::XorMappedAddressNotFound)
}

/// Parse a STUN Allocate Response, extracting XOR-RELAYED-ADDRESS and XOR-MAPPED-ADDRESS.
///
/// Supports both Allocate Success and Allocate Error responses.
pub fn parse_allocate_response(
    data: &[u8],
    transaction_id: &[u8; 12],
) -> Result<TurnResult, StunError> {
    if data.len() < 20 {
        return Err(StunError::ResponseTooShort(data.len()));
    }

    // Validate first two bits are 00
    if (data[0] & 0xC0) != 0 {
        return Err(StunError::InvalidFirstBits);
    }

    let msg_type = u16::from_be_bytes([data[0], data[1]]);

    // Handle Allocate Error (0x0113)
    if msg_type == STUN_ALLOCATE_ERROR {
        let msg_length = u16::from_be_bytes([data[2], data[3]]) as usize;
        let end = 20 + msg_length;
        let mut offset = 20;

        while offset + 4 <= end {
            let attr_type = u16::from_be_bytes([data[offset], data[offset + 1]]);
            let attr_len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]);

            if attr_type == ATTR_ERROR_CODE && attr_len >= 4 {
                // ERROR-CODE attribute value: 2 bytes reserved + class(1) + number(1) + reason
                let val_start = offset + 4;
                let class_num = data[val_start + 2] & 0x07;
                let number = (class_num as u16) * 100 + data[val_start + 3] as u16;
                let reason_start = val_start + 4;
                let reason_end = offset + 4 + attr_len as usize;
                let reason = if reason_end <= data.len() {
                    String::from_utf8_lossy(&data[reason_start..reason_end]).into_owned()
                } else {
                    String::new()
                };
                return Err(StunError::AllocateError { code: number, reason });
            }

            offset += 4 + pad_to_4(attr_len as usize);
        }

        return Err(StunError::AllocateErrorUnknown);
    }

    // Validate message type = Allocate Success (0x0103)
    if msg_type != STUN_ALLOCATE_SUCCESS {
        return Err(StunError::UnexpectedMessageType(msg_type));
    }

    // Validate Magic Cookie
    let cookie = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    if cookie != STUN_MAGIC_COOKIE {
        return Err(StunError::InvalidMagicCookie(cookie));
    }

    // Validate Transaction ID
    if data[8..20] != transaction_id[..] {
        return Err(StunError::TransactionIdMismatch);
    }

    let mut result = TurnResult::default();

    let msg_length = u16::from_be_bytes([data[2], data[3]]) as usize;
    let end = 20 + msg_length;
    let mut offset = 20;

    while offset + 4 <= end {
        let attr_type = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let attr_len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]);

        if attr_type == ATTR_XOR_RELAYED_ADDRESS {
            if let Ok((ip, port)) = parse_xor_address(data, offset + 4, attr_len as usize, &data[8..20]) {
                result.relay_ip = ip;
                result.relay_port = port;
            }
        } else if attr_type == ATTR_XOR_MAPPED_ADDRESS {
            if let Ok((ip, port)) = parse_xor_address(data, offset + 4, attr_len as usize, &data[8..20]) {
                result.mapped_ip = ip;
                result.mapped_port = port;
            }
        }

        offset += 4 + pad_to_4(attr_len as usize);
    }

    if result.relay_ip.is_empty() {
        return Err(StunError::XorRelayedAddressNotFound);
    }

    Ok(result)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: read a big-endian u16 from a byte slice.
    fn read_u16(buf: &[u8], offset: usize) -> u16 {
        u16::from_be_bytes([buf[offset], buf[offset + 1]])
    }

    /// Helper: find an attribute by type in a STUN message, returning (offset, attr_len).
    fn find_attr(buf: &[u8], attr_type: u16) -> Option<(usize, u16)> {
        let msg_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        let end = 20 + msg_len;
        let mut offset = 20;
        while offset + 4 <= end {
            let at = read_u16(buf, offset);
            let al = read_u16(buf, offset + 2);
            if at == attr_type {
                return Some((offset, al));
            }
            offset += 4 + pad_to_4(al as usize);
        }
        None
    }

    #[test]
    fn test_build_binding_request() {
        let req = build_binding_request("test-token");

        // Verify message type
        assert_eq!(read_u16(&req.data, 0), STUN_BINDING_REQUEST);

        // Verify magic cookie
        assert_eq!(
            u32::from_be_bytes([
                req.data[4],
                req.data[5],
                req.data[6],
                req.data[7]
            ]),
            STUN_MAGIC_COOKIE
        );

        // Verify transaction ID is stored in header and struct
        assert_eq!(&req.data[8..20], &req.transaction_id[..]);

        // Verify P2P-TOKEN attribute exists
        let (off, len) = find_attr(&req.data, ATTR_P2P_TOKEN).expect("P2P-TOKEN not found");
        assert_eq!(len as usize, "test-token".len());
        assert_eq!(
            &req.data[off + 4..off + 4 + len as usize],
            b"test-token"
        );

        // Verify SOFTWARE attribute exists
        let (off, len) = find_attr(&req.data, ATTR_SOFTWARE).expect("SOFTWARE not found");
        assert_eq!(len as usize, "MyApp".len());
        assert_eq!(&req.data[off + 4..off + 4 + len as usize], b"MyApp");
    }

    #[test]
    fn test_build_allocate_request() {
        let req = build_allocate_request("mytoken", AF_INET);

        // Verify message type
        assert_eq!(read_u16(&req.data, 0), STUN_ALLOCATE_REQUEST);

        // Verify magic cookie
        assert_eq!(
            u32::from_be_bytes([
                req.data[4],
                req.data[5],
                req.data[6],
                req.data[7]
            ]),
            STUN_MAGIC_COOKIE
        );

        // Verify REQUESTED-TRANSPORT attribute with UDP value (17)
        let (off, len) = find_attr(&req.data, ATTR_REQUESTED_TRANSPORT)
            .expect("REQUESTED-TRANSPORT not found");
        assert_eq!(len, 4);
        assert_eq!(req.data[off + 4], TURN_TRANSPORT_UDP); // 17 = UDP
        assert_eq!(req.data[off + 5], 0); // RFFU
        assert_eq!(req.data[off + 6], 0); // RFFU
        assert_eq!(req.data[off + 7], 0); // RFFU

        // Verify REQUESTED-ADDRESS-FAMILY attribute
        let (off, len) = find_attr(&req.data, ATTR_REQUESTED_ADDRESS_FAMILY)
            .expect("REQUESTED-ADDRESS-FAMILY not found");
        assert_eq!(len, 4);
        assert_eq!(req.data[off + 4], AF_INET);

        // Verify P2P-TOKEN attribute
        let (_, _) = find_attr(&req.data, ATTR_P2P_TOKEN).expect("P2P-TOKEN not found");

        // Verify SOFTWARE attribute
        let (_, _) = find_attr(&req.data, ATTR_SOFTWARE).expect("SOFTWARE not found");
    }

    #[test]
    fn test_build_create_permission_request_ipv4() {
        let tid = [0x42u8; 12];
        let req = build_create_permission_request(
            &["192.168.1.100:12345"],
            "token",
            &tid,
        );

        assert_eq!(read_u16(&req.data, 0), STUN_CREATE_PERMISSION_REQUEST);

        // Verify P2P-TOKEN
        let (_, _) = find_attr(&req.data, ATTR_P2P_TOKEN).expect("P2P-TOKEN not found");

        // Verify XOR-PEER-ADDRESS
        let (off, len) = find_attr(&req.data, ATTR_XOR_PEER_ADDRESS)
            .expect("XOR-PEER-ADDRESS not found");
        assert_eq!(len, 8); // IPv4: 8 bytes value
        assert_eq!(req.data[off + 4], 0); // reserved
        assert_eq!(req.data[off + 5], AF_INET); // family

        // Verify XOR'd port and address can be decoded back
        let cookie_bytes = STUN_MAGIC_COOKIE.to_be_bytes();
        let x_port = u16::from_be_bytes([req.data[off + 6], req.data[off + 7]]);
        let decoded_port = x_port ^ u16::from_be_bytes([cookie_bytes[0], cookie_bytes[1]]);
        assert_eq!(decoded_port, 12345);

        let decoded_ip = format!(
            "{}.{}.{}.{}",
            req.data[off + 8] ^ cookie_bytes[0],
            req.data[off + 9] ^ cookie_bytes[1],
            req.data[off + 10] ^ cookie_bytes[2],
            req.data[off + 11] ^ cookie_bytes[3]
        );
        assert_eq!(decoded_ip, "192.168.1.100");
    }

    #[test]
    fn test_build_create_permission_request_ipv6() {
        let tid = [0xABu8; 12];
        let req = build_create_permission_request(
            &["2001:db8::1:54321"],
            "tok",
            &tid,
        );

        // Verify XOR-PEER-ADDRESS is IPv6
        let (off, len) = find_attr(&req.data, ATTR_XOR_PEER_ADDRESS)
            .expect("XOR-PEER-ADDRESS not found");
        assert_eq!(len, 20); // IPv6: 20 bytes value
        assert_eq!(req.data[off + 5], AF_INET6); // family

        // Verify port
        let cookie_bytes = STUN_MAGIC_COOKIE.to_be_bytes();
        let x_port = u16::from_be_bytes([req.data[off + 6], req.data[off + 7]]);
        let decoded_port = x_port ^ u16::from_be_bytes([cookie_bytes[0], cookie_bytes[1]]);
        assert_eq!(decoded_port, 54321);

        // Verify address can be decoded
        let ip_bytes = ipv6_to_bytes("2001:db8::1").unwrap();
        let mut decoded = [0u8; 16];
        for i in 0..4 {
            decoded[i] = req.data[off + 8 + i] ^ cookie_bytes[i];
        }
        for i in 4..16 {
            decoded[i] = req.data[off + 8 + i] ^ tid[i - 4];
        }
        assert_eq!(decoded, ip_bytes);
    }

    #[test]
    fn test_parse_binding_response() {
        // Construct a valid Binding Success Response with XOR-MAPPED-ADDRESS
        let transaction_id = [0x11u8; 12];
        let cookie_bytes = STUN_MAGIC_COOKIE.to_be_bytes();

        // XOR-MAPPED-ADDRESS attribute value: reserved(1) + family(1) + x_port(2) + x_ip(4) = 8 bytes
        let ip: [u8; 4] = [10, 0, 0, 1];
        let port: u16 = 5555;
        let x_port = port ^ u16::from_be_bytes([cookie_bytes[0], cookie_bytes[1]]);
        let x_port_bytes = x_port.to_be_bytes();

        let mut attr_value = [0u8; 8];
        attr_value[0] = 0;       // reserved
        attr_value[1] = AF_INET; // family
        attr_value[2] = x_port_bytes[0];
        attr_value[3] = x_port_bytes[1];
        for i in 0..4 {
            attr_value[4 + i] = ip[i] ^ cookie_bytes[i];
        }

        // Build message: header(20) + attr(4 + 8) = 32 bytes
        let attrs_len: u16 = 12; // 4 (attr header) + 8 (attr value)
        let total_len = 20 + attrs_len as usize;
        let mut buf = vec![0u8; total_len];

        write_u16(&mut buf, 0, STUN_BINDING_SUCCESS);
        write_u16(&mut buf, 2, attrs_len);
        write_u32(&mut buf, 4, STUN_MAGIC_COOKIE);
        buf[8..20].copy_from_slice(&transaction_id);

        // XOR-MAPPED-ADDRESS attribute
        write_u16(&mut buf, 20, ATTR_XOR_MAPPED_ADDRESS);
        write_u16(&mut buf, 22, 8);
        buf[24..32].copy_from_slice(&attr_value);

        // Parse
        let result = parse_binding_response(&buf, &transaction_id).unwrap();
        assert_eq!(result.ip, "10.0.0.1");
        assert_eq!(result.port, 5555);
    }

    #[test]
    fn test_parse_binding_response_wrong_tid() {
        let transaction_id = [0x11u8; 12];
        let wrong_tid = [0x22u8; 12];
        let cookie_bytes = STUN_MAGIC_COOKIE.to_be_bytes();

        let x_port = (12345u16) ^ u16::from_be_bytes([cookie_bytes[0], cookie_bytes[1]]);
        let mut attr_value = [0u8; 8];
        attr_value[1] = AF_INET;
        attr_value[2..4].copy_from_slice(&x_port.to_be_bytes());
        attr_value[4..8].copy_from_slice(&[192u8 ^ cookie_bytes[0], 168u8 ^ cookie_bytes[1], 1u8 ^ cookie_bytes[2], 1u8 ^ cookie_bytes[3]]);

        let attrs_len: u16 = 12;
        let total_len = 20 + attrs_len as usize;
        let mut buf = vec![0u8; total_len];
        write_u16(&mut buf, 0, STUN_BINDING_SUCCESS);
        write_u16(&mut buf, 2, attrs_len);
        write_u32(&mut buf, 4, STUN_MAGIC_COOKIE);
        buf[8..20].copy_from_slice(&transaction_id);
        write_u16(&mut buf, 20, ATTR_XOR_MAPPED_ADDRESS);
        write_u16(&mut buf, 22, 8);
        buf[24..32].copy_from_slice(&attr_value);

        let result = parse_binding_response(&buf, &wrong_tid);
        assert!(matches!(result, Err(StunError::TransactionIdMismatch)));
    }

    #[test]
    fn test_parse_allocate_response_success() {
        let transaction_id = [0x33u8; 12];
        let cookie_bytes = STUN_MAGIC_COOKIE.to_be_bytes();

        // Build two XOR-address attributes: XOR-RELAYED-ADDRESS and XOR-MAPPED-ADDRESS
        let relay_ip: [u8; 4] = [172, 16, 0, 1];
        let relay_port: u16 = 3478;
        let x_relay_port = relay_port ^ u16::from_be_bytes([cookie_bytes[0], cookie_bytes[1]]);

        let mut relay_attr = [0u8; 8];
        relay_attr[1] = AF_INET;
        relay_attr[2..4].copy_from_slice(&x_relay_port.to_be_bytes());
        for i in 0..4 {
            relay_attr[4 + i] = relay_ip[i] ^ cookie_bytes[i];
        }

        let mapped_ip: [u8; 4] = [203, 0, 113, 5];
        let mapped_port: u16 = 9999;
        let x_mapped_port = mapped_port ^ u16::from_be_bytes([cookie_bytes[0], cookie_bytes[1]]);

        let mut mapped_attr = [0u8; 8];
        mapped_attr[1] = AF_INET;
        mapped_attr[2..4].copy_from_slice(&x_mapped_port.to_be_bytes());
        for i in 0..4 {
            mapped_attr[4 + i] = mapped_ip[i] ^ cookie_bytes[i];
        }

        // header(20) + relay_attr(12) + mapped_attr(12) = 44
        let attrs_len: u16 = 24;
        let total_len = 20 + attrs_len as usize;
        let mut buf = vec![0u8; total_len];

        write_u16(&mut buf, 0, STUN_ALLOCATE_SUCCESS);
        write_u16(&mut buf, 2, attrs_len);
        write_u32(&mut buf, 4, STUN_MAGIC_COOKIE);
        buf[8..20].copy_from_slice(&transaction_id);

        // XOR-RELAYED-ADDRESS
        write_u16(&mut buf, 20, ATTR_XOR_RELAYED_ADDRESS);
        write_u16(&mut buf, 22, 8);
        buf[24..32].copy_from_slice(&relay_attr);

        // XOR-MAPPED-ADDRESS
        write_u16(&mut buf, 32, ATTR_XOR_MAPPED_ADDRESS);
        write_u16(&mut buf, 34, 8);
        buf[36..44].copy_from_slice(&mapped_attr);

        let result = parse_allocate_response(&buf, &transaction_id).unwrap();
        assert_eq!(result.relay_ip, "172.16.0.1");
        assert_eq!(result.relay_port, 3478);
        assert_eq!(result.mapped_ip, "203.0.113.5");
        assert_eq!(result.mapped_port, 9999);
    }

    #[test]
    fn test_parse_allocate_response_error() {
        let transaction_id = [0x44u8; 12];

        // ERROR-CODE attribute: class=4, number=87 -> 487 Role Conflict
        let reason = b"Role Conflict";
        let reason_len = reason.len();
        let attr_val_len = 4 + reason_len;
        let padded_attr = pad_to_4(4 + attr_val_len);
        let attrs_len = padded_attr as u16;
        let total_len = 20 + attrs_len as usize;
        let mut buf = vec![0u8; total_len];

        write_u16(&mut buf, 0, STUN_ALLOCATE_ERROR);
        write_u16(&mut buf, 2, attrs_len);
        write_u32(&mut buf, 4, STUN_MAGIC_COOKIE);
        buf[8..20].copy_from_slice(&transaction_id);

        // ERROR-CODE attribute (0x0009)
        write_u16(&mut buf, 20, ATTR_ERROR_CODE);
        write_u16(&mut buf, 22, attr_val_len as u16);
        // ERROR-CODE value: 2 bytes reserved + class(1) + number(1) + reason
        buf[24] = 0; // reserved
        buf[25] = 0; // reserved
        buf[26] = 4; // class = 4
        buf[27] = 87; // number = 87
        buf[28..28 + reason_len].copy_from_slice(reason);

        let result = parse_allocate_response(&buf, &transaction_id);
        match result {
            Err(StunError::AllocateError { code, reason }) => {
                assert_eq!(code, 487);
                assert_eq!(reason, "Role Conflict");
            }
            other => panic!("expected AllocateError, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_binding_response_too_short() {
        let tid = [0u8; 12];
        let result = parse_binding_response(&[0u8; 10], &tid);
        assert!(matches!(result, Err(StunError::ResponseTooShort(10))));
    }
}
