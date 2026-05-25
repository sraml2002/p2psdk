//! Candidate pair management: priority calculation, check list building, and lookup.

use crate::types::{CandidatePair, CandidateType, CheckState, IceCandidate, IceRole};

// ── Constants (matching IceAgent.ets) ──────────────────────────────────────────

const TYPE_PREF_HOST: u32 = 126;
const TYPE_PREF_SRFLX: u32 = 110;
const TYPE_PREF_RELAY: u32 = 0;
const LOCAL_PREF: u32 = 0;
const COMPONENT_ID: u32 = 1;

/// Maximum retransmissions per pair before failure.
pub const MAX_RETRANSMITS: u32 = 7;

/// Initial retransmission timeout (ms).
pub const INITIAL_RTO_MS: u64 = 500;

/// Maximum retransmission timeout cap (ms).
pub const MAX_RTO_MS: u64 = 16000;

/// Check loop interval (ms).
pub const CHECK_INTERVAL_MS: u64 = 50;

// ── Priority calculation ──────────────────────────────────────────────────────

/// Calculate candidate priority per RFC 8445 §5.1.2.
pub fn calc_candidate_priority(candidate_type: CandidateType) -> u32 {
    let type_pref = match candidate_type {
        CandidateType::Host => TYPE_PREF_HOST,
        CandidateType::Srflx => TYPE_PREF_SRFLX,
        CandidateType::Relay => TYPE_PREF_RELAY,
    };
    (type_pref << 24) | ((LOCAL_PREF & 0xFFFF) << 8) | (256 - COMPONENT_ID)
}

/// Calculate pair priority per RFC 8445 §6.1.2.4.
///
/// `G = controlling priority, D = controlled priority`.
/// `pairPriority = 2^32 * min(G,D) + 2 * max(G,D) + (G > D ? 1 : 0)`
pub fn calc_pair_priority(local_priority: u32, remote_priority: u32, role: IceRole) -> u64 {
    let (g, d) = match role {
        IceRole::Controlling => (local_priority as u64, remote_priority as u64),
        IceRole::Controlled => (remote_priority as u64, local_priority as u64),
    };
    let (min_gd, max_gd) = if g < d { (g, d) } else { (d, g) };
    (1u64 << 32) * min_gd + 2 * max_gd + if g > d { 1 } else { 0 }
}

// ── Check list operations ─────────────────────────────────────────────────────

/// Build all local × remote candidate pairs, sorted by descending priority.
pub fn build_check_list(
    local: &[IceCandidate],
    remote: &[IceCandidate],
    role: IceRole,
) -> Vec<CandidatePair> {
    let mut pairs = Vec::new();
    for l in local {
        for r in remote {
            let priority = calc_pair_priority(l.priority, r.priority, role);
            pairs.push(CandidatePair {
                local: l.clone(),
                remote: r.clone(),
                state: CheckState::Waiting,
                nominated: false,
                priority,
                retransmit_count: 0,
                retransmit_timer_ms: INITIAL_RTO_MS,
                last_sent_time_ms: 0,
                transaction_id: Vec::new(),
            });
        }
    }
    pairs.sort_by(|a, b| b.priority.cmp(&a.priority));
    pairs
}

/// Find pair index by transaction ID.
pub fn find_pair_by_tid(pairs: &[CandidatePair], tid: &[u8]) -> Option<usize> {
    pairs.iter().position(|p| p.transaction_id == tid)
}

/// Find pair index by remote address.
pub fn find_pair_by_remote(pairs: &[CandidatePair], ip: &str, port: u16) -> Option<usize> {
    pairs
        .iter()
        .position(|p| p.remote.connection_address == ip && p.remote.port == port)
}

/// Find the highest-priority succeeded pair.
pub fn find_best_succeeded(pairs: &[CandidatePair]) -> Option<usize> {
    pairs.iter().position(|p| p.state == CheckState::Succeeded)
}

/// True when every pair is SUCCEEDED or FAILED but none succeeded.
pub fn all_pairs_failed(pairs: &[CandidatePair]) -> bool {
    if pairs.is_empty() {
        return false;
    }
    let all_terminal = pairs
        .iter()
        .all(|p| matches!(p.state, CheckState::Succeeded | CheckState::Failed));
    let none_succeeded = pairs.iter().all(|p| p.state != CheckState::Succeeded);
    all_terminal && none_succeeded
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn host_cand(addr: &str, port: u16) -> IceCandidate {
        IceCandidate {
            foundation: format!("host-{addr}"),
            component_id: 1,
            transport: "UDP".into(),
            priority: calc_candidate_priority(CandidateType::Host),
            connection_address: addr.into(),
            port,
            candidate_type: CandidateType::Host,
            related_address: String::new(),
            related_port: 0,
        }
    }

    fn srflx_cand(addr: &str, port: u16) -> IceCandidate {
        IceCandidate {
            foundation: format!("srflx-{addr}"),
            component_id: 1,
            transport: "UDP".into(),
            priority: calc_candidate_priority(CandidateType::Srflx),
            connection_address: addr.into(),
            port,
            candidate_type: CandidateType::Srflx,
            related_address: String::new(),
            related_port: 0,
        }
    }

    #[test]
    fn test_priority_ordering() {
        let host = calc_candidate_priority(CandidateType::Host);
        let srflx = calc_candidate_priority(CandidateType::Srflx);
        let relay = calc_candidate_priority(CandidateType::Relay);
        assert!(host > srflx);
        assert!(srflx > relay);
        assert_eq!(host, 0x7E00_00FF);
    }

    #[test]
    fn test_pair_priority_controlling() {
        let local_p = calc_candidate_priority(CandidateType::Host);
        let remote_p = calc_candidate_priority(CandidateType::Srflx);
        let pp = calc_pair_priority(local_p, remote_p, IceRole::Controlling);
        let g = local_p as u64;
        let d = remote_p as u64;
        let expected = (1u64 << 32) * d.min(g) + 2 * d.max(g) + if g > d { 1 } else { 0 };
        assert_eq!(pp, expected);
    }

    #[test]
    fn test_build_check_list_ordering() {
        let local = vec![host_cand("192.168.1.1", 1234)];
        let remote = vec![
            host_cand("10.0.0.1", 5678),
            srflx_cand("203.0.113.1", 5678),
        ];
        let list = build_check_list(&local, &remote, IceRole::Controlling);
        assert_eq!(list.len(), 2);
        assert!(list[0].priority > list[1].priority);
    }

    #[test]
    fn test_find_pair_by_tid() {
        let local = vec![host_cand("1.1.1.1", 1)];
        let remote = vec![host_cand("2.2.2.2", 2)];
        let mut list = build_check_list(&local, &remote, IceRole::Controlling);
        assert!(find_pair_by_tid(&list, &[0; 12]).is_none());
        list[0].transaction_id = vec![0xAA; 12];
        assert_eq!(find_pair_by_tid(&list, &[0xAA; 12]), Some(0));
    }

    #[test]
    fn test_find_pair_by_remote() {
        let local = vec![host_cand("1.1.1.1", 1)];
        let remote = vec![host_cand("2.2.2.2", 2)];
        let list = build_check_list(&local, &remote, IceRole::Controlling);
        assert_eq!(find_pair_by_remote(&list, "2.2.2.2", 2), Some(0));
        assert!(find_pair_by_remote(&list, "3.3.3.3", 3).is_none());
    }

    #[test]
    fn test_all_pairs_failed() {
        let local = vec![host_cand("1.1.1.1", 1)];
        let remote = vec![host_cand("2.2.2.2", 2)];
        let mut list = build_check_list(&local, &remote, IceRole::Controlling);
        assert!(!all_pairs_failed(&list));
        list[0].state = CheckState::Failed;
        assert!(all_pairs_failed(&list));
        list[0].state = CheckState::Succeeded;
        assert!(!all_pairs_failed(&list));
    }
}
