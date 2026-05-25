//! ICE Agent state machine (RFC 8445).
//!
//! Sans-IO design: the agent never touches real sockets. It returns actions
//! for the caller to execute (send UDP datagrams) and accepts inbound data
//! via `handle_incoming_data`.

use rand::Rng;

use crate::ice::candidate::{format_candidate_line, parse_candidate_line};
use crate::ice::check_list::{
    calc_candidate_priority, INITIAL_RTO_MS, MAX_RETRANSMITS, MAX_RTO_MS,
};
use crate::ice::stun_codec::{build_ice_binding_request, build_ice_binding_response, parse_ice_stun_message};
use crate::types::{
    IceCandidate, IceRole, IceSessionDescription, IceState, CheckState, CandidateType,
    ICE_ERROR_ROLE_CONFLICT,
};

// ---------------------------------------------------------------------------
// Config & action types
// ---------------------------------------------------------------------------

/// ICE agent configuration.
#[derive(Debug, Clone)]
pub struct IceAgentConfig {
    pub is_controlling: bool,
}

impl Default for IceAgentConfig {
    fn default() -> Self {
        Self { is_controlling: true }
    }
}

/// An action the caller must execute on behalf of the ICE agent.
#[derive(Debug, Clone)]
pub struct IceAction {
    pub data: Vec<u8>,
    pub target_ip: String,
    pub target_port: u16,
}

/// Result of processing incoming UDP data.
#[derive(Debug)]
pub struct HandleDataResult {
    /// Non-STUN application data (if any).
    pub app_data: Option<Vec<u8>>,
    /// STUN responses or retransmissions to send.
    pub actions: Vec<IceAction>,
}

// ---------------------------------------------------------------------------
// Internal pair representation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct PairEntry {
    local: IceCandidate,
    remote: IceCandidate,
    state: CheckState,
    nominated: bool,
    priority: u64,
    retransmit_count: u32,
    retransmit_timer_ms: u64,
    last_sent_time_ms: u64,
    transaction_id: Vec<u8>,
}

// ---------------------------------------------------------------------------
// IceAgent
// ---------------------------------------------------------------------------

pub struct IceAgent {
    state: IceState,
    local_ufrag: String,
    local_pwd: String,
    tie_breaker: u64,
    role: IceRole,

    local_candidates: Vec<IceCandidate>,
    remote_candidates: Vec<IceCandidate>,
    remote_ufrag: String,
    remote_pwd: String,
    remote_is_lite: bool,

    check_list: Vec<PairEntry>,
    nominated_pair_idx: Option<usize>,
    nominated_pair_confirmed: bool,
}

impl IceAgent {
    // ── Construction ──────────────────────────────────────────────────────────

    pub fn new(config: IceAgentConfig) -> Self {
        let role = if config.is_controlling {
            IceRole::Controlling
        } else {
            IceRole::Controlled
        };
        let mut rng = rand::thread_rng();
        let local_ufrag: String = (0..4).map(|_| format!("{:02x}", rng.gen::<u8>())).collect();
        let local_pwd: String = (0..16).map(|_| format!("{:02x}", rng.gen::<u8>())).collect();

        Self {
            state: IceState::New,
            local_ufrag,
            local_pwd,
            tie_breaker: rng.gen(),
            role,
            local_candidates: Vec::new(),
            remote_candidates: Vec::new(),
            remote_ufrag: String::new(),
            remote_pwd: String::new(),
            remote_is_lite: false,
            check_list: Vec::new(),
            nominated_pair_idx: None,
            nominated_pair_confirmed: false,
        }
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn state(&self) -> IceState { self.state }
    pub fn local_ufrag(&self) -> &str { &self.local_ufrag }
    pub fn local_pwd(&self) -> &str { &self.local_pwd }
    pub fn role(&self) -> IceRole { self.role }
    pub fn local_candidates(&self) -> &[IceCandidate] { &self.local_candidates }

    pub fn nominated_remote(&self) -> Option<(&str, u16)> {
        self.nominated_pair_idx
            .and_then(|i| self.check_list.get(i))
            .map(|p| (p.remote.connection_address.as_str(), p.remote.port))
    }

    // ── Local candidates ──────────────────────────────────────────────────────

    pub fn add_local_candidate(&mut self, cand: IceCandidate) {
        self.local_candidates.push(cand);
    }

    pub fn add_host_candidate(&mut self, addr: &str, port: u16) {
        let priority = calc_candidate_priority(CandidateType::Host);
        self.local_candidates.push(IceCandidate {
            foundation: format!("host-{addr}"),
            component_id: 1,
            transport: "UDP".into(),
            priority,
            connection_address: addr.into(),
            port,
            candidate_type: CandidateType::Host,
            related_address: String::new(),
            related_port: 0,
        });
    }

    pub fn local_session_description(&self) -> IceSessionDescription {
        IceSessionDescription {
            ice_ufrag: self.local_ufrag.clone(),
            ice_pwd: self.local_pwd.clone(),
            is_lite: false,
            candidates: self.local_candidates.iter().map(format_candidate_line).collect(),
        }
    }

    // ── Remote session ────────────────────────────────────────────────────────

    pub fn set_remote_session_description(&mut self, desc: &IceSessionDescription) {
        self.remote_ufrag = desc.ice_ufrag.clone();
        self.remote_pwd = desc.ice_pwd.clone();
        self.remote_is_lite = desc.is_lite;
        for line in &desc.candidates {
            if let Some(c) = parse_candidate_line(line) {
                self.add_remote_cand(c);
            }
        }
    }

    pub fn add_remote_candidate(&mut self, line: &str) {
        if let Some(c) = parse_candidate_line(line) {
            self.add_remote_cand(c);
        }
    }

    pub fn set_remote_pwd(&mut self, pwd: &str) { self.remote_pwd = pwd.into(); }

    fn add_remote_cand(&mut self, cand: IceCandidate) {
        let dup = self.remote_candidates.iter().any(|c| {
            c.connection_address == cand.connection_address && c.port == cand.port
        });
        if !dup {
            self.remote_candidates.push(cand);
        }
    }

    // ── Check list ────────────────────────────────────────────────────────────

    /// Build the check list and transition to CONNECTING.
    pub fn start_checks(&mut self) -> Result<(), String> {
        if self.remote_ufrag.is_empty() {
            Self::set_state(self,IceState::Failed);
            return Err("remote ufrag not set".into());
        }
        if self.local_candidates.is_empty() || self.remote_candidates.is_empty() {
            Self::set_state(self,IceState::Failed);
            return Err("no candidates to check".into());
        }

        let mut pairs = Vec::new();
        for l in &self.local_candidates {
            for r in &self.remote_candidates {
                let (g, d) = match self.role {
                    IceRole::Controlling => (l.priority as u64, r.priority as u64),
                    IceRole::Controlled => (r.priority as u64, l.priority as u64),
                };
                let (min_gd, max_gd) = if g < d { (g, d) } else { (d, g) };
                let priority = (1u64 << 32) * min_gd + 2 * max_gd + if g > d { 1 } else { 0 };

                pairs.push(PairEntry {
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
        self.check_list = pairs;
        Self::set_state(self,IceState::Connecting);
        Ok(())
    }

    // ── Check cycle ───────────────────────────────────────────────────────────

    /// Drive one check-cycle iteration. Call every ~50 ms.
    /// Returns STUN Binding Requests that must be sent.
    pub fn tick(&mut self, now_ms: u64) -> Vec<IceAction> {
        if matches!(self.state, IceState::Completed | IceState::Failed | IceState::Closed) {
            return Vec::new();
        }

        let frozen = self.nominated_pair_idx.is_some();
        let mut actions = Vec::new();

        for i in 0..self.check_list.len() {
            let pair = &mut self.check_list[i];
            if matches!(pair.state, CheckState::Succeeded | CheckState::Failed) { continue; }
            if frozen && self.nominated_pair_idx != Some(i) { continue; }
            if !matches!(pair.state, CheckState::Waiting | CheckState::InProgress) { continue; }
            if now_ms < pair.last_sent_time_ms + pair.retransmit_timer_ms { continue; }

            if pair.state == CheckState::Waiting {
                pair.state = CheckState::InProgress;
            }

            let username = format!("{}:{}", self.remote_ufrag, self.local_ufrag);
            let remote_pwd = if self.remote_pwd.is_empty() { None } else { Some(self.remote_pwd.as_str()) };

            let result = build_ice_binding_request(
                &username,
                pair.local.priority,
                matches!(self.role, IceRole::Controlling),
                self.tie_breaker,
                pair.nominated,
                remote_pwd,
            );

            pair.transaction_id = result.transaction_id.to_vec();
            pair.last_sent_time_ms = now_ms;
            pair.retransmit_count += 1;

            if pair.retransmit_count >= MAX_RETRANSMITS {
                pair.state = CheckState::Failed;
            } else {
                pair.retransmit_timer_ms = (pair.retransmit_timer_ms * 2).min(MAX_RTO_MS);
            }

            actions.push(IceAction {
                data: result.data,
                target_ip: pair.remote.connection_address.clone(),
                target_port: pair.remote.port,
            });
        }

        self.check_completion();
        actions
    }

    // ── Incoming data ─────────────────────────────────────────────────────────

    pub fn handle_incoming_data(
        &mut self,
        data: &[u8],
        from_ip: &str,
        from_port: u16,
    ) -> HandleDataResult {
        let parsed = parse_ice_stun_message(data);
        if !parsed.is_stun {
            return HandleDataResult { app_data: Some(data.to_vec()), actions: Vec::new() };
        }
        if parsed.is_request {
            self.handle_incoming_stun(data, &parsed, from_ip, from_port)
        } else {
            self.handle_stun_response(&parsed)
        }
    }

    // ── Send app data ─────────────────────────────────────────────────────────

    pub fn send_data(&self, data: &[u8]) -> Option<IceAction> {
        let pair = self.nominated_pair_idx.and_then(|i| self.check_list.get(i))?;
        Some(IceAction {
            data: data.to_vec(),
            target_ip: pair.remote.connection_address.clone(),
            target_port: pair.remote.port,
        })
    }

    // ── Teardown ──────────────────────────────────────────────────────────────

    pub fn stop(&mut self) {
        Self::set_state(self,IceState::Closed);
        self.check_list.clear();
        self.nominated_pair_idx = None;
    }

    // ── Private ───────────────────────────────────────────────────────────────

    fn set_state(agent: &mut Self, s: IceState) {
        if agent.state == s { return; }
        log::debug!("ICE state: {} -> {}", agent.state, s);
        agent.state = s;
    }

    fn handle_incoming_stun(
        &mut self,
        raw: &[u8],
        parsed: &crate::ice::stun_codec::ParsedStunMessage,
        from_ip: &str,
        from_port: u16,
    ) -> HandleDataResult {
        let mut actions = Vec::new();

        // Validate USERNAME: format is "remoteUfrag:localUfrag" (sender's perspective).
        // As receiver, parts[0] should be our ufrag, parts[1] is the sender's.
        if !parsed.username.is_empty() {
            let mut parts = parsed.username.splitn(2, ':');
            match (parts.next(), parts.next()) {
                (Some(our), Some(their)) => {
                    if our != self.local_ufrag || their != self.remote_ufrag {
                        return HandleDataResult { app_data: None, actions };
                    }
                }
                _ => return HandleDataResult { app_data: None, actions },
            }
        }

        // Role conflict
        if let Some(peer_ctrl) = parsed.controlling {
            let local_ctrl = matches!(self.role, IceRole::Controlling);
            if peer_ctrl == local_ctrl && local_ctrl {
                self.role = IceRole::Controlled;
                log::debug!("Role conflict: switched to Controlled");
            }
        }

        // Respond
        let resp = match build_ice_binding_response(raw) {
            Some(r) => r,
            None => return HandleDataResult { app_data: None, actions },
        };
        actions.push(IceAction {
            data: resp,
            target_ip: from_ip.into(),
            target_port: from_port,
        });

        // Mark pair
        let idx = self.check_list.iter().position(|p| {
            p.remote.connection_address == from_ip && p.remote.port == from_port
        });
        if let Some(idx) = idx {
            let pair = &mut self.check_list[idx];
            if pair.state != CheckState::Succeeded {
                pair.state = CheckState::Succeeded;
            }
            if parsed.use_candidate && matches!(self.role, IceRole::Controlled) {
                pair.nominated = true;
                self.nominated_pair_idx = Some(idx);
                if !self.nominated_pair_confirmed {
                    self.nominated_pair_confirmed = true;
                    Self::set_state(self, IceState::Completed);
                }
            }
        }

        if matches!(self.state, IceState::New | IceState::Gathering | IceState::Connecting) {
            Self::set_state(self, IceState::Connected);
        }

        HandleDataResult { app_data: None, actions }
    }

    fn handle_stun_response(
        &mut self,
        parsed: &crate::ice::stun_codec::ParsedStunMessage,
    ) -> HandleDataResult {
        // 487 role conflict
        if parsed.is_error_response && parsed.error_code == ICE_ERROR_ROLE_CONFLICT {
            self.role = match self.role {
                IceRole::Controlling => IceRole::Controlled,
                IceRole::Controlled => IceRole::Controlling,
            };
            for p in &mut self.check_list {
                if p.state == CheckState::InProgress {
                    p.state = CheckState::Waiting;
                    p.retransmit_count = 0;
                    p.retransmit_timer_ms = INITIAL_RTO_MS;
                }
            }
            return HandleDataResult { app_data: None, actions: Vec::new() };
        }

        if !parsed.is_response {
            return HandleDataResult { app_data: None, actions: Vec::new() };
        }

        let idx = match self.check_list.iter().position(|p| p.transaction_id == parsed.transaction_id) {
            Some(i) => i,
            None => return HandleDataResult { app_data: None, actions: Vec::new() },
        };

        self.check_list[idx].state = CheckState::Succeeded;

        // Controlling nomination
        if matches!(self.role, IceRole::Controlling) {
            if self.nominated_pair_confirmed {
                // done
            } else if self.nominated_pair_idx == Some(idx) {
                self.nominated_pair_confirmed = true;
                Self::set_state(self, IceState::Completed);
            } else if self.nominated_pair_idx.is_none() {
                self.nominate_best();
            }
        }

        if matches!(self.state, IceState::Connecting) {
            Self::set_state(self, IceState::Connected);
        }

        HandleDataResult { app_data: None, actions: Vec::new() }
    }

    fn nominate_best(&mut self) {
        let idx = match self.check_list.iter().position(|p| p.state == CheckState::Succeeded) {
            Some(i) => i,
            None => return,
        };
        let pair = &mut self.check_list[idx];
        pair.state = CheckState::InProgress;
        pair.retransmit_count = 0;
        pair.retransmit_timer_ms = INITIAL_RTO_MS;
        pair.nominated = true;
        self.nominated_pair_idx = Some(idx);
    }

    fn check_completion(&mut self) {
        if self.check_list.is_empty() { return; }
        let all_terminal = self.check_list.iter()
            .all(|p| matches!(p.state, CheckState::Succeeded | CheckState::Failed));
        let none_ok = self.check_list.iter().all(|p| p.state != CheckState::Succeeded);
        if all_terminal && none_ok {
            Self::set_state(self, IceState::Failed);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials() {
        let a = IceAgent::new(IceAgentConfig::default());
        assert_eq!(a.local_ufrag().len(), 8);
        assert_eq!(a.local_pwd().len(), 32);
        assert!(matches!(a.state(), IceState::New));
    }

    #[test]
    fn test_add_host_candidate() {
        let mut a = IceAgent::new(IceAgentConfig::default());
        a.add_host_candidate("10.0.0.1", 1234);
        assert_eq!(a.local_candidates().len(), 1);
        let desc = a.local_session_description();
        assert!(desc.candidates[0].contains("10.0.0.1"));
    }

    #[test]
    fn test_start_checks_no_remote() {
        let mut a = IceAgent::new(IceAgentConfig::default());
        a.add_host_candidate("1.1.1.1", 1);
        assert!(a.start_checks().is_err());
        assert!(matches!(a.state(), IceState::Failed));
    }

    #[test]
    fn test_send_data_no_nominated() {
        let a = IceAgent::new(IceAgentConfig::default());
        assert!(a.send_data(b"hello").is_none());
    }

    #[test]
    fn test_stop() {
        let mut a = IceAgent::new(IceAgentConfig::default());
        a.stop();
        assert!(matches!(a.state(), IceState::Closed));
    }

    #[test]
    fn test_tick_after_stop() {
        let mut a = IceAgent::new(IceAgentConfig::default());
        a.stop();
        assert!(a.tick(0).is_empty());
    }

    /// Full loopback: controlling + controlled agents complete handshake.
    #[test]
    fn test_loopback_handshake() {
        let mut ctrl = IceAgent::new(IceAgentConfig { is_controlling: true });
        let mut ctrlled = IceAgent::new(IceAgentConfig { is_controlling: false });

        ctrl.add_host_candidate("192.168.1.100", 5000);
        ctrlled.add_host_candidate("192.168.1.200", 6000);

        let cd = ctrl.local_session_description();
        let cld = ctrlled.local_session_description();
        ctrl.set_remote_session_description(&cld);
        ctrlled.set_remote_session_description(&cd);

        ctrl.start_checks().unwrap();
        ctrlled.start_checks().unwrap();

        let ctrl_addr = ("192.168.1.100", 5000u16);
        let ctrlled_addr = ("192.168.1.200", 6000u16);

        let mut now: u64 = 0;
        for _ in 0..200 {
            now += 50;

            // ctrl sends checks targeting ctrlled; from address is ctrl's local
            for act in ctrl.tick(now) {
                let r = ctrlled.handle_incoming_data(
                    &act.data, ctrl_addr.0, ctrl_addr.1,
                );
                // ctrlled responds back; from address is ctrlled's local
                for resp in r.actions {
                    ctrl.handle_incoming_data(
                        &resp.data, ctrlled_addr.0, ctrlled_addr.1,
                    );
                }
            }

            // ctrlled sends checks targeting ctrl
            for act in ctrlled.tick(now) {
                let r = ctrl.handle_incoming_data(
                    &act.data, ctrlled_addr.0, ctrlled_addr.1,
                );
                for resp in r.actions {
                    ctrlled.handle_incoming_data(
                        &resp.data, ctrl_addr.0, ctrl_addr.1,
                    );
                }
            }

            if matches!(ctrl.state(), IceState::Completed)
                && matches!(ctrlled.state(), IceState::Completed)
            {
                break;
            }
        }

        assert!(matches!(ctrl.state(), IceState::Completed), "ctrl: {:?}", ctrl.state());
        assert!(matches!(ctrlled.state(), IceState::Completed), "ctrlled: {:?}", ctrlled.state());

        // Verify nominated pair exists
        assert!(ctrl.nominated_remote().is_some());
        assert!(ctrlled.nominated_remote().is_some());
    }

    #[test]
    fn test_role_conflict_switch() {
        let mut a = IceAgent::new(IceAgentConfig { is_controlling: true });
        a.add_host_candidate("1.1.1.1", 1);
        a.set_remote_session_description(&IceSessionDescription {
            ice_ufrag: "r".into(),
            ice_pwd: "p".into(),
            is_lite: false,
            candidates: vec!["candidate:1 1 UDP 1 2.2.2.2 2 host".into()],
        });
        a.start_checks().unwrap();
        assert!(matches!(a.role(), IceRole::Controlling));

        let mut parsed = crate::ice::stun_codec::ParsedStunMessage::default();
        parsed.is_stun = true;
        parsed.is_error_response = true;
        parsed.error_code = ICE_ERROR_ROLE_CONFLICT;
        parsed.transaction_id = vec![0; 12];
        a.handle_stun_response(&parsed);
        assert!(matches!(a.role(), IceRole::Controlled));
    }
}
