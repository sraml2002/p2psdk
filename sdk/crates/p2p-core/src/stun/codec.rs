//! STUN protocol utility functions: padding, transaction ID generation,
//! IPv6 address conversion, XOR-MAPPED-ADDRESS parsing, and constants.

use std::fmt;

use rand::Rng;

// Re-export constants from types.rs so codec consumers can import them here.
pub use crate::types::{
    AF_INET, AF_INET6, ATTR_ERROR_CODE, ATTR_FINGERPRINT, ATTR_P2P_TOKEN,
    ATTR_REQUESTED_ADDRESS_FAMILY, ATTR_REQUESTED_TRANSPORT, ATTR_SOFTWARE,
    ATTR_XOR_MAPPED_ADDRESS, ATTR_XOR_PEER_ADDRESS, ATTR_XOR_RELAYED_ADDRESS,
    STUN_ALLOCATE_ERROR, STUN_ALLOCATE_REQUEST, STUN_ALLOCATE_SUCCESS,
    STUN_BINDING_ERROR, STUN_BINDING_REQUEST, STUN_BINDING_SUCCESS,
    STUN_CREATE_PERMISSION_ERROR, STUN_CREATE_PERMISSION_REQUEST,
    STUN_CREATE_PERMISSION_SUCCESS, STUN_MAGIC_COOKIE,
};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by STUN codec functions.
#[derive(Debug)]
pub enum StunCodecError {
    /// Input buffer too short for the requested operation.
    BufferTooShort {
        need: usize,
        have: usize,
        context: &'static str,
    },
    /// Unsupported address family in XOR address attribute.
    UnsupportedFamily(u8),
    /// Malformed IPv6 address string.
    InvalidIpv6(String),
}

impl std::error::Error for StunCodecError {}

impl fmt::Display for StunCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferTooShort { need, have, context } => {
                write!(f, "buffer too short for {context}: need {need}, have {have}")
            }
            Self::UnsupportedFamily(family) => {
                write!(f, "unsupported address family: 0x{family:02x}")
            }
            Self::InvalidIpv6(addr) => {
                write!(f, "invalid IPv6 address: {addr}")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

/// Result of a STUN Binding response (external mapped address).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StunResult {
    pub ip: String,
    pub port: u16,
}

/// Result of a TURN Allocate response (relay + mapped address).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnResult {
    pub relay_ip: String,
    pub relay_port: u16,
    pub mapped_ip: String,
    pub mapped_port: u16,
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Round `n` up to the next multiple of 4 (4-byte alignment padding).
///
/// STUN attributes must be aligned to 4-byte boundaries. This computes the
/// padded length for a given raw length.
pub fn pad_to_4(n: usize) -> usize {
    (n + 3) & !3
}

/// Generate a random 12-byte STUN transaction ID (RFC 5389 §6).
pub fn generate_transaction_id() -> [u8; 12] {
    let mut tid = [0u8; 12];
    rand::thread_rng().fill(&mut tid);
    tid
}

/// Convert an IPv6 address string to 16 big-endian bytes.
///
/// Handles `::` shorthand expansion (e.g. `"2001:db8::1"`).
/// Returns an error for malformed addresses.
pub fn ipv6_to_bytes(ip: &str) -> Result<[u8; 16], StunCodecError> {
    let mut bytes = [0u8; 16];

    let halves: Vec<&str> = ip.split("::").collect();
    let groups: Vec<&str> = if halves.len() == 2 {
        let left: Vec<&str> = if halves[0].is_empty() {
            vec![]
        } else {
            halves[0].split(':').collect()
        };
        let right: Vec<&str> = if halves[1].is_empty() {
            vec![]
        } else {
            halves[1].split(':').collect()
        };
        if left.len() + right.len() > 8 {
            return Err(StunCodecError::InvalidIpv6(ip.to_string()));
        }
        let missing = 8 - left.len() - right.len();
        let mut g: Vec<&str> = left;
        for _ in 0..missing {
            g.push("0");
        }
        g.extend(right);
        g
    } else if halves.len() == 1 {
        ip.split(':').collect()
    } else {
        // Multiple `::` occurrences -- invalid
        return Err(StunCodecError::InvalidIpv6(ip.to_string()));
    };

    if groups.len() != 8 {
        return Err(StunCodecError::InvalidIpv6(ip.to_string()));
    }

    for (i, g) in groups.iter().enumerate() {
        let val = u16::from_str_radix(g, 16)
            .map_err(|_| StunCodecError::InvalidIpv6(ip.to_string()))?;
        bytes[i * 2] = (val >> 8) as u8;
        bytes[i * 2 + 1] = (val & 0xFF) as u8;
    }

    Ok(bytes)
}

/// Convert 16 big-endian bytes to an IPv6 address string (colon-separated hex groups).
///
/// Does not perform `::` compression -- outputs all 8 groups.
pub fn ipv6_to_string(bytes: &[u8; 16]) -> String {
    let groups: Vec<String> = (0..8)
        .map(|i| {
            let val = ((bytes[i * 2] as u16) << 8) | (bytes[i * 2 + 1] as u16);
            format!("{val:x}")
        })
        .collect();
    groups.join(":")
}

/// Parse an XOR-MAPPED-ADDRESS / XOR-RELAYED-ADDRESS attribute value.
///
/// * `buf` -- the full STUN message buffer
/// * `offset` -- byte offset of the attribute **value** (the byte right after the 4-byte
///   attribute header: reserved(1) + family(1) + x_port(2) + x_addr(4|16))
/// * `attr_len` -- length field from the attribute header
/// * `transaction_id` -- 12-byte transaction ID from the STUN header (bytes 8..20)
///
/// XOR rules (RFC 5389 §15.2):
/// - Port is XOR'd with the high 16 bits of the magic cookie (`0x2112`).
/// - IPv4 address is XOR'd with the full 32-bit magic cookie.
/// - IPv6 address is XOR'd with the 32-bit magic cookie concatenated with the 12-byte
///   transaction ID (16 bytes total).
///
/// Returns `(ip_string, port)` on success.
pub fn parse_xor_address(
    buf: &[u8],
    offset: usize,
    attr_len: usize,
    transaction_id: &[u8],
) -> Result<(String, u16), StunCodecError> {
    // Need at least: reserved(1) + family(1) + x_port(2) = 4 bytes
    if attr_len < 4 {
        return Err(StunCodecError::BufferTooShort {
            need: 4,
            have: attr_len,
            context: "XOR address attribute value",
        });
    }

    let needed = offset + 4; // offset to end of x_port
    if buf.len() < needed {
        return Err(StunCodecError::BufferTooShort {
            need: needed,
            have: buf.len(),
            context: "XOR address buffer",
        });
    }

    let family = buf[offset + 1];
    // XOR port with high 16 bits of magic cookie
    let x_port = u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]) ^ 0x2112;

    match family {
        AF_INET => {
            if attr_len < 8 {
                return Err(StunCodecError::BufferTooShort {
                    need: 8,
                    have: attr_len,
                    context: "XOR IPv4 address",
                });
            }
            let addr_end = offset + 8;
            if buf.len() < addr_end {
                return Err(StunCodecError::BufferTooShort {
                    need: addr_end,
                    have: buf.len(),
                    context: "XOR IPv4 buffer",
                });
            }
            let mc = STUN_MAGIC_COOKIE.to_be_bytes();
            let b0 = buf[offset + 4] ^ mc[0];
            let b1 = buf[offset + 5] ^ mc[1];
            let b2 = buf[offset + 6] ^ mc[2];
            let b3 = buf[offset + 7] ^ mc[3];
            let ip = format!("{b0}.{b1}.{b2}.{b3}");
            log::debug!("Parsed XOR IPv4 address: {}:{}", ip, x_port);
            Ok((ip, x_port))
        }
        AF_INET6 => {
            if attr_len < 20 {
                return Err(StunCodecError::BufferTooShort {
                    need: 20,
                    have: attr_len,
                    context: "XOR IPv6 address",
                });
            }
            let addr_end = offset + 20;
            if buf.len() < addr_end {
                return Err(StunCodecError::BufferTooShort {
                    need: addr_end,
                    have: buf.len(),
                    context: "XOR IPv6 buffer",
                });
            }
            let mc = STUN_MAGIC_COOKIE.to_be_bytes();
            let mut addr_bytes = [0u8; 16];
            for i in 0..16 {
                // First 4 bytes XOR with magic cookie, remaining 12 with transaction ID
                let xor_key = if i < 4 { mc[i] } else { transaction_id[i - 4] };
                addr_bytes[i] = buf[offset + 4 + i] ^ xor_key;
            }
            let ip = ipv6_to_string(&addr_bytes);
            log::debug!("Parsed XOR IPv6 address: [{}]:{}", ip, x_port);
            Ok((ip, x_port))
        }
        other => Err(StunCodecError::UnsupportedFamily(other)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pad_to_4() {
        assert_eq!(pad_to_4(0), 0);
        assert_eq!(pad_to_4(1), 4);
        assert_eq!(pad_to_4(2), 4);
        assert_eq!(pad_to_4(3), 4);
        assert_eq!(pad_to_4(4), 4);
        assert_eq!(pad_to_4(5), 8);
        assert_eq!(pad_to_4(7), 8);
        assert_eq!(pad_to_4(8), 8);
        assert_eq!(pad_to_4(12), 12);
        assert_eq!(pad_to_4(13), 16);
        assert_eq!(pad_to_4(100), 100);
        assert_eq!(pad_to_4(101), 104);
    }

    #[test]
    fn test_generate_transaction_id() {
        let tid1 = generate_transaction_id();
        assert_eq!(tid1.len(), 12, "transaction ID must be 12 bytes");

        let tid2 = generate_transaction_id();
        assert_eq!(tid2.len(), 12);
        assert_ne!(
            tid1, tid2,
            "two generated transaction IDs should differ (statistically)"
        );
    }

    #[test]
    fn test_ipv6_roundtrip() {
        // Full form -- leading zeros are stripped by ipv6_to_string
        let ip_full = "2001:0db8:0001:0000:0000:0000:0000:0001";
        let bytes = ipv6_to_bytes(ip_full).unwrap();
        let roundtrip = ipv6_to_string(&bytes);
        assert_eq!(roundtrip, "2001:db8:1:0:0:0:0:1");

        // Address without leading zeros -- exact roundtrip
        let ip2 = "fe80:0:0:0:1234:5678:9abc:def0";
        let bytes2 = ipv6_to_bytes(ip2).unwrap();
        let rt2 = ipv6_to_string(&bytes2);
        assert_eq!(rt2, "fe80:0:0:0:1234:5678:9abc:def0");

        // :: shorthand expansion
        let ip3 = "2001:db8::1";
        let bytes3 = ipv6_to_bytes(ip3).unwrap();
        let rt3 = ipv6_to_string(&bytes3);
        assert_eq!(rt3, "2001:db8:0:0:0:0:0:1");

        // ::1 (loopback)
        let ip4 = "::1";
        let bytes4 = ipv6_to_bytes(ip4).unwrap();
        let rt4 = ipv6_to_string(&bytes4);
        assert_eq!(rt4, "0:0:0:0:0:0:0:1");

        // :: (all zeros)
        let ip5 = "::";
        let bytes5 = ipv6_to_bytes(ip5).unwrap();
        let rt5 = ipv6_to_string(&bytes5);
        assert_eq!(rt5, "0:0:0:0:0:0:0:0");

        // Leading :: expansion
        let ip7 = "::abcd:ef01";
        let bytes7 = ipv6_to_bytes(ip7).unwrap();
        let rt7 = ipv6_to_string(&bytes7);
        assert_eq!(rt7, "0:0:0:0:0:0:abcd:ef01");

        // Trailing :: expansion
        let ip8 = "fe80::";
        let bytes8 = ipv6_to_bytes(ip8).unwrap();
        let rt8 = ipv6_to_string(&bytes8);
        assert_eq!(rt8, "fe80:0:0:0:0:0:0:0");
    }

    #[test]
    fn test_ipv6_errors() {
        // Too many groups
        assert!(ipv6_to_bytes("1:2:3:4:5:6:7:8:9").is_err());
        // Multiple ::
        assert!(ipv6_to_bytes("1::2::3").is_err());
        // Invalid hex
        assert!(ipv6_to_bytes("2001:gggg::1").is_err());
    }

    #[test]
    fn test_parse_xor_address_ipv4() {
        // Construct a known XOR-MAPPED-ADDRESS attribute value for IPv4.
        // Target: 192.168.1.100 : 12345
        let target_ip: [u8; 4] = [192, 168, 1, 100];
        let target_port: u16 = 12345;

        // XOR port with 0x2112
        let x_port = target_port ^ 0x2112;
        // XOR address with magic cookie bytes
        let mc = STUN_MAGIC_COOKIE.to_be_bytes();
        let x_addr: [u8; 4] = [
            target_ip[0] ^ mc[0],
            target_ip[1] ^ mc[1],
            target_ip[2] ^ mc[2],
            target_ip[3] ^ mc[3],
        ];

        // Build attribute value: reserved(1) + family(1) + x_port(2) + x_addr(4)
        let mut attr = [0u8; 8];
        attr[0] = 0; // reserved
        attr[1] = AF_INET;
        attr[2..4].copy_from_slice(&x_port.to_be_bytes());
        attr[4..8].copy_from_slice(&x_addr);

        // Place attr into a buffer with a 20-byte STUN header prefix.
        // The transaction ID occupies bytes 8..20.
        let mut buf = vec![0u8; 20];
        let transaction_id = [0xAAu8; 12];
        buf[8..20].copy_from_slice(&transaction_id);
        buf.extend_from_slice(&attr);

        // offset = 20: attribute value starts right after the 20-byte header
        let result = parse_xor_address(&buf, 20, 8, &transaction_id).unwrap();
        assert_eq!(result.0, "192.168.1.100");
        assert_eq!(result.1, 12345);
    }

    #[test]
    fn test_parse_xor_address_ipv6() {
        // Target: 2001:db8::1 : 50000
        let target_ip_bytes = ipv6_to_bytes("2001:db8::1").unwrap();
        let target_port: u16 = 50000;

        // XOR port with 0x2112
        let x_port = target_port ^ 0x2112;

        // XOR address with magic_cookie (4 bytes) + transaction_id (12 bytes)
        let mc = STUN_MAGIC_COOKIE.to_be_bytes();
        let transaction_id: [u8; 12] = [0xBB; 12];
        let mut x_addr = [0u8; 16];
        for i in 0..4 {
            x_addr[i] = target_ip_bytes[i] ^ mc[i];
        }
        for i in 0..12 {
            x_addr[4 + i] = target_ip_bytes[4 + i] ^ transaction_id[i];
        }

        // Build attribute value: reserved(1) + family(1) + x_port(2) + x_addr(16)
        let mut attr = [0u8; 20];
        attr[0] = 0;
        attr[1] = AF_INET6;
        attr[2..4].copy_from_slice(&x_port.to_be_bytes());
        attr[4..20].copy_from_slice(&x_addr);

        let mut buf = vec![0u8; 20];
        buf[8..20].copy_from_slice(&transaction_id);
        buf.extend_from_slice(&attr);

        let result = parse_xor_address(&buf, 20, 20, &transaction_id).unwrap();
        assert_eq!(result.0, "2001:db8:0:0:0:0:0:1");
        assert_eq!(result.1, 50000);
    }

    #[test]
    fn test_parse_xor_address_too_short() {
        let buf = [0u8; 20];
        let tid = [0u8; 12];
        // attr_len = 2, which is less than the required minimum of 4
        let result = parse_xor_address(&buf, 0, 2, &tid);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_xor_address_unsupported_family() {
        let mut attr = [0u8; 8];
        attr[1] = 0x03; // unsupported family
        let mut buf = vec![0u8; 20];
        let tid = [0u8; 12];
        buf[8..20].copy_from_slice(&tid);
        buf.extend_from_slice(&attr);

        let result = parse_xor_address(&buf, 20, 8, &tid);
        assert!(result.is_err());
    }

    #[test]
    fn test_stun_result_struct() {
        let r = StunResult {
            ip: "1.2.3.4".to_string(),
            port: 5678,
        };
        assert_eq!(r.ip, "1.2.3.4");
        assert_eq!(r.port, 5678);
    }

    #[test]
    fn test_turn_result_struct() {
        let r = TurnResult {
            relay_ip: "10.0.0.1".to_string(),
            relay_port: 40000,
            mapped_ip: "192.168.1.1".to_string(),
            mapped_port: 12345,
        };
        assert_eq!(r.relay_ip, "10.0.0.1");
        assert_eq!(r.relay_port, 40000);
        assert_eq!(r.mapped_ip, "192.168.1.1");
        assert_eq!(r.mapped_port, 12345);
    }
}
