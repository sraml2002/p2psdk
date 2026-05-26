//! P2P SDK Rust Demo App
//!
//! 使用 P2pClient 公开接口：init → on_state_change → on_data
//! → register_ids → query_ids → connect → send_text → close

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use p2p_sdk::{Config, P2pClient, IceState};
use p2p_tokio::SyncHttpTransport;

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

// ── 主流程 ───────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let config_path = args.get(1).map(|s| s.as_str()).unwrap_or("config.json");

    let config = read_config(config_path);

    // 初始化 P2pClient
    let mut client = P2pClient::new();
    client.init(Config {
        ids_url: config.ids_url.clone(),
        nat_url: config.nat_url.clone(),
    });

    // 注册回调
    let (state_tx, state_rx) = mpsc::channel::<IceState>();
    let (data_tx, data_rx) = mpsc::channel::<String>();

    client.on_state_change(Box::new(move |state: IceState| {
        let _ = state_tx.send(state);
    }));

    client.on_data(Box::new(move |payload: Vec<u8>| {
        let text = String::from_utf8_lossy(&payload).to_string();
        let _ = data_tx.send(text);
    }));

    let http = SyncHttpTransport::new();

    // Step 1: 注册 IDS
    print!("[1/3] 注册 IDS...");
    io::stdout().flush().ok();
    client
        .register_ids(&http, &config.app_id, &config.user_id, &config.odid, "")
        .map_err(|e| format!("失败: {}", e))
        .unwrap();
    println!(" 成功");

    // Step 2: 查询 IDS
    print!("[2/3] 查询 IDS...");
    io::stdout().flush().ok();
    let peer = client
        .query_ids(&http, &config.app_id, &config.user_id)
        .map_err(|e| format!("失败: {}", e))
        .unwrap();
    if peer.token.is_empty() {
        eprintln!(" 未找到对端 service 记录");
        std::process::exit(1);
    }
    println!(" 找到对端: {}", peer.token);

    // Step 3: 一键建链（token 由 SDK 内部自动生成）
    print!("[3/3] 建立 P2P 连接...");
    io::stdout().flush().ok();
    client.connect(&peer.token, &config.odid, 30)
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
                match state {
                    IceState::Completed | IceState::Connected => break true,
                    IceState::Failed => break false,
                    IceState::Disconnected => {
                        eprintln!("[ICE] 连接断开");
                        break false;
                    }
                    _ => {}
                }
            }
            Err(_) => {
                eprintln!("[ICE] 协商超时");
                break false;
            }
        }
    };

    if !connected {
        let _ = client.close();
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
                    let state = client.ice_state();
                    println!("[状态] ICE: {}", state.map(|s| s.to_string()).unwrap_or("无".into()));
                }
                _ => match client.send_text(&text) {
                    Ok(()) => println!("[我] {}", text),
                    Err(e) => eprintln!("[错误] {}", e),
                },
            }
        }

        // 接收对端消息
        while let Ok(text) = data_rx.try_recv() {
            println!("[对端] {}", text);
        }

        // 检测断连
        if let Ok(state) = state_rx.try_recv() {
            println!("[ICE] {}", state);
            if state == IceState::Disconnected || state == IceState::Failed {
                eprintln!("[ICE] 连接断开");
                running.store(false, Ordering::Relaxed);
            }
        }

        thread::sleep(Duration::from_millis(100));
    }

    let _ = client.close();
    println!("已退出");
}
