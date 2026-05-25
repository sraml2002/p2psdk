//! P2P Client internal logic — NAPI-agnostic.
//!
//! All protocol logic runs in Rust. I/O uses p2p-tokio (POSIX).
//! This module exposes plain Rust functions for napi_bridge to call.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use p2p_core::frame::encode_data_frame;
use p2p_core::ice::agent::{HandleDataResult, IceAgent, IceAgentConfig};
use p2p_core::ice::check_list::calc_candidate_priority;
use p2p_core::sdp::{generate_sdp_offer, parse_sdp_answer};
use p2p_core::stun::client::{get_external_address, get_turn_relay_address};
use p2p_core::ice::candidate::format_candidate_line;
use p2p_core::types::{
    ConnectorMessage, IceCandidate, IceCandidateMessage, IceDataMessage,
    IceSessionDescription, CandidateType, AF_INET, AF_INET6,
};
use p2p_io::traits::{HttpTransport, Platform, UdpTransport};
use p2p_sdk::connector::ConnectorClient;
use p2p_tokio::http::SyncHttpTransport;
use p2p_tokio::platform::StdPlatform;
use p2p_tokio::udp::SyncUdpTransport;
use p2p_tokio::ws::SyncSignalingTransport;

use crate::hilog;
use crate::napi_bridge;

// ---------------------------------------------------------------------------
// Embedded token — encrypted at build time, decrypted at runtime
// ---------------------------------------------------------------------------

mod embedded_token {
    include!(concat!(env!("OUT_DIR"), "/embedded_token.rs"));

    pub fn decrypt() -> Option<String> {
        use aes_gcm::aead::Aead;
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
        use sha2::Digest;

        let seed = format!("p2psdk-embedded-token{}", EMBEDDED_TS);
        let aes_key = sha2::Sha256::digest(seed.as_bytes());
        let cipher = Aes256Gcm::new_from_slice(&aes_key).ok()?;
        let nonce = Nonce::from_slice(EMBEDDED_IV);
        let plain = cipher.decrypt(nonce, EMBEDDED_CIPHER.as_ref()).ok()?;
        String::from_utf8(plain).ok()
    }
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct NatRoute {
    stun_ip: String,
    stun_port: u16,
    turn_ip: String,
    turn_port: u16,
}

struct ThreadHandles {
    tick: Option<JoinHandle<()>>,
    recv: Option<JoinHandle<()>>,
    connector: Option<JoinHandle<()>>,
    ice_stop: Arc<AtomicBool>,
    connector_stop: Arc<AtomicBool>,
}

impl ThreadHandles {
    fn new() -> Self {
        Self {
            tick: None,
            recv: None,
            connector: None,
            ice_stop: Arc::new(AtomicBool::new(false)),
            connector_stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

struct OutgoingConnectorMsg {
    target_id: String,
    data: serde_json::Value,
}

struct Inner {
    ice_agent: Option<IceAgent>,
    ids_url: String,
    nat_url: String,
    nat_route: Option<NatRoute>,
    stun_external_ip: String,
    stun_external_port: u16,
    stun_external_ip_v6: String,
    stun_external_port_v6: u16,
    turn_relay_ip: String,
    turn_relay_port: u16,
    turn_relay_ip_v6: String,
    turn_relay_port_v6: u16,
    connector_registered: bool,
    identifier: String,
    cached_p2p_token: String,
    ice_udp: Option<Arc<SyncUdpTransport>>,
    threads: ThreadHandles,
    connector_tx: Option<mpsc::Sender<OutgoingConnectorMsg>>,
}

// ---------------------------------------------------------------------------
// Global singleton — one instance shared across all NAPI calls
// ---------------------------------------------------------------------------

static GLOBAL_INNER: once_cell::sync::Lazy<Arc<Mutex<Inner>>> =
    once_cell::sync::Lazy::new(|| {
        Arc::new(Mutex::new(Inner {
            ice_agent: None,
            ids_url: String::new(),
            nat_url: String::new(),
            nat_route: None,
            stun_external_ip: String::new(),
            stun_external_port: 0,
            stun_external_ip_v6: String::new(),
            stun_external_port_v6: 0,
            turn_relay_ip: String::new(),
            turn_relay_port: 0,
            turn_relay_ip_v6: String::new(),
            turn_relay_port_v6: 0,
            connector_registered: false,
            identifier: String::new(),
            cached_p2p_token: String::new(),
            ice_udp: None,
            threads: ThreadHandles::new(),
            connector_tx: None,
        }))
    });

fn get_inner() -> &'static Arc<Mutex<Inner>> {
    &GLOBAL_INNER
}

// ---------------------------------------------------------------------------
// Public API — called by napi_bridge.rs
// ---------------------------------------------------------------------------

pub fn init(config_json: &str) -> i32 {
    let config: serde_json::Value = match serde_json::from_str(config_json) {
        Ok(v) => v,
        Err(e) => {
            hilog::log_error(&format!("init: invalid JSON: {e}"));
            return -1;
        }
    };
    let mut inner = get_inner().lock().unwrap();
    inner.ids_url = config.get("idsUrl").and_then(|v| v.as_str()).unwrap_or("").into();
    inner.nat_url = config.get("natUrl").and_then(|v| v.as_str()).unwrap_or("").into();
    hilog::log_info(&format!("init: ids={}, nat={}", inner.ids_url, inner.nat_url));
    0
}

pub fn generate_token() -> String {
    match embedded_token::decrypt() {
        Some(token) => {
            hilog::log_info(&format!("generateToken: OK, len={}", token.len()));
            token
        }
        None => {
            hilog::log_error("generateToken: failed to decrypt embedded token");
            String::new()
        }
    }
}

pub fn gather_candidates(p2p_token: &str) -> String {
    let inner = get_inner().clone();
    let http = SyncHttpTransport::new();
    let platform = StdPlatform::new();

    let nat_url = {
        let guard = get_inner().lock().unwrap();
        guard.nat_url.clone()
    };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gather_candidates_sync(&inner, &http, &platform, p2p_token, &nat_url)
    }));

    match result {
        Ok(s) => s,
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".into()
            };
            hilog::log_error(&format!("gatherCandidates PANIC: {msg}"));
            format!("{{\"error\":\"panic: {msg}\"}}")
        }
    }
}

fn gather_candidates_inner(inner: &Arc<Mutex<Inner>>, http: &SyncHttpTransport, p2p_token: &str) {
    let platform = StdPlatform::new();
    let nat_url = {
        let guard = inner.lock().unwrap();
        guard.nat_url.clone()
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gather_candidates_sync(inner, http, &platform, p2p_token, &nat_url)
    }));
    match result {
        Ok(s) => hilog::log_info(&format!("connect: gatherCandidates done, result len={}", s.len())),
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".into()
            };
            hilog::log_error(&format!("connect: gatherCandidates PANIC: {msg}"));
        }
    }
}

pub fn connect_connector(url: &str, identifier: &str, auth_token: &str) -> i32 {
    stop_connector(get_inner());

    let (tx, rx) = mpsc::channel::<OutgoingConnectorMsg>();
    let stop = {
        let mut inner = get_inner().lock().unwrap();
        inner.threads.connector_stop = Arc::new(AtomicBool::new(false));
        inner.identifier = identifier.into();
        inner.connector_tx = Some(tx);
        Arc::clone(&inner.threads.connector_stop)
    };

    let inner = get_inner().clone();
    let url = url.to_string();
    let identifier = identifier.to_string();
    let auth_token = auth_token.to_string();

    let handle = thread::spawn(move || {
        connector_loop_inner(&url, &identifier, &auth_token, rx, &inner, &stop);
    });

    get_inner().lock().unwrap().threads.connector = Some(handle);
    0
}

pub fn disconnect_connector() -> i32 {
    stop_connector(get_inner());
    0
}

pub fn is_connector_registered() -> i32 {
    if get_inner().lock().unwrap().connector_registered { 1 } else { 0 }
}

pub fn initiate_ice(target_id: &str) -> i32 {
    let (token, identifier) = {
        let inner = get_inner().lock().unwrap();
        if !inner.connector_registered {
            hilog::log_error("initiateIce: connector not registered");
            return -1;
        }
        (inner.cached_p2p_token.clone(), inner.identifier.clone())
    };

    let inner = get_inner().clone();
    let target_id = target_id.to_string();
    let platform = StdPlatform::new();

    thread::spawn(move || {
        initiate_ice_bg_inner(&inner, &target_id, &identifier, &token, &platform);
    });
    0
}

pub fn stop_ice() -> i32 {
    stop_ice_threads(get_inner());
    let mut inner = get_inner().lock().unwrap();
    if let Some(agent) = &mut inner.ice_agent {
        agent.stop();
    }
    inner.ice_agent = None;
    0
}

pub fn send(data: &[u8]) -> i32 {
    let (action, udp) = {
        let inner = get_inner().lock().unwrap();
        let agent = match inner.ice_agent.as_ref() {
            Some(a) => a,
            None => {
                hilog::log_error("send: no ICE agent");
                return -2;
            }
        };
        let udp = match inner.ice_udp.as_ref() {
            Some(u) => u.clone(),
            None => {
                hilog::log_error("send: no UDP socket");
                return -3;
            }
        };
        (agent.send_data(&data), udp)
    };
    if let Some(act) = action {
        let _ = udp.send_to(&act.data, &act.target_ip, act.target_port);
    }
    0
}

pub fn send_text(text: &str) -> i32 {
    let frame = encode_data_frame(text);
    let (action, udp) = {
        let inner = get_inner().lock().unwrap();
        let agent = match inner.ice_agent.as_ref() {
            Some(a) => a,
            None => return -2,
        };
        let udp = match inner.ice_udp.as_ref() {
            Some(u) => u.clone(),
            None => return -3,
        };
        (agent.send_data(&frame), udp)
    };
    if let Some(act) = action {
        let _ = udp.send_to(&act.data, &act.target_ip, act.target_port);
    }
    0
}

pub fn register_ids(app_id: &str, user_id: &str, odid: &str, push_token: &str) -> String {
    let ids_url = {
        let inner = get_inner().lock().unwrap();
        inner.ids_url.clone()
    };
    let http = SyncHttpTransport::new();
    let url = format!("{ids_url}/api/ids");
    let body = serde_json::json!({
        "appId": app_id,
        "userId": user_id,
        "type": "app",
        "odid": odid,
        "token": push_token,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
    hilog::log_info(&format!("IDS register: POST {url}"));

    match http.post(&url, &[("Content-Type".into(), "application/json".into())], &body_bytes) {
        Ok((status, resp)) => {
            if (200..300).contains(&status) {
                resp
            } else {
                format!("{{\"error\":\"HTTP {status}\"}}")
            }
        }
        Err(e) => format!("{{\"error\":\"{e}\"}}"),
    }
}

pub fn query_ids(app_id: &str, user_id: &str) -> String {
    let ids_url = {
        let inner = get_inner().lock().unwrap();
        inner.ids_url.clone()
    };
    let http = SyncHttpTransport::new();
    let url = format!("{ids_url}/api/ids/{app_id}/{user_id}");
    hilog::log_info(&format!("IDS query: GET {url}"));

    match http.get(&url, &[]) {
        Ok((status, resp)) => {
            if (200..300).contains(&status) {
                resp
            } else {
                format!("{{\"error\":\"HTTP {status}\"}}")
            }
        }
        Err(e) => format!("{{\"error\":\"{e}\"}}"),
    }
}

pub fn ice_sdp_negotiate(peer_id: &str, odid: &str, is_device: bool) -> i32 {
    if is_device {
        hilog::log_error("iceSdpNegotiate: device-to-device (ICE-Full) not yet supported");
        return -1;
    }
    let inner = get_inner().clone();
    let http = SyncHttpTransport::new();
    let peer_id = peer_id.to_string();
    let odid = odid.to_string();

    thread::spawn(move || {
        ice_sdp_negotiate_bg_inner(&inner, &http, &peer_id, &odid);
    });
    0
}

pub fn connect(peer_id: &str, odid: &str, is_device: bool, heartbeat_interval: u32) -> i32 {
    if is_device {
        hilog::log_error("connect: device-to-device not yet supported");
        return -1;
    }

    let inner = get_inner().clone();
    let peer_id = peer_id.to_string();
    let odid = odid.to_string();

    thread::spawn(move || {
        let http = SyncHttpTransport::new();

        // Step 1: Get embedded token
        let p2p_token = generate_token();
        if p2p_token.is_empty() {
            hilog::log_error("connect: failed to get embedded token");
            return;
        }

        // Step 2: Gather candidates
        gather_candidates_inner(&inner, &http, &p2p_token);

        // Step 3: ICE SDP negotiate
        ice_sdp_negotiate_bg_inner(&inner, &http, &peer_id, &odid);

        let _ = heartbeat_interval;
        hilog::log_info(&format!("connect: negotiation started, heartbeat={}s", heartbeat_interval));
    });
    0
}

pub fn close() -> i32 {
    stop_ice_threads(get_inner());
    stop_connector(get_inner());
    let mut inner = get_inner().lock().unwrap();
    if let Some(agent) = &mut inner.ice_agent {
        agent.stop();
    }
    inner.ice_agent = None;
    inner.ice_udp = None;
    drop(inner);
    napi_bridge::release_all_tsfn();
    0
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn send_via_connector(inner: &Arc<Mutex<Inner>>, target_id: &str, data: &serde_json::Value) {
    let tx = {
        let guard = inner.lock().unwrap();
        guard.connector_tx.clone()
    };
    if let Some(tx) = tx {
        let _ = tx.send(OutgoingConnectorMsg {
            target_id: target_id.into(),
            data: data.clone(),
        });
    }
}

/// Stop ICE threads. Sets stop flag and takes handles while holding the lock,
/// then releases the lock before joining (avoids deadlock: threads also acquire the Mutex).
fn stop_ice_threads(inner: &Arc<Mutex<Inner>>) {
    let (tick, recv) = {
        let mut guard = inner.lock().unwrap();
        guard.threads.ice_stop.store(true, Ordering::Release);
        (guard.threads.tick.take(), guard.threads.recv.take())
    };
    if let Some(h) = tick { let _ = h.join(); }
    if let Some(h) = recv { let _ = h.join(); }
}

/// Stop connector thread. Same pattern as stop_ice_threads.
fn stop_connector(inner: &Arc<Mutex<Inner>>) {
    let handle = {
        let mut guard = inner.lock().unwrap();
        guard.threads.connector_stop.store(true, Ordering::Release);
        guard.connector_registered = false;
        guard.connector_tx = None;
        guard.threads.connector.take()
    };
    if let Some(h) = handle { let _ = h.join(); }
}

// ---------------------------------------------------------------------------
// Sync: Gather candidates (blocks until done)
// ---------------------------------------------------------------------------

fn gather_candidates_sync(
    inner: &Arc<Mutex<Inner>>,
    http: &SyncHttpTransport,
    platform: &StdPlatform,
    p2p_token: &str,
    nat_url: &str,
) -> String {
    hilog::log_info("Gathering candidates...");

    // Step 1: NAT route — fetch HTTP outside the lock, then store result
    {
        let needs_fetch = { inner.lock().unwrap().nat_route.is_none() && !nat_url.is_empty() };
        if needs_fetch {
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

            let stun_body = serde_json::json!({ "type": 2 });
            if let Ok(body_bytes) = serde_json::to_vec(&stun_body) {
                match http.post(nat_url, &headers, &body_bytes) {
                    Ok((status, resp)) if (200..300).contains(&status) => {
                        let json: serde_json::Value = serde_json::from_str(&resp).unwrap_or_default();
                        let data = json.get("data").unwrap_or(&json);
                        stun_ip = data.get("stunIp").and_then(|v| v.as_str()).unwrap_or("").into();
                        stun_port = parse_port(data.get("stunPort"));
                    }
                    Ok((status, _)) => hilog::log_error(&format!("NAT STUN: HTTP {status}")),
                    Err(e) => hilog::log_error(&format!("NAT STUN: {e}")),
                }
            }

            let turn_body = serde_json::json!({ "type": 3 });
            if let Ok(body_bytes) = serde_json::to_vec(&turn_body) {
                match http.post(nat_url, &headers, &body_bytes) {
                    Ok((status, resp)) if (200..300).contains(&status) => {
                        let json: serde_json::Value = serde_json::from_str(&resp).unwrap_or_default();
                        let data = json.get("data").unwrap_or(&json);
                        turn_ip = data.get("turnIp").and_then(|v| v.as_str()).unwrap_or("").into();
                        turn_port = parse_port(data.get("turnPort"));
                    }
                    Ok((status, _)) => hilog::log_error(&format!("NAT TURN: HTTP {status}")),
                    Err(e) => hilog::log_error(&format!("NAT TURN: {e}")),
                }
            }

            inner.lock().unwrap().nat_route = Some(NatRoute { stun_ip, stun_port, turn_ip, turn_port });
        }
    }

    // Step 2: Local addresses
    let all_addrs = platform.get_local_addresses();
    let local_addrs_v4: Vec<String> = all_addrs.iter()
        .filter(|a| !a.contains(':') && *a != "127.0.0.1")
        .cloned().collect();
    let local_addrs_v6: Vec<String> = all_addrs.iter()
        .filter(|a| a.contains(':') && *a != "::1" && *a != "::")
        .cloned().collect();

    // Step 3: UDP sockets
    let route = { inner.lock().unwrap().nat_route.clone() };

    let (udp_v4, port_v4) = match SyncUdpTransport::bind_any(0) {
        Ok(u) => {
            let (_, port) = u.local_addr().unwrap_or(("".into(), 0));
            (Some(u), port)
        }
        Err(e) => { hilog::log_error(&format!("UDP v4 bind: {e}")); (None, 0u16) }
    };
    let (udp_v6, port_v6) = match SyncUdpTransport::bind_any_v6(0) {
        Ok(u) => {
            let (_, port) = u.local_addr().unwrap_or(("".into(), 0));
            (Some(u), port)
        }
        Err(e) => { hilog::log_error(&format!("UDP v6 bind: {e}")); (None, 0u16) }
    };

    if udp_v4.is_none() && udp_v6.is_none() {
        return serde_json::json!({
            "candidateLines": ["UDP bind failed"],
            "localAddresses": all_addrs,
        }).to_string();
    }

    // Step 4: STUN binding IPv4
    if let (Some(ref udp), Some(ref route)) = (&udp_v4, &route) {
        if !route.stun_ip.is_empty() && route.stun_port > 0 {
            let stun_ip = route.stun_ip.clone();
            let stun_port = route.stun_port;
            let send = &mut |data: &[u8]| { let _ = udp.send_to(data, &stun_ip, stun_port); };
            let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                udp.recv_from(timeout_ms).ok().map(|(d, _, _)| d)
            };
            match get_external_address(send, recv, &stun_ip, stun_port, p2p_token) {
                Ok(result) => {
                    let mut guard = inner.lock().unwrap();
                    guard.stun_external_ip = result.ip.clone();
                    guard.stun_external_port = result.port;
                }
                Err(e) => hilog::log_error(&format!("STUN v4: {e}")),
            }
        }
    }

    // Step 4b: STUN binding IPv6
    if let (Some(ref udp), Some(ref route)) = (&udp_v6, &route) {
        if !local_addrs_v6.is_empty() && !route.stun_ip.is_empty() && route.stun_port > 0 {
            let stun_ip = route.stun_ip.clone();
            let stun_port = route.stun_port;
            let send = &mut |data: &[u8]| { let _ = udp.send_to(data, &stun_ip, stun_port); };
            let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                udp.recv_from(timeout_ms).ok().map(|(d, _, _)| d)
            };
            match get_external_address(send, recv, &stun_ip, stun_port, p2p_token) {
                Ok(result) => {
                    let mut guard = inner.lock().unwrap();
                    guard.stun_external_ip_v6 = result.ip.clone();
                    guard.stun_external_port_v6 = result.port;
                }
                Err(e) => hilog::log_error(&format!("STUN v6: {e}")),
            }
        }
    }

    // Step 5: TURN allocate IPv4
    if let (Some(ref udp), Some(ref route)) = (&udp_v4, &route) {
        if !route.turn_ip.is_empty() && route.turn_port > 0 {
            let turn_ip = route.turn_ip.clone();
            let turn_port = route.turn_port;
            let send = &mut |data: &[u8]| { let _ = udp.send_to(data, &turn_ip, turn_port); };
            let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                udp.recv_from(timeout_ms).ok().map(|(d, _, _)| d)
            };
            match get_turn_relay_address(send, recv, &turn_ip, turn_port, p2p_token, AF_INET) {
                Ok(result) => {
                    let mut guard = inner.lock().unwrap();
                    guard.turn_relay_ip = result.relay_ip.clone();
                    guard.turn_relay_port = result.relay_port;
                }
                Err(e) => hilog::log_error(&format!("TURN v4: {e}")),
            }
        }
    }

    // Step 5b: TURN allocate IPv6
    if let (Some(ref udp), Some(ref route)) = (&udp_v6, &route) {
        if !local_addrs_v6.is_empty() && !route.turn_ip.is_empty() && route.turn_port > 0 {
            let turn_ip = route.turn_ip.clone();
            let turn_port = route.turn_port;
            let send = &mut |data: &[u8]| { let _ = udp.send_to(data, &turn_ip, turn_port); };
            let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                udp.recv_from(timeout_ms).ok().map(|(d, _, _)| d)
            };
            match get_turn_relay_address(send, recv, &turn_ip, turn_port, p2p_token, AF_INET6) {
                Ok(result) => {
                    let mut guard = inner.lock().unwrap();
                    if result.relay_ip.contains(':') {
                        guard.turn_relay_ip_v6 = result.relay_ip.clone();
                        guard.turn_relay_port_v6 = result.relay_port;
                    }
                }
                Err(e) => hilog::log_error(&format!("TURN v6: {e}")),
            }
        }
    }

    // Step 6: Build candidate lines
    let guard = inner.lock().unwrap();
    let mut lines = Vec::new();
    let mut foundation: u32 = 1;

    for addr in &local_addrs_v4 {
        let cand = IceCandidate {
            foundation: foundation.to_string(), component_id: 1, transport: "UDP".into(),
            priority: calc_candidate_priority(CandidateType::Host),
            connection_address: addr.clone(), port: port_v4,
            candidate_type: CandidateType::Host, related_address: String::new(), related_port: 0,
        };
        lines.push(format_candidate_line(&cand));
        foundation += 1;
    }
    for addr in &local_addrs_v6 {
        let cand = IceCandidate {
            foundation: foundation.to_string(), component_id: 1, transport: "UDP".into(),
            priority: calc_candidate_priority(CandidateType::Host),
            connection_address: addr.clone(), port: port_v6,
            candidate_type: CandidateType::Host, related_address: String::new(), related_port: 0,
        };
        lines.push(format_candidate_line(&cand));
        foundation += 1;
    }
    if !guard.stun_external_ip.is_empty() {
        let cand = IceCandidate {
            foundation: foundation.to_string(), component_id: 1, transport: "UDP".into(),
            priority: calc_candidate_priority(CandidateType::Srflx),
            connection_address: guard.stun_external_ip.clone(), port: guard.stun_external_port,
            candidate_type: CandidateType::Srflx,
            related_address: local_addrs_v4.first().cloned().unwrap_or_default(), related_port: port_v4,
        };
        lines.push(format_candidate_line(&cand));
        foundation += 1;
    }
    if !guard.turn_relay_ip.is_empty() {
        let cand = IceCandidate {
            foundation: foundation.to_string(), component_id: 1, transport: "UDP".into(),
            priority: calc_candidate_priority(CandidateType::Relay),
            connection_address: guard.turn_relay_ip.clone(), port: guard.turn_relay_port,
            candidate_type: CandidateType::Relay,
            related_address: guard.stun_external_ip.clone(), related_port: guard.stun_external_port,
        };
        lines.push(format_candidate_line(&cand));
    }

    let info = serde_json::json!({
        "candidateLines": if lines.is_empty() { vec!["no candidates found".to_string()] } else { lines },
        "localAddresses": all_addrs,
        "stunExternalIp": guard.stun_external_ip,
        "stunExternalPort": guard.stun_external_port.to_string(),
        "turnRelayIp": guard.turn_relay_ip,
        "turnRelayPort": guard.turn_relay_port.to_string(),
    });

    drop(guard);

    // Cache p2p_token and create ICE agent for later use
    inner.lock().unwrap().cached_p2p_token = p2p_token.into();
    {
        let ice_udp = match SyncUdpTransport::bind_any(0) {
            Ok(u) => Arc::new(u),
            Err(_) => return info.to_string(),
        };
        let (_, ice_port) = match ice_udp.local_addr() {
            Ok(a) => a,
            Err(_) => return info.to_string(),
        };

        let mut agent = IceAgent::new(IceAgentConfig { is_controlling: true });
        for addr in &local_addrs_v4 { agent.add_host_candidate(addr, ice_port); }
        for addr in &local_addrs_v6 { agent.add_host_candidate(addr, ice_port); }

        let g = inner.lock().unwrap();
        if !g.stun_external_ip.is_empty() {
            agent.add_local_candidate(IceCandidate {
                foundation: "srflx1".into(), component_id: 1, transport: "UDP".into(),
                priority: calc_candidate_priority(CandidateType::Srflx),
                connection_address: g.stun_external_ip.clone(), port: g.stun_external_port,
                candidate_type: CandidateType::Srflx,
                related_address: local_addrs_v4.first().cloned().unwrap_or_default(), related_port: port_v4,
            });
        }
        if !g.turn_relay_ip.is_empty() {
            agent.add_local_candidate(IceCandidate {
                foundation: "relay1".into(), component_id: 1, transport: "UDP".into(),
                priority: calc_candidate_priority(CandidateType::Relay),
                connection_address: g.turn_relay_ip.clone(), port: g.turn_relay_port,
                candidate_type: CandidateType::Relay,
                related_address: g.stun_external_ip.clone(), related_port: g.stun_external_port,
            });
        }
        drop(g);

        let mut guard = inner.lock().unwrap();
        guard.ice_agent = Some(agent);
        guard.ice_udp = Some(ice_udp);
    }

    info.to_string()
}

// ---------------------------------------------------------------------------
// Background: Connector loop (no callbacks, logs to hilog)
// ---------------------------------------------------------------------------

fn connector_loop_inner(
    url: &str,
    identifier: &str,
    auth_token: &str,
    rx: mpsc::Receiver<OutgoingConnectorMsg>,
    inner: &Arc<Mutex<Inner>>,
    stop: &Arc<AtomicBool>,
) {
    let initial_delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(30);
    let mut delay = initial_delay;

    while !stop.load(Ordering::Relaxed) {
        let ws = SyncSignalingTransport::new();
        let mut connector = ConnectorClient::new(Box::new(ws));

        match connector.connect(url, identifier, auth_token) {
            Ok(()) => { delay = initial_delay; hilog::log_info(&format!("Connector connected to {url}")); }
            Err(e) => {
                hilog::log_error(&format!("Connector connect failed: {e}"));
                thread::sleep(delay);
                delay = (delay * 2).min(max_delay);
                continue;
            }
        }

        inner.lock().unwrap().connector_registered = false;
        let mut was_registered = false;

        while !stop.load(Ordering::Relaxed) {
            while let Ok(out_msg) = rx.try_recv() {
                match connector.send_to(&out_msg.target_id, &out_msg.data) {
                    Ok(()) => hilog::log_info(&format!("Sent to {}", out_msg.target_id)),
                    Err(e) => hilog::log_error(&format!("Send failed: {e}")),
                }
            }

            let msgs = connector.poll();
            if !connector.is_connected() {
                if was_registered {
                    inner.lock().unwrap().connector_registered = false;
                    napi_bridge::fire_connector_state(false);
                    napi_bridge::fire_state("CONNECTOR_DISCONNECTED");
                }
                break;
            }
            if connector.is_registered() && !was_registered {
                was_registered = true;
                inner.lock().unwrap().connector_registered = true;
                hilog::log_info("Connector registered");
                napi_bridge::fire_connector_state(true);
                napi_bridge::fire_state("CONNECTOR_REGISTERED");
            }

            for msg in msgs {
                handle_connector_msg_inner(&msg, inner);
            }
            thread::sleep(Duration::from_millis(50));
        }

        connector.disconnect();
        while rx.try_recv().is_ok() {}
        if !stop.load(Ordering::Relaxed) {
            thread::sleep(delay);
            delay = (delay * 2).min(max_delay);
        }
    }
}

fn handle_connector_msg_inner(msg: &ConnectorMessage, inner: &Arc<Mutex<Inner>>) {
    let action = msg.data.get("action").and_then(|v| v.as_str()).unwrap_or("");
    hilog::log_info(&format!("Connector msg: {action} from {}", msg.from));

    match action {
        "iceOffer" => handle_ice_offer_inner(&msg.from, &msg.data, inner),
        "iceAnswer" => handle_ice_answer_inner(&msg.data, inner),
        "iceCandidate" => {
            let cand_msg: IceCandidateMessage = match serde_json::from_value(msg.data.clone()) {
                Ok(m) => m,
                Err(_) => return,
            };
            let mut guard = inner.lock().unwrap();
            if let Some(agent) = &mut guard.ice_agent {
                agent.add_remote_candidate(&cand_msg.candidate);
            }
        }
        _ => {}
    }
}

fn handle_ice_offer_inner(
    from_id: &str,
    data: &serde_json::Value,
    inner: &Arc<Mutex<Inner>>,
) {
    hilog::log_info("Handling iceOffer");
    {
        stop_ice_threads(inner);
        let mut guard = inner.lock().unwrap();
        if let Some(agent) = &mut guard.ice_agent { agent.stop(); }
        guard.ice_agent = None;
    }

    let platform = StdPlatform::new();
    let udp = match SyncUdpTransport::bind_any(0) {
        Ok(u) => Arc::new(u),
        Err(e) => { hilog::log_error(&format!("UDP bind: {e}")); return; }
    };
    let local_addrs = platform.get_local_addresses();
    let (_, local_port) = match udp.local_addr() {
        Ok(a) => a,
        Err(e) => { hilog::log_error(&format!("local_addr: {e}")); return; }
    };

    let mut agent = IceAgent::new(IceAgentConfig { is_controlling: false });
    for addr in &local_addrs {
        if *addr != "127.0.0.1" && *addr != "::1" && *addr != "0.0.0.0" {
            agent.add_host_candidate(addr, local_port);
        }
    }

    {
        let guard = inner.lock().unwrap();
        if !guard.stun_external_ip.is_empty() {
            agent.add_local_candidate(IceCandidate {
                foundation: "srflx1".into(), component_id: 1, transport: "UDP".into(),
                priority: calc_candidate_priority(CandidateType::Srflx),
                connection_address: guard.stun_external_ip.clone(), port: guard.stun_external_port,
                candidate_type: CandidateType::Srflx,
                related_address: local_addrs.first().cloned().unwrap_or_default(), related_port: local_port,
            });
        }
        if !guard.turn_relay_ip.is_empty() {
            agent.add_local_candidate(IceCandidate {
                foundation: "relay1".into(), component_id: 1, transport: "UDP".into(),
                priority: calc_candidate_priority(CandidateType::Relay),
                connection_address: guard.turn_relay_ip.clone(), port: guard.turn_relay_port,
                candidate_type: CandidateType::Relay,
                related_address: guard.stun_external_ip.clone(), related_port: guard.stun_external_port,
            });
        }
    }

    let ice_msg: IceDataMessage = match serde_json::from_value(data.clone()) {
        Ok(m) => m,
        Err(e) => { hilog::log_error(&format!("Parse iceOffer: {e}")); return; }
    };
    agent.set_remote_session_description(&IceSessionDescription {
        ice_ufrag: ice_msg.ice_ufrag,
        ice_pwd: ice_msg.ice_pwd,
        is_lite: ice_msg.is_lite,
        candidates: ice_msg.candidates,
    });
    agent.start_checks().unwrap_or_else(|e| hilog::log_error(&format!("start_checks: {e}")));

    let desc = agent.local_session_description();
    let answer = IceDataMessage {
        action: "iceAnswer".into(),
        ice_ufrag: desc.ice_ufrag,
        ice_pwd: desc.ice_pwd,
        is_lite: desc.is_lite,
        candidates: desc.candidates,
    };
    send_via_connector(inner, from_id, &serde_json::to_value(&answer).unwrap_or_default());

    {
        let mut guard = inner.lock().unwrap();
        guard.ice_agent = Some(agent);
        guard.ice_udp = Some(udp);
    }
    start_ice_threads_inner(inner);
}

fn handle_ice_answer_inner(data: &serde_json::Value, inner: &Arc<Mutex<Inner>>) {
    hilog::log_info("Handling iceAnswer");
    let ice_msg: IceDataMessage = match serde_json::from_value(data.clone()) {
        Ok(m) => m,
        Err(e) => { hilog::log_error(&format!("Parse iceAnswer: {e}")); return; }
    };
    let mut guard = inner.lock().unwrap();
    if let Some(agent) = &mut guard.ice_agent {
        agent.set_remote_session_description(&IceSessionDescription {
            ice_ufrag: ice_msg.ice_ufrag,
            ice_pwd: ice_msg.ice_pwd,
            is_lite: ice_msg.is_lite,
            candidates: ice_msg.candidates,
        });
        if let Err(e) = agent.start_checks() {
            drop(guard);
            hilog::log_error(&format!("start_checks: {e}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Background: Initiate ICE (offerer)
// ---------------------------------------------------------------------------

fn initiate_ice_bg_inner(
    inner: &Arc<Mutex<Inner>>,
    target_id: &str,
    _identifier: &str,
    p2p_token: &str,
    platform: &StdPlatform,
) {
    hilog::log_info(&format!("Initiating ICE to {target_id}"));

    {
        stop_ice_threads(inner);
        let mut guard = inner.lock().unwrap();
        if let Some(agent) = &mut guard.ice_agent { agent.stop(); }
        guard.ice_agent = None;
    }

    let udp = match SyncUdpTransport::bind_any(0) {
        Ok(u) => Arc::new(u),
        Err(e) => { hilog::log_error(&format!("UDP bind: {e}")); return; }
    };
    let local_addrs = platform.get_local_addresses();
    let (_, local_port) = match udp.local_addr() {
        Ok(a) => a,
        Err(e) => { hilog::log_error(&format!("local_addr: {e}")); return; }
    };

    let mut agent = IceAgent::new(IceAgentConfig { is_controlling: true });
    for addr in &local_addrs {
        if *addr != "127.0.0.1" && *addr != "::1" && *addr != "0.0.0.0" {
            agent.add_host_candidate(addr, local_port);
        }
    }

    let route = { inner.lock().unwrap().nat_route.clone() };
    if let Some(ref route) = route {
        if !route.stun_ip.is_empty() && route.stun_port > 0 {
            let cached_ip = { inner.lock().unwrap().stun_external_ip.clone() };
            if !cached_ip.is_empty() {
                agent.add_local_candidate(IceCandidate {
                    foundation: "srflx1".into(), component_id: 1, transport: "UDP".into(),
                    priority: calc_candidate_priority(CandidateType::Srflx),
                    connection_address: cached_ip, port: inner.lock().unwrap().stun_external_port,
                    candidate_type: CandidateType::Srflx,
                    related_address: local_addrs.first().cloned().unwrap_or_default(), related_port: local_port,
                });
            } else {
                let stun_ip = route.stun_ip.clone();
                let stun_port = route.stun_port;
                let send = &mut |data: &[u8]| { let _ = udp.send_to(data, &stun_ip, stun_port); };
                let recv = &mut |timeout_ms: u64| -> Option<Vec<u8>> {
                    udp.recv_from(timeout_ms).ok().map(|(d, _, _)| d)
                };
                match get_external_address(send, recv, &stun_ip, stun_port, p2p_token) {
                    Ok(result) => {
                        agent.add_local_candidate(IceCandidate {
                            foundation: "srflx1".into(), component_id: 1, transport: "UDP".into(),
                            priority: calc_candidate_priority(CandidateType::Srflx),
                            connection_address: result.ip.clone(), port: result.port,
                            candidate_type: CandidateType::Srflx,
                            related_address: local_addrs.first().cloned().unwrap_or_default(), related_port: local_port,
                        });
                        let mut guard = inner.lock().unwrap();
                        guard.stun_external_ip = result.ip;
                        guard.stun_external_port = result.port;
                    }
                    Err(e) => hilog::log_error(&format!("ICE STUN: {e}")),
                }
            }
        }
    }

    {
        let guard = inner.lock().unwrap();
        if !guard.turn_relay_ip.is_empty() {
            agent.add_local_candidate(IceCandidate {
                foundation: "relay1".into(), component_id: 1, transport: "UDP".into(),
                priority: calc_candidate_priority(CandidateType::Relay),
                connection_address: guard.turn_relay_ip.clone(), port: guard.turn_relay_port,
                candidate_type: CandidateType::Relay,
                related_address: guard.stun_external_ip.clone(), related_port: guard.stun_external_port,
            });
        }
    }

    let desc = agent.local_session_description();
    let offer = IceDataMessage {
        action: "iceOffer".into(),
        ice_ufrag: desc.ice_ufrag,
        ice_pwd: desc.ice_pwd,
        is_lite: desc.is_lite,
        candidates: desc.candidates,
    };

    {
        let mut guard = inner.lock().unwrap();
        guard.ice_agent = Some(agent);
        guard.ice_udp = Some(udp);
    }

    send_via_connector(inner, target_id, &serde_json::to_value(&offer).unwrap_or_default());
    start_ice_threads_inner(inner);
}

// ---------------------------------------------------------------------------
// ICE threads (tick + recv, no callbacks)
// ---------------------------------------------------------------------------

fn start_ice_threads_inner(inner: &Arc<Mutex<Inner>>) {
    stop_ice_threads(inner);
    let stop = {
        let mut guard = inner.lock().unwrap();
        guard.threads.ice_stop = Arc::new(AtomicBool::new(false));
        Arc::clone(&guard.threads.ice_stop)
    };

    let inner_tick = inner.clone();
    let inner_recv = inner.clone();
    let stop_tick = stop.clone();
    let stop_recv = stop;

    let tick_handle = thread::spawn(move || {
        let mut now_ms: u64 = 0;
        let mut last_state: String = String::new();
        while !stop_tick.load(Ordering::Relaxed) {
            now_ms += 50;
            let actions = {
                let mut guard = inner_tick.lock().unwrap();
                match &mut guard.ice_agent {
                    Some(agent) => {
                        let state = format!("{:?}", agent.state()).to_uppercase();
                        if state != last_state {
                            last_state = state.clone();
                            napi_bridge::fire_state(&state);
                            hilog::log_info(&format!("ICE state: {state}"));
                        }
                        agent.tick(now_ms)
                    }
                    None => Vec::new(),
                }
            };
            let udp = { inner_tick.lock().unwrap().ice_udp.clone() };
            if let Some(ref udp) = udp {
                for act in &actions {
                    let _ = udp.send_to(&act.data, &act.target_ip, act.target_port);
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
                None => { thread::sleep(Duration::from_millis(100)); continue; }
            };
            match udp.recv_from(200) {
                Ok((data, from_ip, from_port)) => {
                    let result = {
                        let mut guard = inner_recv.lock().unwrap();
                        match &mut guard.ice_agent {
                            Some(agent) => agent.handle_incoming_data(&data, &from_ip, from_port),
                            None => HandleDataResult { app_data: None, actions: Vec::new() },
                        }
                    };
                    for act in result.actions {
                        let _ = udp.send_to(&act.data, &act.target_ip, act.target_port);
                    }
                    if let Some(app_data) = result.app_data {
                        hilog::log_info(&format!("ICE app data: {} bytes", app_data.len()));
                        napi_bridge::fire_data(&app_data);
                    }
                }
                Err(_) => continue,
            }
        }
    });

    let mut guard = inner.lock().unwrap();
    guard.threads.tick = Some(tick_handle);
    guard.threads.recv = Some(recv_handle);
}

// ---------------------------------------------------------------------------
// Background: Connect via SDP
// ---------------------------------------------------------------------------

fn ice_sdp_negotiate_bg_inner(
    inner: &Arc<Mutex<Inner>>,
    http: &SyncHttpTransport,
    peer_id: &str,
    odid: &str,
) {
    hilog::log_info(&format!("ICE SDP negotiate to {peer_id}, odid={odid}"));

    let (sdp_offer, _default_port) = {
        let guard = inner.lock().unwrap();
        let agent = match &guard.ice_agent {
            Some(a) => a,
            None => { hilog::log_error("iceSdpNegotiate: no ICE agent"); return; }
        };
        let desc = agent.local_session_description();
        let mut dip = String::new();
        for line in &desc.candidates {
            if line.contains("host") && !line.contains("::") {
                let parts: Vec<&str> = line.split(' ').collect();
                if parts.len() >= 5 { dip = parts[4].into(); break; }
            }
        }
        let port = guard.ice_udp.as_ref().and_then(|u| u.local_addr().ok().map(|a| a.1)).unwrap_or(0);
        let offer = generate_sdp_offer(odid, &desc.ice_ufrag, &desc.ice_pwd, &desc.candidates, &dip, port);
        (offer, port)
    };

    let url = format!("http://{peer_id}/api/ice/offer");

    match http.post(&url, &[("Content-Type".into(), "application/sdp".into())], sdp_offer.as_bytes()) {
        Ok((status, resp)) if (200..300).contains(&status) => {
            let sdp_answer = resp.trim();
            if sdp_answer.is_empty() {
                hilog::log_error("iceSdpNegotiate: empty SDP answer");
                return;
            }
            let answer_info = parse_sdp_answer(sdp_answer);
            let remote_desc = IceSessionDescription {
                ice_ufrag: answer_info.ufrag.clone(),
                ice_pwd: answer_info.pwd.clone(),
                is_lite: answer_info.is_lite,
                candidates: answer_info.candidates.clone(),
            };

            let mut guard = inner.lock().unwrap();
            if let Some(agent) = &mut guard.ice_agent {
                agent.set_remote_session_description(&remote_desc);
                agent.start_checks().unwrap_or_else(|e| hilog::log_error(&format!("start_checks: {e}")));
            }
            drop(guard);
            start_ice_threads_inner(inner);
            hilog::log_info(&format!("SDP answer processed, {} remote candidates", answer_info.candidates.len()));
        }
        Ok((status, _)) => hilog::log_error(&format!("SDP HTTP {status}")),
        Err(e) => hilog::log_error(&format!("SDP error: {e}")),
    }
}
