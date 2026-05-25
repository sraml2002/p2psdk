//! SDP offer generation and SDP answer parsing for ICE-Lite P2P.

#[derive(Debug, Clone)]
pub struct SdpAnswerInfo {
    pub ufrag: String,
    pub pwd: String,
    pub is_lite: bool,
    pub candidates: Vec<String>,
}

/// Generate a standard SDP offer text.
/// `odid` is written to the SDP `o=` field (RFC 8866 origin username).
pub fn generate_sdp_offer(
    odid: &str,
    local_ufrag: &str,
    local_pwd: &str,
    candidates: &[String],
    default_ip: &str,
    default_port: u16,
) -> String {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut lines = Vec::new();
    lines.push("v=0".into());
    lines.push(format!("o={odid} {timestamp} 1 IN IP4 {default_ip}"));
    lines.push("s=-".into());
    lines.push("t=0 0".into());
    lines.push(format!("m=data {default_port} UDP/RTP/AVP 0"));
    lines.push(format!("c=IN IP4 {default_ip}"));
    lines.push(format!("a=ice-ufrag:{local_ufrag}"));
    lines.push(format!("a=ice-pwd:{local_pwd}"));

    for cand in candidates {
        lines.push(format!("a={cand}"));
    }

    let mut sdp = lines.join("\r\n");
    sdp.push_str("\r\n");
    sdp
}

/// Parse a standard SDP answer text.
pub fn parse_sdp_answer(sdp_text: &str) -> SdpAnswerInfo {
    let mut info = SdpAnswerInfo {
        ufrag: String::new(),
        pwd: String::new(),
        is_lite: false,
        candidates: Vec::new(),
    };

    for line in sdp_text.split("\r\n") {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("a=ice-ufrag:") {
            info.ufrag = val.into();
        } else if let Some(val) = trimmed.strip_prefix("a=ice-pwd:") {
            info.pwd = val.into();
        } else if trimmed == "a=ice-lite" {
            info.is_lite = true;
        } else if let Some(candidate_line) = trimmed.strip_prefix("a=candidate:") {
            info.candidates.push(format!("candidate:{candidate_line}"));
        }
    }

    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_sdp_offer() {
        let candidates = vec![
            "candidate:1 1 UDP 2130706431 192.168.1.1 12345 host".into(),
        ];
        let sdp = generate_sdp_offer("device-001", "ufrag123", "pwd456", &candidates, "192.168.1.1", 12345);
        assert!(sdp.starts_with("v=0\r\n"));
        assert!(sdp.contains("o=device-001"));
        assert!(sdp.contains("a=ice-ufrag:ufrag123"));
        assert!(sdp.contains("a=ice-pwd:pwd456"));
        assert!(sdp.contains("a=candidate:1 1 UDP"));
        assert!(sdp.ends_with("\r\n"));
    }

    #[test]
    fn test_parse_sdp_answer() {
        let sdp = "v=0\r\n\
                    o=- 1234 1 IN IP4 10.0.0.1\r\n\
                    s=-\r\n\
                    a=ice-ufrag:remote_ufrag\r\n\
                    a=ice-pwd:remote_pwd\r\n\
                    a=ice-lite\r\n\
                    a=candidate:1 1 UDP 1234 10.0.0.1 54321 host\r\n";
        let info = parse_sdp_answer(sdp);
        assert_eq!(info.ufrag, "remote_ufrag");
        assert_eq!(info.pwd, "remote_pwd");
        assert!(info.is_lite);
        assert_eq!(info.candidates.len(), 1);
        assert!(info.candidates[0].starts_with("candidate:1 1 UDP"));
    }

    #[test]
    fn test_parse_sdp_answer_no_lite() {
        let sdp = "a=ice-ufrag:abc\r\na=ice-pwd:def\r\n";
        let info = parse_sdp_answer(sdp);
        assert_eq!(info.ufrag, "abc");
        assert_eq!(info.pwd, "def");
        assert!(!info.is_lite);
        assert!(info.candidates.is_empty());
    }
}
