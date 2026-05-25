//! P2P SDK Rust Demo App
//!
//! 直接依赖 p2p-sdk/p2p-tokio/p2p-core crate，编译为单一可执行文件。
//! 对标 ArkTS App "P2P Chat"：一键连接 + 聊天。

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use p2p_core::frame::{encode_heartbeat_reply, parse_frame};
use p2p_core::ice::agent::HandleDataResult;
use p2p_core::types::{IceState, TYPE_DATA, TYPE_HEARTBEAT};
use p2p_io::traits::{Platform, UdpTransport};
use p2p_sdk::{Config, P2pClient};
use p2p_tokio::{StdPlatform, SyncHttpTransport, SyncUdpTransport};

// ── 配置 ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct AppConfig {
    #[serde(rename = "idsUrl")]
    ids_url: String,
    #[serde(rename = "natUrl")]
    nat_url: String,
    #[serde(rename = "appId")]
    app_id: String,
    #[serde(rename = "userId")]
    user_id: String,
    odid: String,
}

fn read_config(path: &str) -> AppConfig {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("读取配置失败 '{}': {}", path, e);
        std::process::exit(1);
    });
    serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("解析配置失败: {}", e);
        std::process::exit(1);
    })
}

// ── 编译时嵌入的 Token（build.rs 加密嵌入，运行时解密） ────────────────

mod embedded_token {
    include!(concat!(env!("OUT_DIR"), "/embedded_token.rs"));

    pub fn decrypt() -> Option<String> {
        use sha2::Digest;
        let seed = format!("p2psdk-embedded-token{}", EMBEDDED_TS);
        let aes_key = sha2::Sha256::digest(seed.as_bytes());

        let iv_bytes = {
            let mut iv = [0u8; 12];
            for i in 0..12 {
                iv[i] = aes_key[i] ^ aes_key[i + 12] ^ aes_key[i + 20];
            }
            iv
        };

        use aes_gcm::aead::Aead;
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
        let cipher = Aes256Gcm::new_from_slice(&aes_key).ok()?;
        let nonce = Nonce::from_slice(&iv_bytes);
        let plain = cipher.decrypt(nonce, EMBEDDED_CIPHER).ok()?;
        String::from_utf8(plain).ok()
    }
}

// ── 一键建链 ─────────────────────────────────────────────────────────

struct IceRunner {
    client: Arc<Mutex<P2pClient>>,
    udp: Arc<SyncUdpTransport>,
    stop: Arc<AtomicBool>,
    data_tx: Sender<String>,
    state_tx: Sender<IceState>,
    heartbeat_interval: u64,
}

impl IceRunner {
    /// 一键连接：resolve NAT → gather candidates → SDP negotiate → 启动 ICE 线程
    fn connect(
        token: &str,
        config: &AppConfig,
        peer_addr: &str,
        data_tx: Sender<String>,
        state_tx: Sender<IceState>,
    ) -> Result<Self, String> {
        let http = SyncHttpTransport::new();
        let platform = StdPlatform::new();

        let mut client = P2pClient::new();
        client.init(Config {
            ids_url: config.ids_url.clone(),
            nat_url: config.nat_url.clone(),
            app_id: config.app_id.clone(),
        });

        // 获取 STUN/TURN 服务器地址
        client.resolve_nat_route(&http, token)?;

        // 绑定 UDP + 收集候选 + 创建 IceAgent
        let udp = SyncUdpTransport::bind_any(0)
            .map_err(|e| format!("UDP bind 失败: {}", e))?;
        client.setup_ice_and_gather(&udp, &platform, token, true)?;

        // 获取 default IP/port 用于 SDP offer
        let (default_ip, default_port) = udp.local_addr()
            .map_err(|e| format!("获取本地地址失败: {}", e))?;
        let local_addrs = platform.get_local_addresses();
        let default_ip = local_addrs
            .iter()
            .find(|a| *a != "127.0.0.1" && *a != "::1")
            .cloned()
            .unwrap_or(default_ip);

        let client = Arc::new(Mutex::new(client));
        let udp = Arc::new(udp);
        let stop = Arc::new(AtomicBool::new(false));

        let runner = Self {
            client: client.clone(),
            udp: udp.clone(),
            stop: stop.clone(),
            data_tx,
            state_tx,
            heartbeat_interval: 30,
        };

        // SDP 交换 + start_checks
        {
            let mut c = runner.client.lock().unwrap();
            c.connect_via_sdp(&http, &config.odid, peer_addr, &default_ip, default_port)?;
        }

        // 启动 tick 和 recv 线程
        runner.start_tick_thread();
        runner.start_recv_thread();

        Ok(runner)
    }

    fn start_tick_thread(&self) {
        let client = self.client.clone();
        let udp = self.udp.clone();
        let stop = self.stop.clone();
        let state_tx = self.state_tx.clone();
        let heartbeat_interval = self.heartbeat_interval;

        thread::spawn(move || {
            let mut now_ms: u64 = 0;
            let mut last_state: Option<IceState> = None;
            let mut last_hb_ms: u64 = 0;

            while !stop.load(Ordering::Relaxed) {
                let actions = {
                    let mut c = client.lock().unwrap();
                    let actions = c.tick(now_ms);

                    // 检测 ICE 状态变化
                    if let Some(state) = c.ice_state() {
                        if last_state != Some(state) {
                            let _ = state_tx.send(state);
                            last_state = Some(state);
                        }
                    }

                    actions
                };

                for act in &actions {
                    let _ = udp.send_to(&act.data, &act.target_ip, act.target_port);
                }

                // 心跳
                if heartbeat_interval > 0 && last_state == Some(IceState::Completed) {
                    let elapsed = now_ms.saturating_sub(last_hb_ms);
                    if elapsed >= heartbeat_interval * 1000 {
                        let hb = encode_heartbeat_reply();
                        let c = client.lock().unwrap();
                        if let Some(act) = c.send_data(&hb) {
                            let _ = udp.send_to(&act.data, &act.target_ip, act.target_port);
                        }
                        last_hb_ms = now_ms;
                    }
                }

                now_ms += 50;
                thread::sleep(Duration::from_millis(50));
            }
        });
    }

    fn start_recv_thread(&self) {
        let client = self.client.clone();
        let udp = self.udp.clone();
        let stop = self.stop.clone();
        let data_tx = self.data_tx.clone();
        let heartbeat_interval = self.heartbeat_interval;

        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let (data, from_ip, from_port): (Vec<u8>, String, u16) = match udp.recv_from(200) {
                    Ok(result) => result,
                    Err(_) => continue,
                };

                let HandleDataResult { app_data, actions } = {
                    let mut c = client.lock().unwrap();
                    c.handle_incoming_udp(&data, &from_ip, from_port)
                };

                for act in &actions {
                    let _ = udp.send_to(&act.data, &act.target_ip, act.target_port);
                }

                if let Some(payload) = app_data {
                    if heartbeat_interval > 0 {
                        // 心跳帧自动回复
                        if let Some(frame) = parse_frame(&payload) {
                            if frame.frame_type == TYPE_HEARTBEAT {
                                let hb = encode_heartbeat_reply();
                                let c = client.lock().unwrap();
                                if let Some(act) = c.send_data(&hb) {
                                    let _ = udp.send_to(&act.data, &act.target_ip, act.target_port);
                                }
                                continue;
                            }
                        }
                    }

                    // 数据帧 → 传递给主线程
                    if let Some(frame) = P2pClient::parse_received(&payload) {
                        if frame.frame_type == TYPE_DATA {
                            let text = String::from_utf8_lossy(&frame.payload).to_string();
                            let _ = data_tx.send(text);
                        }
                    }
                }
            }
        });
    }

    fn send_text(&self, text: &str) -> Result<(), String> {
        let act = {
            let c = self.client.lock().unwrap();
            c.send_text(text).ok_or_else(|| "连接未建立".to_string())?
        };
        self.udp
            .send_to(&act.data, &act.target_ip, act.target_port)
            .map_err(|e| format!("发送失败: {}", e))
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let mut c = self.client.lock().unwrap();
        c.stop_ice();
    }
}

// ── 主流程 ───────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let config_path = args.get(1).map(|s| s.as_str()).unwrap_or("config.json");

    let config = read_config(config_path);

    // 解密编译时嵌入的 Token
    let token = embedded_token::decrypt().unwrap_or_else(|| {
        eprintln!("嵌入 Token 解密失败");
        std::process::exit(1);
    });
    let http = SyncHttpTransport::new();

    // 初始化 P2pClient（仅用于 register/query）
    let mut client = P2pClient::new();
    client.init(Config {
        ids_url: config.ids_url.clone(),
        nat_url: config.nat_url.clone(),
        app_id: config.app_id.clone(),
    });

    // Step 1: 注册 IDS
    print!("[1/3] 注册 IDS...");
    io::stdout().flush().ok();
    client
        .register_ids(&http, &config.user_id, &config.odid, "")
        .map_err(|e| format!("失败: {}", e))
        .unwrap();
    println!(" 成功");

    // Step 2: 查询 IDS
    print!("[2/3] 查询 IDS...");
    io::stdout().flush().ok();
    let peer = client
        .query_ids(&http, &config.user_id)
        .map_err(|e| format!("失败: {}", e))
        .unwrap();
    if peer.token.is_empty() {
        eprintln!(" 未找到对端 service 记录");
        std::process::exit(1);
    }
    println!(" 找到对端: {}", peer.token);

    // Step 3: 一键建链
    print!("[3/3] 建立 P2P 连接...");
    io::stdout().flush().ok();

    let (data_tx, data_rx) = mpsc::channel::<String>();
    let (state_tx, state_rx) = mpsc::channel::<IceState>();

    let runner = IceRunner::connect(&token, &config, &peer.token, data_tx, state_tx)
        .unwrap_or_else(|e| {
            eprintln!(" 失败: {}", e);
            std::process::exit(1);
        });

    println!(" 等待 ICE 协商...");

    // 等待 ICE 完成
    let connected = loop {
        match state_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(state) => {
                println!("[ICE] {}", state);
                if state == IceState::Completed || state == IceState::Connected {
                    break true;
                }
                if state == IceState::Failed {
                    break false;
                }
            }
            Err(_) => {
                eprintln!("[ICE] 协商超时");
                break false;
            }
        }
    };

    if !connected {
        runner.stop();
        std::process::exit(1);
    }

    // 聊天循环
    println!("已连接，输入消息按回车发送，/quit 退出\n");

    let running = Arc::new(AtomicBool::new(true));

    // stdin 线程
    let (stdin_tx, stdin_rx): (Sender<String>, Receiver<String>) = mpsc::channel();
    {
        let running_clone = running.clone();
        thread::spawn(move || {
            let stdin = io::stdin();
            for line in stdin.lock().lines() {
                match line {
                    Ok(text) => {
                        if stdin_tx.send(text).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            running_clone.store(false, Ordering::Relaxed);
        });
    }

    while running.load(Ordering::Relaxed) {
        // 用户输入
        while let Ok(text) = stdin_rx.try_recv() {
            match text.as_str() {
                "/quit" => {
                    println!("正在退出...");
                    running.store(false, Ordering::Relaxed);
                }
                "/status" => {
                    let state = runner.client.lock().unwrap().ice_state();
                    println!("[状态] ICE: {}", state.map(|s| s.to_string()).unwrap_or("无".into()));
                }
                _ => match runner.send_text(&text) {
                    Ok(()) => println!("[我] {}", text),
                    Err(e) => eprintln!("[错误] {}", e),
                },
            }
        }

        // 接收对端消息
        while let Ok(text) = data_rx.try_recv() {
            println!("[对端] {}", text);
        }

        thread::sleep(Duration::from_millis(100));
    }

    runner.stop();
    println!("已退出");
}
