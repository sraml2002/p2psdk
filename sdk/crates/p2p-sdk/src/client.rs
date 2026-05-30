//! P2P Client facade.
//!
//! Coordinates ICE, STUN, TURN, IDS, and Connector signaling.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use std::collections::VecDeque;

use p2p_core::frame::{encode_data_frame, encode_data_frame_with_seq, encode_heartbeat_reply, parse_frame, ParsedFrame};
use p2p_core::ice::agent::{HandleDataResult, IceAction, IceAgent, IceAgentConfig};
use p2p_core::ice::check_list::calc_candidate_priority;
use p2p_core::sdp::{generate_sdp_offer, parse_sdp_answer};
use p2p_core::stun::client::{get_external_address, get_turn_relay_address};
use p2p_core::types::{
    CandidateType, ConnectorMessage, IceCandidate, IceCandidateMessage, IceDataMessage, IceState,
    AF_INET,
};
use p2p_io::traits::{HttpTransport, IoProvider, Platform, SignalingTransport, UdpTransport};
use p2p_tokio::SyncIoProvider;

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
// ClientInner — shared mutable state for connect threads
// ---------------------------------------------------------------------------

struct ClientInner {
    io: Option<Arc<dyn IoProvider>>,

    // NAT / STUN / TURN cache
    nat_route: Option<NatRoute>,
    stun_external_ip: String,
    stun_external_port: u16,
    turn_relay_ip: String,
    turn_relay_port: u16,
    turn_mapped_ip: String,
    turn_mapped_port: u16,

    // ICE runtime
    ice_agent: Option<IceAgent>,
    ice_udp: Option<Arc<dyn UdpTransport>>,
    tick_handle: Option<JoinHandle<()>>,
    recv_handle: Option<JoinHandle<()>>,
    ice_stop: Arc<AtomicBool>,

    // Callbacks
    on_state_change: Option<Box<dyn Fn(IceState) + Send>>,
    on_data: Option<Box<dyn Fn(Vec<u8>) + Send>>,
    on_log: Option<Box<dyn Fn(&str) + Send>>,

    // Heartbeat
    heartbeat_interval_secs: u32,
    last_recv_instant: Option<Instant>,

    // seqId queue for automatic request-response correlation
    pending_seq_ids: Mutex<VecDeque<u64>>,
}

impl ClientInner {
    fn new() -> Self {
        Self {
            io: None,
            nat_route: None,
            stun_external_ip: String::new(),
            stun_external_port: 0,
            turn_relay_ip: String::new(),
            turn_relay_port: 0,
            turn_mapped_ip: String::new(),
            turn_mapped_port: 0,
            ice_agent: None,
            ice_udp: None,
            tick_handle: None,
            recv_handle: None,
            ice_stop: Arc::new(AtomicBool::new(false)),
            on_state_change: None,
            on_data: None,
            on_log: None,
            heartbeat_interval_secs: 0,
            last_recv_instant: None,
            pending_seq_ids: Mutex::new(VecDeque::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// P2pClient
// ---------------------------------------------------------------------------

/// P2P Client — the main SDK facade.
///
/// Coordinates ICE, STUN, TURN, IDS, and Connector signaling.
pub struct P2pClient {
    config: Option<Config>,
    inner: Arc<Mutex<ClientInner>>,

    // Connector signaling
    connector: Option<ConnectorClient>,

    // Identifier
    identifier: String,
}

impl P2pClient {
    pub fn new() -> Self {
        Self {
            config: None,
            inner: Arc::new(Mutex::new(ClientInner::new())),
            connector: None,
            identifier: String::new(),
        }
    }

    pub fn init(&mut self, config: Config) {
        self.config = Some(config);
        self.inner.lock().unwrap().io = Some(Arc::new(SyncIoProvider));
    }

    // ── Callback registration ─────────────────────────────────────────────

    /// Register a callback for ICE state changes.
    pub fn on_state_change(&self, cb: Box<dyn Fn(IceState) + Send>) {
        self.inner.lock().unwrap().on_state_change = Some(cb);
    }

    /// Register a callback for received data (data-frame payload only;
    /// heartbeat frames are handled internally and not reported).
    pub fn on_data(&self, cb: Box<dyn Fn(Vec<u8>) + Send>) {
        self.inner.lock().unwrap().on_data = Some(cb);
    }

    /// Register a diagnostic log callback.
    pub fn on_log(&self, cb: Box<dyn Fn(&str) + Send>) {
        self.inner.lock().unwrap().on_log = Some(cb);
    }

    // ── NAT route resolution ──────────────────────────────────────────────

    /// Fetch NAT route info (STUN/TURN server addresses) from the route service.
    pub fn resolve_nat_route(
        &self,
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
                v.as_u64()
                    .map(|n| n as u16)
                    .or_else(|| v.as_str().and_then(|s| s.parse::<u16>().ok()))
            })
            .unwrap_or(0)
        };

        let mut stun_ip = String::new();
        let mut stun_port: u16 = 0;
        let mut turn_ip = String::new();
        let mut turn_port: u16 = 0;

        let stun_body = serde_json::json!({ "type": 2 });
        if let Ok(body_bytes) = serde_json::to_vec(&stun_body) {
            match http.post(url, &headers, &body_bytes) {
                Ok((status, resp)) if (200..300).contains(&status) => {
                    let json: serde_json::Value =
                        serde_json::from_str(&resp).unwrap_or_default();
                    let data = json.get("data").unwrap_or(&json);
                    stun_ip = data
                        .get("stunIp")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into();
                    stun_port = parse_port(data.get("stunPort"));
                }
                Ok((status, _)) => eprintln!("NAT STUN: HTTP {status}"),
                Err(e) => eprintln!("NAT STUN: {e}"),
            }
        }

        let turn_body = serde_json::json!({ "type": 3 });
        if let Ok(body_bytes) = serde_json::to_vec(&turn_body) {
            match http.post(url, &headers, &body_bytes) {
                Ok((status, resp)) if (200..300).contains(&status) => {
                    let json: serde_json::Value =
                        serde_json::from_str(&resp).unwrap_or_default();
                    let data = json.get("data").unwrap_or(&json);
                    turn_ip = data
                        .get("turnIp")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into();
                    turn_port = parse_port(data.get("turnPort"));
                }
                Ok((status, _)) => eprintln!("NAT TURN: HTTP {status}"),
                Err(e) => eprintln!("NAT TURN: {e}"),
            }
        }

        self.inner.lock().unwrap().nat_route = Some(NatRoute {
            stun_ip,
            stun_port,
            turn_ip,
            turn_port,
        });
        Ok(())
    }

    // ── Candidate gathering ───────────────────────────────────────────────

    /// Gather candidate information for display (no ICE agent created).
    pub fn gather_candidate_info(
        &self,
        udp: &dyn UdpTransport,
        platform: &dyn Platform,
        http: &dyn HttpTransport,
        p2p_token: &str,
    ) -> Result<CandidateInfo, String> {
        {
            let inner = self.inner.lock().unwrap();
            if inner.nat_route.is_none() {
                drop(inner);
                self.resolve_nat_route(http, p2p_token)?;
            }
        }

        let local_addrs = platform.get_local_addresses();

        {
            let mut inner = self.inner.lock().unwrap();
            if inner.stun_external_ip.is_empty() {
                if let Some(route) = &inner.nat_route {
                    let send = &mut |data: &[u8]| {
                        let _ = udp.send_to(data, &route.stun_ip, route.stun_port);
                    };
                    let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                        udp.recv_from(timeout_ms).ok().map(|(data, _, _)| data)
                    };

                    if let Ok(result) =
                        get_external_address(send, recv, &route.stun_ip, route.stun_port, p2p_token)
                    {
                        inner.stun_external_ip = result.ip.clone();
                        inner.stun_external_port = result.port;
                    }
                }
            }

            if inner.turn_relay_ip.is_empty() {
                if let Some(route) = &inner.nat_route {
                    let send = &mut |data: &[u8]| {
                        let _ = udp.send_to(data, &route.turn_ip, route.turn_port);
                    };
                    let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                        udp.recv_from(timeout_ms).ok().map(|(data, _, _)| data)
                    };

                    if let Ok(result) = get_turn_relay_address(
                        send,
                        recv,
                        &route.turn_ip,
                        route.turn_port,
                        p2p_token,
                        AF_INET,
                    ) {
                        inner.turn_relay_ip = result.relay_ip.clone();
                        inner.turn_relay_port = result.relay_port;
                        inner.turn_mapped_ip = result.mapped_ip.clone();
                        inner.turn_mapped_port = result.mapped_port;
                    }
                }
            }
        }

        let inner = self.inner.lock().unwrap();
        let mut lines = Vec::new();
        for addr in &local_addrs {
            if *addr != "127.0.0.1" && *addr != "::1" {
                lines.push(format!("host    {addr}"));
            }
        }
        if !inner.stun_external_ip.is_empty() {
            if let Some(route) = &inner.nat_route {
                lines.push(format!(
                    "srflx   {}:{} (via {})",
                    inner.stun_external_ip, inner.stun_external_port, route.stun_ip
                ));
            }
        }
        if !inner.turn_relay_ip.is_empty() {
            if let Some(route) = &inner.nat_route {
                lines.push(format!(
                    "relay   {}:{} (via {})",
                    inner.turn_relay_ip, inner.turn_relay_port, route.turn_ip
                ));
            }
        }
        if lines.is_empty() {
            lines.push("no candidates found".into());
        }

        Ok(CandidateInfo {
            candidate_lines: lines,
            local_addresses: local_addrs,
            stun_external_ip: inner.stun_external_ip.clone(),
            stun_external_port: inner.stun_external_port,
            turn_relay_ip: inner.turn_relay_ip.clone(),
            turn_relay_port: inner.turn_relay_port,
        })
    }

    /// Create ICE agent and gather all candidates (uses cached STUN/TURN).
    pub fn setup_ice_and_gather(
        &self,
        udp: &dyn UdpTransport,
        platform: &dyn Platform,
        p2p_token: &str,
        is_controlling: bool,
    ) -> Result<(), String> {
        let local_addrs = platform.get_local_addresses();
        let (_, local_port) = udp.local_addr().map_err(|e| e.to_string())?;

        let mut agent = IceAgent::new(IceAgentConfig { is_controlling });

        for addr in &local_addrs {
            agent.add_host_candidate(addr, local_port);
        }

        let mut inner = self.inner.lock().unwrap();

        if inner.nat_route.is_some() {
            if !inner.stun_external_ip.is_empty() {
                agent.add_local_candidate(IceCandidate {
                    foundation: "srflx1".into(),
                    component_id: 1,
                    transport: "UDP".into(),
                    priority: calc_candidate_priority(CandidateType::Srflx),
                    connection_address: inner.stun_external_ip.clone(),
                    port: inner.stun_external_port,
                    candidate_type: CandidateType::Srflx,
                    related_address: local_addrs.first().cloned().unwrap_or_default(),
                    related_port: local_port,
                });
            } else {
                let route = inner.nat_route.as_ref().unwrap();
                let send = &mut |data: &[u8]| {
                    let _ = udp.send_to(data, &route.stun_ip, route.stun_port);
                };
                let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                    udp.recv_from(timeout_ms).ok().map(|(data, _, _)| data)
                };
                if let Ok(result) =
                    get_external_address(send, recv, &route.stun_ip, route.stun_port, p2p_token)
                {
                    inner.stun_external_ip = result.ip.clone();
                    inner.stun_external_port = result.port;
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

            if !inner.turn_relay_ip.is_empty() {
                agent.add_local_candidate(IceCandidate {
                    foundation: "relay1".into(),
                    component_id: 1,
                    transport: "UDP".into(),
                    priority: calc_candidate_priority(CandidateType::Relay),
                    connection_address: inner.turn_relay_ip.clone(),
                    port: inner.turn_relay_port,
                    candidate_type: CandidateType::Relay,
                    related_address: inner.stun_external_ip.clone(),
                    related_port: inner.stun_external_port,
                });
            } else {
                let route = inner.nat_route.as_ref().unwrap();
                let send = &mut |data: &[u8]| {
                    let _ = udp.send_to(data, &route.turn_ip, route.turn_port);
                };
                let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                    udp.recv_from(timeout_ms).ok().map(|(data, _, _)| data)
                };
                if let Ok(turn_result) = get_turn_relay_address(
                    send,
                    recv,
                    &route.turn_ip,
                    route.turn_port,
                    p2p_token,
                    AF_INET,
                ) {
                    inner.turn_relay_ip = turn_result.relay_ip.clone();
                    inner.turn_relay_port = turn_result.relay_port;
                    inner.turn_mapped_ip = turn_result.mapped_ip.clone();
                    inner.turn_mapped_port = turn_result.mapped_port;
                    agent.add_local_candidate(IceCandidate {
                        foundation: "relay1".into(),
                        component_id: 1,
                        transport: "UDP".into(),
                        priority: calc_candidate_priority(CandidateType::Relay),
                        connection_address: turn_result.relay_ip,
                        port: turn_result.relay_port,
                        candidate_type: CandidateType::Relay,
                        related_address: inner.stun_external_ip.clone(),
                        related_port: inner.stun_external_port,
                    });
                }
            }
        }

        inner.ice_agent = Some(agent);
        Ok(())
    }

    /// Gather ICE candidates and create ICE agent (legacy, no caching).
    pub fn gather_candidates(
        &self,
        udp: &dyn UdpTransport,
        platform: &dyn Platform,
        p2p_token: &str,
        is_controlling: bool,
    ) -> Result<CandidateInfo, String> {
        let local_addrs = platform.get_local_addresses();
        let (_, local_port) = udp.local_addr().map_err(|e| e.to_string())?;

        let mut agent = IceAgent::new(IceAgentConfig { is_controlling });

        for addr in &local_addrs {
            agent.add_host_candidate(addr, local_port);
        }

        let inner = self.inner.lock().unwrap();

        // Clone route fields to avoid borrow conflict
        let route_clone = inner.nat_route.clone();
        drop(inner);

        if let Some(route) = &route_clone {
            let send = &mut |data: &[u8]| {
                let _ = udp.send_to(data, &route.stun_ip, route.stun_port);
            };
            let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                udp.recv_from(timeout_ms).ok().map(|(data, _, _)| data)
            };

            if let Ok(result) =
                get_external_address(send, recv, &route.stun_ip, route.stun_port, p2p_token)
            {
                let mut inner = self.inner.lock().unwrap();
                inner.stun_external_ip = result.ip.clone();
                inner.stun_external_port = result.port;
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

            if let Ok(turn_result) = get_turn_relay_address(
                send,
                recv,
                &route.turn_ip,
                route.turn_port,
                p2p_token,
                AF_INET,
            ) {
                let mut inner = self.inner.lock().unwrap();
                inner.turn_relay_ip = turn_result.relay_ip.clone();
                inner.turn_relay_port = turn_result.relay_port;
                inner.turn_mapped_ip = turn_result.mapped_ip.clone();
                inner.turn_mapped_port = turn_result.mapped_port;
                agent.add_local_candidate(IceCandidate {
                    foundation: "relay1".into(),
                    component_id: 1,
                    transport: "UDP".into(),
                    priority: calc_candidate_priority(CandidateType::Relay),
                    connection_address: turn_result.relay_ip,
                    port: turn_result.relay_port,
                    candidate_type: CandidateType::Relay,
                    related_address: inner.stun_external_ip.clone(),
                    related_port: inner.stun_external_port,
                });
            }
        }

        let desc = agent.local_session_description();
        {
            let mut inner = self.inner.lock().unwrap();
            inner.ice_agent = Some(agent);
        }

        let inner = self.inner.lock().unwrap();
        Ok(CandidateInfo {
            candidate_lines: desc.candidates,
            local_addresses: local_addrs,
            stun_external_ip: inner.stun_external_ip.clone(),
            stun_external_port: inner.stun_external_port,
            turn_relay_ip: inner.turn_relay_ip.clone(),
            turn_relay_port: inner.turn_relay_port,
        })
    }

    // ── High-level connect ────────────────────────────────────────────────

    /// One-stop P2P connection: NAT route → gather candidates → SDP negotiate
    /// → start ICE tick/recv threads with heartbeat.
    ///
    /// Spawns a background thread. Results are delivered via `on_state_change`
    /// and `on_data` callbacks. Requires `init` to have been called.
    pub fn connect(
        &self,
        peer_addr: &str,
        odid: &str,
        heartbeat_interval_secs: u32,
    ) -> Result<(), String> {
        let config = self.config.as_ref().ok_or("not initialized")?;
        let io = {
            let inner = self.inner.lock().unwrap();
            inner
                .io
                .clone()
                .ok_or("not initialized — call init first")?
        };

        let p2p_token = crate::token::generate_token_with_url(&config.nat_token_url)
            .map_err(|e| format!("failed to generate P2P token: {e}"))?;

        // Store heartbeat interval
        self.inner.lock().unwrap().heartbeat_interval_secs = heartbeat_interval_secs;

        let inner = self.inner.clone();
        let nat_url = config.nat_url.clone();
        let p2p_token = p2p_token;
        let peer_addr = peer_addr.to_string();
        let odid = odid.to_string();

        thread::spawn(move || {
            if let Err(e) = connect_background(&inner, &io, &nat_url, &p2p_token, &peer_addr, &odid)
            {
                eprintln!("connect failed: {e}");
                fire_state_change(&inner, IceState::Failed);
            }
        });

        Ok(())
    }

    // ── Connector signaling ───────────────────────────────────────────────

    pub fn connect_connector(
        &mut self,
        ws: Box<dyn SignalingTransport>,
        url: &str,
        identifier: &str,
        auth_token: &str,
    ) -> Result<(), String> {
        let mut connector = ConnectorClient::new(ws);
        connector
            .connect(url, identifier, auth_token)
            .map_err(|e| e.to_string())?;
        self.connector = Some(connector);
        self.identifier = identifier.into();
        Ok(())
    }

    pub fn poll_connector(&mut self) -> Vec<ConnectorMessage> {
        if let Some(connector) = &mut self.connector {
            connector.poll()
        } else {
            Vec::new()
        }
    }

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

    // ── ICE signaling via Connector ───────────────────────────────────────

    pub fn initiate_ice(&self, peer_id: &str) -> Result<(), String> {
        let inner = self.inner.lock().unwrap();
        let agent = inner.ice_agent.as_ref().ok_or("no ICE agent")?;
        let desc = agent.local_session_description();
        let msg = IceDataMessage {
            action: "iceOffer".into(),
            ice_ufrag: desc.ice_ufrag,
            ice_pwd: desc.ice_pwd,
            is_lite: desc.is_lite,
            candidates: desc.candidates,
        };
        let data = serde_json::to_value(&msg).map_err(|e| e.to_string())?;
        drop(inner);
        self.send_connector_message(peer_id, &data)
    }

    pub fn handle_signaling_message(
        &self,
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
                let mut inner = self.inner.lock().unwrap();
                let agent = inner.ice_agent.as_mut().ok_or("no ICE agent")?;
                agent.set_remote_session_description(&desc);
                if action == "iceOffer" {
                    agent.start_checks()?;
                }
                Ok(Vec::new())
            }
            "iceCandidate" => {
                let cand_msg: IceCandidateMessage =
                    serde_json::from_value(data.clone()).map_err(|e| e.to_string())?;
                let mut inner = self.inner.lock().unwrap();
                let agent = inner.ice_agent.as_mut().ok_or("no ICE agent")?;
                agent.add_remote_candidate(&cand_msg.candidate);
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    // ── SDP-based connection (low-level) ──────────────────────────────────

    pub fn connect_via_sdp(
        &self,
        http: &dyn HttpTransport,
        odid: &str,
        peer_addr: &str,
        default_ip: &str,
        default_port: u16,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        let agent = inner.ice_agent.as_mut().ok_or("no ICE agent")?;
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

    // ── ICE data flow (low-level) ─────────────────────────────────────────

    /// Drive the ICE check cycle. Call periodically (~50 ms).
    pub fn tick(&self, now_ms: u64) -> Vec<IceAction> {
        self.inner
            .lock()
            .unwrap()
            .ice_agent
            .as_mut()
            .map(|a| a.tick(now_ms))
            .unwrap_or_default()
    }

    /// Process incoming UDP data through the ICE agent.
    pub fn handle_incoming_udp(
        &self,
        data: &[u8],
        from_ip: &str,
        from_port: u16,
    ) -> HandleDataResult {
        self.inner
            .lock()
            .unwrap()
            .ice_agent
            .as_mut()
            .map(|a| a.handle_incoming_data(data, from_ip, from_port))
            .unwrap_or(HandleDataResult {
                app_data: None,
                actions: Vec::new(),
            })
    }

    /// Send application data through the ICE nominated pair (auto-send).
    pub fn send_data(&self, data: &[u8]) -> Result<(), String> {
        let inner = self.inner.lock().unwrap();
        let agent = inner.ice_agent.as_ref().ok_or("no ICE agent")?;
        let udp = inner.ice_udp.as_ref().ok_or("no UDP socket")?;
        let act = agent.send_data(data).ok_or("no nominated pair")?;
        udp.send_to(&act.data, &act.target_ip, act.target_port)
            .map_err(|e| e.to_string())
    }

    /// Send a text message as a P2P data frame (auto-send).
    /// Automatically picks up a pending seqId from the queue for response correlation.
    pub fn send_text(&self, text: &str) -> Result<(), String> {
        let seq_id = self.inner.lock().unwrap()
            .pending_seq_ids.lock().unwrap().pop_front().unwrap_or(0);
        sdk_log(&self.inner, &format!(
            "[P2PSDK] send → seqId={}, text=\"{}\" ({} bytes)",
            seq_id, text, text.len()));
        let frame = if seq_id > 0 {
            encode_data_frame_with_seq(text, seq_id)
        } else {
            encode_data_frame(text)
        };
        self.send_data(&frame)
    }

    /// Parse received data as a P2P frame.
    pub fn parse_received(data: &[u8]) -> Option<ParsedFrame> {
        parse_frame(data)
    }

    // ── ICE state ─────────────────────────────────────────────────────────

    pub fn ice_state(&self) -> Option<IceState> {
        self.inner
            .lock()
            .unwrap()
            .ice_agent
            .as_ref()
            .map(|a| a.state())
    }

    pub fn is_ice_completed(&self) -> bool {
        self.ice_state() == Some(IceState::Completed)
    }

    // ── IDS operations ────────────────────────────────────────────────────

    pub fn register_ids(
        &self,
        http: &dyn HttpTransport,
        app_id: &str,
        user_id: &str,
        odid: &str,
        push_token: &str,
    ) -> Result<(), String> {
        let config = self.config.as_ref().ok_or("not initialized")?;
        ids::register_ids(
            http,
            &config.ids_url,
            app_id,
            user_id,
            "app",
            odid,
            push_token,
        )
    }

    pub fn query_ids(
        &self,
        http: &dyn HttpTransport,
        app_id: &str,
        user_id: &str,
    ) -> Result<ids::IdsRecord, String> {
        let config = self.config.as_ref().ok_or("not initialized")?;
        ids::query_ids(http, &config.ids_url, app_id, user_id)
    }

    // ── Teardown ──────────────────────────────────────────────────────────

    /// Stop ICE threads, close UDP, release all resources.
    pub fn close(&self) -> Result<(), String> {
        stop_ice_threads(&self.inner);
        let mut inner = self.inner.lock().unwrap();
        if let Some(agent) = &mut inner.ice_agent {
            agent.stop();
        }
        inner.ice_agent = None;
        inner.ice_udp = None;
        inner.heartbeat_interval_secs = 0;
        Ok(())
    }

    pub fn disconnect_connector(&mut self) {
        if let Some(connector) = &mut self.connector {
            connector.disconnect();
        }
    }

    pub fn stop_ice(&self) {
        stop_ice_threads(&self.inner);
        let mut inner = self.inner.lock().unwrap();
        if let Some(agent) = &mut inner.ice_agent {
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
// Internal: background connect
// ---------------------------------------------------------------------------

fn connect_background(
    inner: &Arc<Mutex<ClientInner>>,
    io: &Arc<dyn IoProvider>,
    nat_url: &str,
    p2p_token: &str,
    peer_addr: &str,
    odid: &str,
) -> Result<(), String> {
    // Step 1: Resolve NAT route
    let http = io.create_http();
    if !nat_url.is_empty() {
        let headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), p2p_token.into()),
        ];
        let parse_port = |v: Option<&serde_json::Value>| -> u16 {
            v.and_then(|v| {
                v.as_u64()
                    .map(|n| n as u16)
                    .or_else(|| v.as_str().and_then(|s| s.parse::<u16>().ok()))
            })
            .unwrap_or(0)
        };
        let mut stun_ip = String::new();
        let mut stun_port: u16 = 0;
        let mut turn_ip = String::new();
        let mut turn_port: u16 = 0;

        let body = serde_json::json!({ "type": 2 });
        if let Ok(b) = serde_json::to_vec(&body) {
            if let Ok((s, r)) = http.post(nat_url, &headers, &b) {
                if (200..300).contains(&s) {
                    let j: serde_json::Value = serde_json::from_str(&r).unwrap_or_default();
                    let d = j.get("data").unwrap_or(&j);
                    stun_ip = d.get("stunIp").and_then(|v| v.as_str()).unwrap_or("").into();
                    stun_port = parse_port(d.get("stunPort"));
                }
            }
        }
        let body = serde_json::json!({ "type": 3 });
        if let Ok(b) = serde_json::to_vec(&body) {
            if let Ok((s, r)) = http.post(nat_url, &headers, &b) {
                if (200..300).contains(&s) {
                    let j: serde_json::Value = serde_json::from_str(&r).unwrap_or_default();
                    let d = j.get("data").unwrap_or(&j);
                    turn_ip = d.get("turnIp").and_then(|v| v.as_str()).unwrap_or("").into();
                    turn_port = parse_port(d.get("turnPort"));
                }
            }
        }
        inner.lock().unwrap().nat_route = Some(NatRoute {
            stun_ip,
            stun_port,
            turn_ip,
            turn_port,
        });
    }

    // Step 2: Local addresses + UDP sockets for STUN/TURN queries
    let local_addrs = io.get_local_addresses();
    let local_addrs_v4: Vec<String> = local_addrs
        .iter()
        .filter(|a| !a.contains(':') && *a != "127.0.0.1")
        .cloned()
        .collect();

    let route = inner.lock().unwrap().nat_route.clone();
    let udp_v4 = io.create_udp(0).ok();

    // Step 3: STUN binding
    if let (Some(ref udp), Some(ref route)) = (&udp_v4, &route) {
        if !route.stun_ip.is_empty() && route.stun_port > 0 {
            let sip = route.stun_ip.clone();
            let sport = route.stun_port;
            let send = &mut |data: &[u8]| {
                let _ = udp.send_to(data, &sip, sport);
            };
            let recv = &mut |t: u64| -> Option<Vec<u8>> { udp.recv_from(t).ok().map(|(d, _, _)| d) };
            if let Ok(result) = get_external_address(send, recv, &sip, sport, p2p_token) {
                inner.lock().unwrap().stun_external_ip = result.ip.clone();
                inner.lock().unwrap().stun_external_port = result.port;
            }
        }
    }

    // Step 4: TURN allocate
    if let (Some(ref udp), Some(ref route)) = (&udp_v4, &route) {
        if !route.turn_ip.is_empty() && route.turn_port > 0 {
            let tip = route.turn_ip.clone();
            let tport = route.turn_port;
            let send = &mut |data: &[u8]| {
                let _ = udp.send_to(data, &tip, tport);
            };
            let recv = &mut |t: u64| -> Option<Vec<u8>> { udp.recv_from(t).ok().map(|(d, _, _)| d) };
            if let Ok(result) = get_turn_relay_address(send, recv, &tip, tport, p2p_token, AF_INET)
            {
                let mut inner = inner.lock().unwrap();
                inner.turn_relay_ip = result.relay_ip.clone();
                inner.turn_relay_port = result.relay_port;
            }
        }
    }

    // Step 5: Create ICE agent + dedicated UDP socket
    let ice_udp: Arc<dyn UdpTransport> = Arc::from(
        io.create_udp(0)
            .map_err(|e| format!("ICE UDP bind: {e}"))?,
    );
    let (_, ice_port) = ice_udp
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?;

    let mut agent = IceAgent::new(IceAgentConfig {
        is_controlling: true,
    });
    for addr in &local_addrs_v4 {
        agent.add_host_candidate(addr, ice_port);
    }
    {
        let guard = inner.lock().unwrap();
        if !guard.stun_external_ip.is_empty() {
            agent.add_local_candidate(IceCandidate {
                foundation: "srflx1".into(),
                component_id: 1,
                transport: "UDP".into(),
                priority: calc_candidate_priority(CandidateType::Srflx),
                connection_address: guard.stun_external_ip.clone(),
                port: guard.stun_external_port,
                candidate_type: CandidateType::Srflx,
                related_address: local_addrs_v4.first().cloned().unwrap_or_default(),
                related_port: ice_port,
            });
        }
        if !guard.turn_relay_ip.is_empty() {
            agent.add_local_candidate(IceCandidate {
                foundation: "relay1".into(),
                component_id: 1,
                transport: "UDP".into(),
                priority: calc_candidate_priority(CandidateType::Relay),
                connection_address: guard.turn_relay_ip.clone(),
                port: guard.turn_relay_port,
                candidate_type: CandidateType::Relay,
                related_address: guard.stun_external_ip.clone(),
                related_port: guard.stun_external_port,
            });
        }
    }

    // Step 6: SDP offer/answer
    let desc = agent.local_session_description();
    let mut default_ip = String::new();
    for line in &desc.candidates {
        if line.contains("host") && !line.contains("::") {
            let parts: Vec<&str> = line.split(' ').collect();
            if parts.len() >= 5 {
                default_ip = parts[4].into();
                break;
            }
        }
    }
    let sdp_offer = generate_sdp_offer(odid, &desc.ice_ufrag, &desc.ice_pwd, &desc.candidates, &default_ip, ice_port);
    let url = format!("http://{peer_addr}/api/ice/offer");
    let (_, resp) = http
        .post(
            &url,
            &[("Content-Type".into(), "application/sdp".into())],
            sdp_offer.as_bytes(),
        )
        .map_err(|e| format!("SDP HTTP: {e}"))?;

    let answer = parse_sdp_answer(resp.trim());
    agent.set_remote_session_description(&p2p_core::types::IceSessionDescription {
        ice_ufrag: answer.ufrag,
        ice_pwd: answer.pwd,
        is_lite: answer.is_lite,
        candidates: answer.candidates,
    });
    agent
        .start_checks()
        .map_err(|e| format!("start_checks: {e}"))?;

    // Step 7: Store agent + UDP, start threads
    {
        let mut inner = inner.lock().unwrap();
        inner.ice_agent = Some(agent);
        inner.ice_udp = Some(ice_udp);
    }
    start_ice_threads(inner);
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal: ICE thread management
// ---------------------------------------------------------------------------

fn stop_ice_threads(inner: &Arc<Mutex<ClientInner>>) {
    let (tick, recv) = {
        let mut guard = inner.lock().unwrap();
        guard.ice_stop.store(true, Ordering::Release);
        (guard.tick_handle.take(), guard.recv_handle.take())
    };
    if let Some(h) = tick {
        let _ = h.join();
    }
    if let Some(h) = recv {
        let _ = h.join();
    }
}

fn start_ice_threads(inner: &Arc<Mutex<ClientInner>>) {
    stop_ice_threads(inner);
    let stop = {
        let mut guard = inner.lock().unwrap();
        guard.ice_stop = Arc::new(AtomicBool::new(false));
        Arc::clone(&guard.ice_stop)
    };

    let inner_tick = inner.clone();
    let inner_recv = inner.clone();
    let stop_tick = stop.clone();
    let stop_recv = stop;

    let tick_handle = thread::spawn(move || {
        let mut now_ms: u64 = 0;
        let mut last_state: Option<IceState> = None;
        let mut last_heartbeat_ms: u64 = 0;

        while !stop_tick.load(Ordering::Relaxed) {
            now_ms += 50;

            // ICE tick
            let actions = {
                let mut guard = inner_tick.lock().unwrap();
                match &mut guard.ice_agent {
                    Some(agent) => {
                        let state = agent.state();
                        if last_state != Some(state) {
                            last_state = Some(state);
                            drop(guard);
                            fire_state_change(&inner_tick, state);
                            // Re-acquire for tick
                            let mut guard = inner_tick.lock().unwrap();
                            guard
                                .ice_agent
                                .as_mut()
                                .map(|a| a.tick(now_ms))
                                .unwrap_or_default()
                        } else {
                            agent.tick(now_ms)
                        }
                    }
                    None => Vec::new(),
                }
            };

            // Send STUN actions
            let udp = { inner_tick.lock().unwrap().ice_udp.clone() };
            if let Some(ref udp) = udp {
                for act in &actions {
                    let _ = udp.send_to(&act.data, &act.target_ip, act.target_port);
                }
            }

            // Heartbeat send
            let hb_secs = { inner_tick.lock().unwrap().heartbeat_interval_secs };
            if hb_secs > 0 && last_state == Some(IceState::Completed) {
                let hb_ms = hb_secs as u64 * 1000;
                if now_ms.saturating_sub(last_heartbeat_ms) >= hb_ms {
                    last_heartbeat_ms = now_ms;
                    let hb = encode_heartbeat_reply();
                    let guard = inner_tick.lock().unwrap();
                    if let Some(ref agent) = guard.ice_agent {
                        if let Some(act) = agent.send_data(&hb) {
                            if let Some(ref udp) = guard.ice_udp {
                                let _ = udp.send_to(&act.data, &act.target_ip, act.target_port);
                            }
                        }
                    }
                }
            }

            // Heartbeat timeout detection
            if hb_secs > 0 && last_state == Some(IceState::Completed) {
                let timeout = Duration::from_secs(hb_secs as u64 * 2);
                let timed_out = {
                    let guard = inner_tick.lock().unwrap();
                    guard
                        .last_recv_instant
                        .map(|t| t.elapsed() >= timeout)
                        .unwrap_or(false)
                };
                if timed_out {
                    fire_state_change(&inner_tick, IceState::Disconnected);
                }
            }

            thread::sleep(Duration::from_millis(50));
        }
    });

    let recv_handle = thread::spawn(move || {
        while !stop_recv.load(Ordering::Relaxed) {
            let udp = { inner_recv.lock().unwrap().ice_udp.clone() };
            let udp = match udp {
                Some(u) => u,
                None => {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };

            match udp.recv_from(200) {
                Ok((data, from_ip, from_port)) => {
                    let result = {
                        let mut guard = inner_recv.lock().unwrap();
                        match &mut guard.ice_agent {
                            Some(agent) => agent.handle_incoming_data(&data, &from_ip, from_port),
                            None => HandleDataResult {
                                app_data: None,
                                actions: Vec::new(),
                            },
                        }
                    };
                    for act in result.actions {
                        let _ = udp.send_to(&act.data, &act.target_ip, act.target_port);
                    }
                    if let Some(app_data) = result.app_data {
                        // Update last_recv_instant
                        inner_recv.lock().unwrap().last_recv_instant = Some(Instant::now());

                        // Auto-reply heartbeat
                        let hb_secs = { inner_recv.lock().unwrap().heartbeat_interval_secs };
                        if hb_secs > 0 {
                            if let Some(parsed) = parse_frame(&app_data) {
                                if parsed.frame_type == p2p_core::types::TYPE_HEARTBEAT {
                                    let hb = encode_heartbeat_reply();
                                    let guard = inner_recv.lock().unwrap();
                                    if let Some(ref agent) = guard.ice_agent {
                                        if let Some(act) = agent.send_data(&hb) {
                                            let _ = udp
                                                .send_to(&act.data, &act.target_ip, act.target_port);
                                        }
                                    }
                                    continue;
                                }
                            }
                        }

                        // Data frame → extract payload → callback
                        if let Some(parsed) = parse_frame(&app_data) {
                            if parsed.frame_type == p2p_core::types::TYPE_DATA {
                                let payload_len = parsed.payload.len();
                                if parsed.seq_id > 0 {
                                    sdk_log(&inner_recv, &format!(
                                        "[P2PSDK] recv → seqId={}, payload={} bytes",
                                        parsed.seq_id, payload_len));
                                    inner_recv.lock().unwrap()
                                        .pending_seq_ids.lock().unwrap()
                                        .push_back(parsed.seq_id);
                                } else {
                                    sdk_log(&inner_recv, &format!(
                                        "[P2PSDK] recv → seqId=0, payload={} bytes",
                                        payload_len));
                                }
                                fire_on_data(&inner_recv, parsed.payload);
                            } else {
                                sdk_log(&inner_recv, &format!(
                                    "[P2PSDK] recv → unknown frame type=0x{:x}, {} bytes",
                                    parsed.frame_type, app_data.len()));
                            }
                        } else {
                            sdk_log(&inner_recv, &format!(
                                "[P2PSDK] recv → parse_frame failed, {} bytes raw",
                                app_data.len()));
                        }
                    }
                }
                Err(_) => continue,
            }
        }
    });

    let mut guard = inner.lock().unwrap();
    guard.tick_handle = Some(tick_handle);
    guard.recv_handle = Some(recv_handle);
}

fn fire_state_change(inner: &Arc<Mutex<ClientInner>>, state: IceState) {
    // Take the callback out of the lock so it can be called without holding inner.
    // This avoids deadlock if the callback itself tries to acquire inner (e.g. send_data).
    let cb = {
        let mut guard = inner.lock().unwrap();
        guard.on_state_change.take()
    };
    if let Some(cb) = cb {
        cb(state);
        // Restore callback for future state changes
        inner.lock().unwrap().on_state_change = Some(cb);
    }
}

fn fire_on_data(inner: &Arc<Mutex<ClientInner>>, payload: Vec<u8>) {
    let guard = inner.lock().unwrap();
    if let Some(ref cb) = guard.on_data {
        cb(payload);
    }
}

fn sdk_log(inner: &Arc<Mutex<ClientInner>>, msg: &str) {
    let cb = {
        let mut guard = inner.lock().unwrap();
        guard.on_log.take()
    };
    if let Some(ref cb) = cb {
        cb(msg);
    }
    if cb.is_some() {
        inner.lock().unwrap().on_log = cb;
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
