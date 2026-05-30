//! P2P data frame encoding/decoding.
//!
//! Frame format: [4B payload length BE][4B frame type BE][8B seqId BE][N bytes payload]

use crate::types::{TYPE_DATA, TYPE_HEARTBEAT};

#[derive(Debug, Clone)]
pub struct ParsedFrame {
    pub frame_type: u32,
    pub seq_id: u64,
    pub payload: Vec<u8>,
}

/// Encode a text message into a P2P data frame (seqId = 0, no correlation).
pub fn encode_data_frame(text: &str) -> Vec<u8> {
    encode_data_frame_with_seq(text, 0)
}

/// Encode a text message into a P2P data frame with a specific seqId.
pub fn encode_data_frame_with_seq(text: &str, seq_id: u64) -> Vec<u8> {
    let payload = text.as_bytes();
    let total_len = 16 + payload.len();
    let mut buf = Vec::with_capacity(total_len);
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&TYPE_DATA.to_be_bytes());
    buf.extend_from_slice(&seq_id.to_be_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// Encode a heartbeat reply frame (seqId = 0, minimal 1-byte payload).
pub fn encode_heartbeat_reply() -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.extend_from_slice(&1u32.to_be_bytes()); // payload length = 1
    buf.extend_from_slice(&TYPE_HEARTBEAT.to_be_bytes());
    buf.extend_from_slice(&0u64.to_be_bytes()); // seqId = 0
    buf.push(0x00); // minimal 1-byte payload
    buf
}

/// Parse a P2P frame from raw data. Returns None if data is too short.
pub fn parse_frame(data: &[u8]) -> Option<ParsedFrame> {
    if data.len() < 16 {
        return None;
    }
    let payload_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let frame_type = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let seq_id = u64::from_be_bytes([
        data[8], data[9], data[10], data[11],
        data[12], data[13], data[14], data[15],
    ]);

    if 16 + payload_len > data.len() {
        return None;
    }

    Some(ParsedFrame {
        frame_type,
        seq_id,
        payload: data[16..16 + payload_len].to_vec(),
    })
}

/// Check if data is a STUN message.
/// STUN: first 2 bits are 0 and bytes[4:8] == magic cookie 0x2112A442.
pub fn is_stun_message(data: &[u8]) -> bool {
    if data.len() < 8 {
        return false;
    }
    let first_two_bits = (data[0] >> 6) & 0x03;
    if first_two_bits != 0 {
        return false;
    }
    let magic = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    magic == 0x2112A442
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_data_frame() {
        let frame = encode_data_frame("hello");
        // 4B len (5) + 4B type (0x02) + 8B seqId (0) + 5B payload = 21
        assert_eq!(frame.len(), 21);
        assert_eq!(&frame[0..4], &5u32.to_be_bytes());
        assert_eq!(&frame[4..8], &TYPE_DATA.to_be_bytes());
        assert_eq!(&frame[8..16], &0u64.to_be_bytes());
        assert_eq!(&frame[16..21], b"hello");
    }

    #[test]
    fn test_encode_data_frame_with_seq() {
        let frame = encode_data_frame_with_seq("hello", 42);
        assert_eq!(frame.len(), 21);
        assert_eq!(&frame[8..16], &42u64.to_be_bytes());
    }

    #[test]
    fn test_encode_heartbeat_reply() {
        let frame = encode_heartbeat_reply();
        // 4B len (1) + 4B type + 8B seqId (0) + 1B payload = 17
        assert_eq!(frame.len(), 17);
        assert_eq!(&frame[0..4], &1u32.to_be_bytes());
        assert_eq!(&frame[4..8], &TYPE_HEARTBEAT.to_be_bytes());
        assert_eq!(&frame[8..16], &0u64.to_be_bytes());
    }

    #[test]
    fn test_parse_frame_roundtrip() {
        let encoded = encode_data_frame("test message");
        let parsed = parse_frame(&encoded).unwrap();
        assert_eq!(parsed.frame_type, TYPE_DATA);
        assert_eq!(parsed.seq_id, 0);
        assert_eq!(String::from_utf8(parsed.payload).unwrap(), "test message");
    }

    #[test]
    fn test_parse_frame_with_seq() {
        let encoded = encode_data_frame_with_seq("hello", 12345);
        let parsed = parse_frame(&encoded).unwrap();
        assert_eq!(parsed.seq_id, 12345);
        assert_eq!(String::from_utf8(parsed.payload).unwrap(), "hello");
    }

    #[test]
    fn test_parse_frame_too_short() {
        assert!(parse_frame(&[0; 15]).is_none());
    }

    #[test]
    fn test_parse_frame_incomplete_payload() {
        let mut data = vec![0u8; 16];
        data[0..4].copy_from_slice(&100u32.to_be_bytes()); // claims 100 bytes
        data[4..8].copy_from_slice(&TYPE_DATA.to_be_bytes());
        assert!(parse_frame(&data).is_none());
    }

    #[test]
    fn test_is_stun_message() {
        let mut data = vec![0u8; 20];
        // Valid STUN: first 2 bits = 0, magic cookie at [4:8]
        data[4..8].copy_from_slice(&0x2112A442u32.to_be_bytes());
        assert!(is_stun_message(&data));

        // Not STUN: first 2 bits set
        data[0] = 0xC0;
        assert!(!is_stun_message(&data));

        // Not STUN: wrong magic cookie
        data[0] = 0x00;
        data[4..8].copy_from_slice(&0x12345678u32.to_be_bytes());
        assert!(!is_stun_message(&data));
    }
}
