//! P2P Client facade.
//!
//! Coordinates ICE, STUN, TURN, IDS, and Connector signaling.

use p2p_core::frame::{encode_data_frame, parse_frame, ParsedFrame};
use p2p_core::ice::agent::{HandleDataResult, IceAction, IceAgent, IceAgentConfig};
use p2p_core::ice::check_list::calc_candidate_priority;
use p2p_core::sdp::{generate_sdp_offer, parse_sdp_answer};
use p2p_core::stun::client::{get_external_address, get_turn_relay_address};
use p2p_core::types::{
    ConnectorMessage, IceCandidate, IceCandidateMessage, IceDataMessage, CandidateType, IceState,
    AF_INET,
};
use p2p_io::traits::{HttpTransport, Platform, SignalingTransport, UdpTransport};

use crate::config::Config;
use crate::connector::ConnectorClient;
use crate::ids;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Candidate information returned from gathering.
#[derive(Debug, Clone)]
pub struct CandidateInfo {
    pub candidate_lines: Vec<String>,
    pub local_addresses: Vec<String>,
    pub stun_external_ip: String,
    pub stun_external_port: u16,
    pub turn_relay_ip: String,
    pub turn_relay_port: u16,
}

/// NAT route info from the route service.
#[derive(Debug, Clone)]
struct NatRoute {
    stun_ip: String,
    stun_port: u16,
    turn_ip: String,
    turn_port: u16,
}

// ---------------------------------------------------------------------------
// P2pClient
// ---------------------------------------------------------------------------

/// P2P Client — the main SDK facade.
///
/// Coordinates ICE, STUN, TURN, IDS, and Connector signaling.
pub struct P2pClient {
    config: Option<Config>,
    nat_route: Option<NatRoute>,

    // Cached STUN/TURN results
    stun_external_ip: String,
    stun_external_port: u16,
    turn_relay_ip: String,
    turn_relay_port: u16,
    turn_mapped_ip: String,
    turn_mapped_port: u16,

    // ICE
    ice_agent: Option<IceAgent>,

    // Connector signaling
    connector: Option<ConnectorClient>,

    // Identifier
    identifier: String,
}

impl P2pClient {
    pub fn new() -> Self {
        Self {
            config: None,
            nat_route: None,
            stun_external_ip: String::new(),
            stun_external_port: 0,
            turn_relay_ip: String::new(),
            turn_relay_port: 0,
            turn_mapped_ip: String::new(),
            turn_mapped_port: 0,
            ice_agent: None,
            connector: None,
            identifier: String::new(),
        }
    }

    pub fn init(&mut self, config: Config) {
        self.config = Some(config);
    }

    // ── NAT route resolution ────────────────────────────────────────────────

    /// Fetch NAT route info (STUN/TURN server addresses) from the route service.
    ///
    /// Failures are non-fatal: if STUN/TURN queries fail, empty values are stored
    /// and ICE will proceed with host candidates only.
    pub fn resolve_nat_route(
        &mut self,
        http: &dyn HttpTransport,
        p2p_token: &str,
    ) -> Result<(), String> {
        let config = self.config.as_ref().ok_or("not initialized")?;
        if config.nat_url.is_empty() {
            return Ok(());
        }
        let url = &config.nat_url;
        let headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), p2p_token.into()),
        ];

        let parse_port = |v: Option<&serde_json::Value>| -> u16 {
            v.and_then(|v| {
                v.as_u64().map(|n| n as u16)
                    .or_else(|| v.as_str().and_then(|s| s.parse::<u16>().ok()))
            }).unwrap_or(0)
        };

        let mut stun_ip = String::new();
        let mut stun_port: u16 = 0;
        let mut turn_ip = String::new();
        let mut turn_port: u16 = 0;

        // STUN (type=2) — failure is non-fatal
        let stun_body = serde_json::json!({ "type": 2 });
        if let Ok(body_bytes) = serde_json::to_vec(&stun_body) {
            match http.post(url, &headers, &body_bytes) {
                Ok((status, resp)) if (200..300).contains(&status) => {
                    let json: serde_json::Value = serde_json::from_str(&resp).unwrap_or_default();
                    let data = json.get("data").unwrap_or(&json);
                    stun_ip = data.get("stunIp").and_then(|v| v.as_str()).unwrap_or("").into();
                    stun_port = parse_port(data.get("stunPort"));
                }
                Ok((status, _)) => eprintln!("NAT STUN: HTTP {status}"),
                Err(e) => eprintln!("NAT STUN: {e}"),
            }
        }

        // TURN (type=3) — failure is non-fatal
        let turn_body = serde_json::json!({ "type": 3 });
        if let Ok(body_bytes) = serde_json::to_vec(&turn_body) {
            match http.post(url, &headers, &body_bytes) {
                Ok((status, resp)) if (200..300).contains(&status) => {
                    let json: serde_json::Value = serde_json::from_str(&resp).unwrap_or_default();
                    let data = json.get("data").unwrap_or(&json);
                    turn_ip = data.get("turnIp").and_then(|v| v.as_str()).unwrap_or("").into();
                    turn_port = parse_port(data.get("turnPort"));
                }
                Ok((status, _)) => eprintln!("NAT TURN: HTTP {status}"),
                Err(e) => eprintln!("NAT TURN: {e}"),
            }
        }

        self.nat_route = Some(NatRoute { stun_ip, stun_port, turn_ip, turn_port });
        Ok(())
    }

    // ── Candidate gathering ─────────────────────────────────────────────────

    /// Gather candidate information for display (no ICE agent created).
    /// Caches STUN/TURN results for later use by `setup_ice_and_gather`.
    pub fn gather_candidate_info(
        &mut self,
        udp: &dyn UdpTransport,
        platform: &dyn Platform,
        http: &dyn HttpTransport,
        p2p_token: &str,
    ) -> Result<CandidateInfo, String> {
        // Resolve NAT route if not cached
        if self.nat_route.is_none() {
            self.resolve_nat_route(http, p2p_token)?;
        }

        let local_addrs = platform.get_local_addresses();

        // STUN binding (srflx) — only if not cached
        if self.stun_external_ip.is_empty() {
            if let Some(route) = &self.nat_route {
                let send = &mut |data: &[u8]| {
                    let _ = udp.send_to(data, &route.stun_ip, route.stun_port);
                };
                let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                    udp.recv_from(timeout_ms).ok().map(|(data, _, _)| data)
                };

                if let Ok(result) = get_external_address(
                    send, recv, &route.stun_ip, route.stun_port, p2p_token,
                ) {
                    self.stun_external_ip = result.ip.clone();
                    self.stun_external_port = result.port;
                }
            }
        }

        // TURN allocate (relay) — only if not cached
        if self.turn_relay_ip.is_empty() {
            if let Some(route) = &self.nat_route {
                let send = &mut |data: &[u8]| {
                    let _ = udp.send_to(data, &route.turn_ip, route.turn_port);
                };
                let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                    udp.recv_from(timeout_ms).ok().map(|(data, _, _)| data)
                };

                if let Ok(result) = get_turn_relay_address(
                    send, recv,
                    &route.turn_ip, route.turn_port,
                    p2p_token, AF_INET,
                ) {
                    self.turn_relay_ip = result.relay_ip.clone();
                    self.turn_relay_port = result.relay_port;
                    self.turn_mapped_ip = result.mapped_ip.clone();
                    self.turn_mapped_port = result.mapped_port;
                }
            }
        }

        // Build display lines
        let mut lines = Vec::new();
        for addr in &local_addrs {
            if *addr != "127.0.0.1" && *addr != "::1" {
                lines.push(format!("host    {addr}"));
            }
        }
        if !self.stun_external_ip.is_empty() {
            if let Some(route) = &self.nat_route {
                lines.push(format!(
                    "srflx   {}:{} (via {})",
                    self.stun_external_ip, self.stun_external_port, route.stun_ip
                ));
            }
        }
        if !self.turn_relay_ip.is_empty() {
            if let Some(route) = &self.nat_route {
                lines.push(format!(
                    "relay   {}:{} (via {})",
                    self.turn_relay_ip, self.turn_relay_port, route.turn_ip
                ));
            }
        }
        if lines.is_empty() {
            lines.push("no candidates found".into());
        }

        Ok(CandidateInfo {
            candidate_lines: lines,
            local_addresses: local_addrs,
            stun_external_ip: self.stun_external_ip.clone(),
            stun_external_port: self.stun_external_port,
            turn_relay_ip: self.turn_relay_ip.clone(),
            turn_relay_port: self.turn_relay_port,
        })
    }

    /// Create ICE agent and gather all candidates.
    /// Uses cached STUN/TURN results if available, queries fresh otherwise.
    pub fn setup_ice_and_gather(
        &mut self,
        udp: &dyn UdpTransport,
        platform: &dyn Platform,
        p2p_token: &str,
        is_controlling: bool,
    ) -> Result<(), String> {
        let local_addrs = platform.get_local_addresses();
        let (_, local_port) = udp.local_addr().map_err(|e| e.to_string())?;

        let mut agent = IceAgent::new(IceAgentConfig { is_controlling });

        // Add host candidates
        for addr in &local_addrs {
            agent.add_host_candidate(addr, local_port);
        }

        // STUN external address (srflx) — use cached or query fresh
        if self.nat_route.is_some() {
            if !self.stun_external_ip.is_empty() {
                // Use cached result
                agent.add_local_candidate(IceCandidate {
                    foundation: "srflx1".into(),
                    component_id: 1,
                    transport: "UDP".into(),
                    priority: calc_candidate_priority(CandidateType::Srflx),
                    connection_address: self.stun_external_ip.clone(),
                    port: self.stun_external_port,
                    candidate_type: CandidateType::Srflx,
                    related_address: local_addrs.first().cloned().unwrap_or_default(),
                    related_port: local_port,
                });
            } else {
                // Query fresh
                let route = self.nat_route.as_ref().unwrap();

                let send = &mut |data: &[u8]| {
                    let _ = udp.send_to(data, &route.stun_ip, route.stun_port);
                };
                let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                    udp.recv_from(timeout_ms).ok().map(|(data, _, _)| data)
                };

                if let Ok(result) = get_external_address(
                    send, recv, &route.stun_ip, route.stun_port, p2p_token,
                ) {
                    self.stun_external_ip = result.ip.clone();
                    self.stun_external_port = result.port;
                    agent.add_local_candidate(IceCandidate {
                        foundation: "srflx1".into(),
                        component_id: 1,
                        transport: "UDP".into(),
                        priority: calc_candidate_priority(CandidateType::Srflx),
                        connection_address: result.ip,
                        port: result.port,
                        candidate_type: CandidateType::Srflx,
                        related_address: local_addrs.first().cloned().unwrap_or_default(),
                        related_port: local_port,
                    });
                }
            }

            // TURN relay candidate — use cached or query fresh
            if !self.turn_relay_ip.is_empty() {
                agent.add_local_candidate(IceCandidate {
                    foundation: "relay1".into(),
                    component_id: 1,
                    transport: "UDP".into(),
                    priority: calc_candidate_priority(CandidateType::Relay),
                    connection_address: self.turn_relay_ip.clone(),
                    port: self.turn_relay_port,
                    candidate_type: CandidateType::Relay,
                    related_address: self.stun_external_ip.clone(),
                    related_port: self.stun_external_port,
                });
            } else {
                let route = self.nat_route.as_ref().unwrap();

                let send = &mut |data: &[u8]| {
                    let _ = udp.send_to(data, &route.turn_ip, route.turn_port);
                };
                let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                    udp.recv_from(timeout_ms).ok().map(|(data, _, _)| data)
                };

                if let Ok(turn_result) = get_turn_relay_address(
                    send, recv,
                    &route.turn_ip, route.turn_port,
                    p2p_token, AF_INET,
                ) {
                    self.turn_relay_ip = turn_result.relay_ip.clone();
                    self.turn_relay_port = turn_result.relay_port;
                    self.turn_mapped_ip = turn_result.mapped_ip.clone();
                    self.turn_mapped_port = turn_result.mapped_port;
                    agent.add_local_candidate(IceCandidate {
                        foundation: "relay1".into(),
                        component_id: 1,
                        transport: "UDP".into(),
                        priority: calc_candidate_priority(CandidateType::Relay),
                        connection_address: turn_result.relay_ip,
                        port: turn_result.relay_port,
                        candidate_type: CandidateType::Relay,
                        related_address: self.stun_external_ip.clone(),
                        related_port: self.stun_external_port,
                    });
                }
            }
        }

        self.ice_agent = Some(agent);
        Ok(())
    }

    /// Gather ICE candidates: STUN external address, TURN relay, and local addresses.
    /// This creates an ICE agent. For display-only, use `gather_candidate_info` instead.
    pub fn gather_candidates(
        &mut self,
        udp: &dyn UdpTransport,
        platform: &dyn Platform,
        p2p_token: &str,
        is_controlling: bool,
    ) -> Result<CandidateInfo, String> {
        let local_addrs = platform.get_local_addresses();
        let (_, local_port) = udp.local_addr().map_err(|e| e.to_string())?;

        let mut agent = IceAgent::new(IceAgentConfig { is_controlling });

        // Add host candidates
        for addr in &local_addrs {
            agent.add_host_candidate(addr, local_port);
        }

        // STUN external address (srflx candidate)
        if let Some(route) = &self.nat_route {
            let send = &mut |data: &[u8]| {
                let _ = udp.send_to(data, &route.stun_ip, route.stun_port);
            };
            let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                udp.recv_from(timeout_ms).ok().map(|(data, _, _)| data)
            };

            if let Ok(result) = get_external_address(send, recv, &route.stun_ip, route.stun_port, p2p_token) {
                self.stun_external_ip = result.ip.clone();
                self.stun_external_port = result.port;
                agent.add_local_candidate(p2p_core::types::IceCandidate {
                    foundation: "srflx1".into(),
                    component_id: 1,
                    transport: "UDP".into(),
                    priority: p2p_core::ice::check_list::calc_candidate_priority(CandidateType::Srflx),
                    connection_address: result.ip,
                    port: result.port,
                    candidate_type: CandidateType::Srflx,
                    related_address: local_addrs.first().cloned().unwrap_or_default(),
                    related_port: local_port,
                });
            }

            // TURN relay candidate
            if let Ok(turn_result) = get_turn_relay_address(
                send, recv,
                &route.turn_ip, route.turn_port,
                p2p_token, p2p_core::types::AF_INET,
            ) {
                self.turn_relay_ip = turn_result.relay_ip.clone();
                self.turn_relay_port = turn_result.relay_port;
                self.turn_mapped_ip = turn_result.mapped_ip.clone();
                self.turn_mapped_port = turn_result.mapped_port;
                agent.add_local_candidate(p2p_core::types::IceCandidate {
                    foundation: "relay1".into(),
                    component_id: 1,
                    transport: "UDP".into(),
                    priority: p2p_core::ice::check_list::calc_candidate_priority(CandidateType::Relay),
                    connection_address: turn_result.relay_ip,
                    port: turn_result.relay_port,
                    candidate_type: CandidateType::Relay,
                    related_address: self.stun_external_ip.clone(),
                    related_port: self.stun_external_port,
                });
            }
        }

        let desc = agent.local_session_description();
        self.ice_agent = Some(agent);

        Ok(CandidateInfo {
            candidate_lines: desc.candidates,
            local_addresses: local_addrs,
            stun_external_ip: self.stun_external_ip.clone(),
            stun_external_port: self.stun_external_port,
            turn_relay_ip: self.turn_relay_ip.clone(),
            turn_relay_port: self.turn_relay_port,
        })
    }

    // ── Connector signaling ─────────────────────────────────────────────────

    /// Connect to the Connector signaling server and register.
    pub fn connect_connector(
        &mut self,
        ws: Box<dyn SignalingTransport>,
        url: &str,
        identifier: &str,
        auth_token: &str,
    ) -> Result<(), String> {
        let mut connector = ConnectorClient::new(ws);
        connector.connect(url, identifier, auth_token).map_err(|e| e.to_string())?;
        self.connector = Some(connector);
        self.identifier = identifier.into();
        Ok(())
    }

    /// Poll the Connector for incoming messages.
    pub fn poll_connector(&mut self) -> Vec<ConnectorMessage> {
        if let Some(connector) = &mut self.connector {
            connector.poll()
        } else {
            Vec::new()
        }
    }

    /// Send a message to a peer via the Connector.
    pub fn send_connector_message(
        &self,
        target_id: &str,
        data: &serde_json::Value,
    ) -> Result<(), String> {
        self.connector
            .as_ref()
            .ok_or("not connected to connector")?
            .send_to(target_id, data)
            .map_err(|e| e.to_string())
    }

    // ── ICE signaling via Connector ─────────────────────────────────────────

    /// Initiate ICE by sending an offer via Connector signaling.
    pub fn initiate_ice(&mut self, peer_id: &str) -> Result<(), String> {
        let agent = self.ice_agent.as_ref().ok_or("no ICE agent")?;
        let desc = agent.local_session_description();
        let msg = IceDataMessage {
            action: "iceOffer".into(),
            ice_ufrag: desc.ice_ufrag,
            ice_pwd: desc.ice_pwd,
            is_lite: desc.is_lite,
            candidates: desc.candidates,
        };
        let data = serde_json::to_value(&msg).map_err(|e| e.to_string())?;
        self.send_connector_message(peer_id, &data)
    }

    /// Handle a signaling message (offer / answer / candidate).
    pub fn handle_signaling_message(
        &mut self,
        msg: &ConnectorMessage,
    ) -> Result<Vec<IceAction>, String> {
        let data = &msg.data;
        let action = data.get("action").and_then(|v| v.as_str()).unwrap_or("");

        match action {
            "iceOffer" | "iceAnswer" => {
                let ice_msg: IceDataMessage =
                    serde_json::from_value(data.clone()).map_err(|e| e.to_string())?;
                let desc = p2p_core::types::IceSessionDescription {
                    ice_ufrag: ice_msg.ice_ufrag,
                    ice_pwd: ice_msg.ice_pwd,
                    is_lite: ice_msg.is_lite,
                    candidates: ice_msg.candidates,
                };
                let agent = self.ice_agent.as_mut().ok_or("no ICE agent")?;
                agent.set_remote_session_description(&desc);
                if action == "iceOffer" {
                    agent.start_checks()?;
                }
                Ok(Vec::new())
            }
            "iceCandidate" => {
                let cand_msg: IceCandidateMessage =
                    serde_json::from_value(data.clone()).map_err(|e| e.to_string())?;
                let agent = self.ice_agent.as_mut().ok_or("no ICE agent")?;
                agent.add_remote_candidate(&cand_msg.candidate);
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    // ── SDP-based connection ────────────────────────────────────────────────

    /// Connect via SDP offer/answer over HTTP.
    ///
    /// `peer_addr` is the signaling URL from IDS (e.g. "81.71.29.250:34848").
    /// Posts raw SDP to `http://{peer_addr}/api/ice/offer`.
    pub fn connect_via_sdp(
        &mut self,
        http: &dyn HttpTransport,
        odid: &str,
        peer_addr: &str,
        default_ip: &str,
        default_port: u16,
    ) -> Result<(), String> {
        let agent = self.ice_agent.as_mut().ok_or("no ICE agent")?;
        let desc = agent.local_session_description();
        let sdp_offer = generate_sdp_offer(
            odid,
            &desc.ice_ufrag,
            &desc.ice_pwd,
            &desc.candidates,
            default_ip,
            default_port,
        );

        let sdp_answer = ids::send_ice_offer(http, peer_addr, &sdp_offer)?;
        let answer = parse_sdp_answer(&sdp_answer);

        let remote_desc = p2p_core::types::IceSessionDescription {
            ice_ufrag: answer.ufrag,
            ice_pwd: answer.pwd,
            is_lite: answer.is_lite,
            candidates: answer.candidates,
        };
        agent.set_remote_session_description(&remote_desc);
        agent.start_checks()?;

        Ok(())
    }

    // ── ICE data flow ──────────────────────────────────────────────────────

    /// Drive the ICE check cycle. Call periodically (~50 ms).
    pub fn tick(&mut self, now_ms: u64) -> Vec<IceAction> {
        self.ice_agent.as_mut().map(|a| a.tick(now_ms)).unwrap_or_default()
    }

    /// Process incoming UDP data through the ICE agent.
    pub fn handle_incoming_udp(
        &mut self,
        data: &[u8],
        from_ip: &str,
        from_port: u16,
    ) -> HandleDataResult {
        self.ice_agent
            .as_mut()
            .map(|a| a.handle_incoming_data(data, from_ip, from_port))
            .unwrap_or(HandleDataResult { app_data: None, actions: Vec::new() })
    }

    /// Send application data through the ICE nominated pair.
    pub fn send_data(&self, data: &[u8]) -> Option<IceAction> {
        self.ice_agent.as_ref().and_then(|a| a.send_data(data))
    }

    /// Send a text message as a P2P data frame.
    pub fn send_text(&self, text: &str) -> Option<IceAction> {
        let frame = encode_data_frame(text);
        self.send_data(&frame)
    }

    /// Parse received data as a P2P frame.
    pub fn parse_received(data: &[u8]) -> Option<ParsedFrame> {
        parse_frame(data)
    }

    // ── ICE state ───────────────────────────────────────────────────────────

    pub fn ice_state(&self) -> Option<IceState> {
        self.ice_agent.as_ref().map(|a| a.state())
    }

    pub fn is_ice_completed(&self) -> bool {
        self.ice_state() == Some(IceState::Completed)
    }

    // ── IDS operations ──────────────────────────────────────────────────────

    /// Register this device with the IDS service.
    ///
    /// Matches ArkTS: `registerIds(userId, odid, pushToken)` with type fixed to "app".
    pub fn register_ids(
        &self,
        http: &dyn HttpTransport,
        user_id: &str,
        odid: &str,
        push_token: &str,
    ) -> Result<(), String> {
        let config = self.config.as_ref().ok_or("not initialized")?;
        ids::register_ids(http, &config.ids_url, &config.app_id, user_id, "app", odid, push_token)
    }

    /// Query the IDS service for a user's records.
    pub fn query_ids(
        &self,
        http: &dyn HttpTransport,
        user_id: &str,
    ) -> Result<ids::IdsRecord, String> {
        let config = self.config.as_ref().ok_or("not initialized")?;
        ids::query_ids(http, &config.ids_url, &config.app_id, user_id)
    }

    // ── Teardown ────────────────────────────────────────────────────────────

    pub fn disconnect_connector(&mut self) {
        if let Some(connector) = &mut self.connector {
            connector.disconnect();
        }
    }

    pub fn stop_ice(&mut self) {
        if let Some(agent) = &mut self.ice_agent {
            agent.stop();
        }
    }
}

impl Default for P2pClient {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_client() {
        let client = P2pClient::new();
        assert!(client.ice_state().is_none());
    }

    #[test]
    fn test_init_config() {
        let mut client = P2pClient::new();
        client.init(Config::default());
        assert!(client.config.is_some());
    }

    #[test]
    fn test_parse_received_frame() {
        let frame = encode_data_frame("hello");
        let parsed = P2pClient::parse_received(&frame).unwrap();
        assert_eq!(parsed.frame_type, p2p_core::types::TYPE_DATA);
        assert_eq!(String::from_utf8(parsed.payload).unwrap(), "hello");
    }
}
