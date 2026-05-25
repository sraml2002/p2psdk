//! ICE candidate encoding/decoding (SDP candidate line format).
//!
//! To be implemented in Phase 4.

use crate::types::{CandidateType, IceCandidate};

/// Parse a candidate line like "candidate:1 1 UDP 2130706431 192.168.1.1 12345 host"
pub fn parse_candidate_line(line: &str) -> Option<IceCandidate> {
    let stripped = line.strip_prefix("candidate:")?;
    let parts: Vec<&str> = stripped.split_whitespace().collect();
    if parts.len() < 7 {
        return None;
    }

    // parts[6] may be "typ" (RFC 5245 SDP attribute) or the type directly
    let type_idx = if parts.len() > 7 && parts[6] == "typ" { 7 } else { 6 };

    let mut related_address = String::new();
    let mut related_port: u16 = 0;
    // Look for "raddr <ip> rport <port>" in remaining parts
    for i in type_idx + 1..parts.len().saturating_sub(1) {
        if parts[i] == "raddr" {
            related_address = parts.get(i + 1).map(|s| (*s).into()).unwrap_or_default();
        }
        if parts[i] == "rport" {
            related_port = parts.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0);
        }
    }

    // parts[6] may be "typ" (RFC 5245 SDP attribute) or the type directly
    let type_idx = if parts.len() > 7 && parts[6] == "typ" { 7 } else { 6 };

    Some(IceCandidate {
        foundation: parts[0].into(),
        component_id: parts[1].parse().ok()?,
        transport: parts[2].into(),
        priority: parts[3].parse().ok()?,
        connection_address: parts[4].into(),
        port: parts[5].parse().ok()?,
        candidate_type: CandidateType::from_str(parts[type_idx])?,
        related_address,
        related_port,
    })
}

/// Format an IceCandidate as an SDP candidate line.
pub fn format_candidate_line(cand: &IceCandidate) -> String {
    let mut line = format!(
        "candidate:{} {} {} {} {} {} typ {}",
        cand.foundation,
        cand.component_id,
        cand.transport,
        cand.priority,
        cand.connection_address,
        cand.port,
        cand.candidate_type.as_str(),
    );
    if !cand.related_address.is_empty() {
        line.push_str(&format!(" raddr {} rport {}", cand.related_address, cand.related_port));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_candidate() {
        let line = "candidate:1 1 UDP 2130706431 192.168.1.100 54321 host";
        let cand = parse_candidate_line(line).unwrap();
        assert_eq!(cand.foundation, "1");
        assert_eq!(cand.component_id, 1);
        assert_eq!(cand.connection_address, "192.168.1.100");
        assert_eq!(cand.port, 54321);
        assert_eq!(cand.candidate_type, CandidateType::Host);
    }

    #[test]
    fn test_parse_srflx_candidate() {
        let line = "candidate:2 1 UDP 1694498815 203.0.113.1 54321 srflx raddr 192.168.1.100 rport 54321";
        let cand = parse_candidate_line(line).unwrap();
        assert_eq!(cand.candidate_type, CandidateType::Srflx);
        assert_eq!(cand.related_address, "192.168.1.100");
        assert_eq!(cand.related_port, 54321);
    }

    #[test]
    fn test_format_candidate() {
        let cand = IceCandidate {
            foundation: "1".into(),
            component_id: 1,
            transport: "UDP".into(),
            priority: 2130706431,
            connection_address: "192.168.1.100".into(),
            port: 54321,
            candidate_type: CandidateType::Host,
            related_address: String::new(),
            related_port: 0,
        };
        let line = format_candidate_line(&cand);
        assert_eq!(line, "candidate:1 1 UDP 2130706431 192.168.1.100 54321 typ host");
    }

    #[test]
    fn test_parse_candidate_with_typ() {
        let line = "candidate:1 1 UDP 100 203.0.113.45 49152 typ host";
        let cand = parse_candidate_line(line).unwrap();
        assert_eq!(cand.foundation, "1");
        assert_eq!(cand.connection_address, "203.0.113.45");
        assert_eq!(cand.port, 49152);
        assert_eq!(cand.candidate_type, CandidateType::Host);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_candidate_line("not a candidate").is_none());
        assert!(parse_candidate_line("candidate:1 1 UDP").is_none());
    }
}
