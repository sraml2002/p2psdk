//! P2P SDK Rust Demo App
//!
//! 通过 C FFI 动态加载 libppsdk.so，复现 ArkTS App (IdsPage2) 的一键连接 + 聊天功能。
//! Token 嵌入在 libppsdk.so 内部，本程序不处理 token。

use std::ffi::{c_char, c_int, CStr, CString};
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use libloading::Library;

// ── C FFI 类型 ─────────────────────────────────────────────────────

type CbState = extern "C" fn(*const c_char);
type CbData = extern "C" fn(*const u8, usize);
type CbLog = extern "C" fn(*const c_char);

type FnRegisterCallbacks = extern "C" fn(CbState, CbData, CbLog) -> c_int;
type FnInit = extern "C" fn(*const c_char) -> c_int;
type FnRegisterIds = extern "C" fn(*const c_char, *const c_char, *const c_char, *const c_char)
    -> *mut c_char;
type FnQueryIds = extern "C" fn(*const c_char, *const c_char) -> *mut c_char;
type FnConnect = extern "C" fn(*const c_char, *const c_char) -> c_int;
type FnSendText = extern "C" fn(*const c_char) -> c_int;
type FnClose = extern "C" fn() -> c_int;
type FnFreeString = extern "C" fn(*mut c_char);

// ── 全局状态（C 回调写入） ─────────────────────────────────────────

static ICE_STATE: Mutex<Option<String>> = Mutex::new(None);
static DATA_TX: Mutex<Option<mpsc::Sender<String>>> = Mutex::new(None);

extern "C" fn on_state(state: *const c_char) {
    let s = unsafe { CStr::from_ptr(state) }
        .to_str()
        .unwrap_or("")
        .to_string();
    eprintln!("[ICE] {}", s);
    *ICE_STATE.lock().unwrap() = Some(s);
}

extern "C" fn on_data(data: *const u8, len: usize) {
    if len <= 8 {
        return;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len) };
    // 帧格式: [4B payload len BE][4B frame type BE][payload...]
    // 心跳帧由 SDK 内部处理（heartbeat_interval=30），此处仅收到 TYPE_DATA
    let text = String::from_utf8_lossy(&slice[8..]).to_string();
    if let Some(tx) = DATA_TX.lock().unwrap().as_ref() {
        let _ = tx.send(text);
    }
}

extern "C" fn on_log(msg: *const c_char) {
    let s = unsafe { CStr::from_ptr(msg) }
        .to_str()
        .unwrap_or("");
    eprintln!("[SDK] {}", s);
}

// ── P2pApi ─────────────────────────────────────────────────────────

struct P2pApi {
    register_callbacks: FnRegisterCallbacks,
    init: FnInit,
    register_ids: FnRegisterIds,
    query_ids: FnQueryIds,
    connect: FnConnect,
    send_text: FnSendText,
    close: FnClose,
    free_string: FnFreeString,
}

fn load_api(path: &str) -> P2pApi {
    let lib = unsafe {
        Library::new(path).unwrap_or_else(|e| {
            eprintln!("加载 {} 失败: {e}", path);
            eprintln!("用法: app-test-rust <config.json> [libppsdk.so]");
            std::process::exit(1);
        })
    };
    let api = unsafe {
        P2pApi {
            register_callbacks: *lib
                .get(b"ppsdk_register_callbacks\0")
                .expect("ppsdk_register_callbacks 未找到"),
            init: *lib.get(b"ppsdk_init\0").expect("ppsdk_init 未找到"),
            register_ids: *lib
                .get(b"ppsdk_register_ids\0")
                .expect("ppsdk_register_ids 未找到"),
            query_ids: *lib
                .get(b"ppsdk_query_ids\0")
                .expect("ppsdk_query_ids 未找到"),
            connect: *lib.get(b"ppsdk_connect\0").expect("ppsdk_connect 未找到"),
            send_text: *lib
                .get(b"ppsdk_send_text\0")
                .expect("ppsdk_send_text 未找到"),
            close: *lib.get(b"ppsdk_close\0").expect("ppsdk_close 未找到"),
            free_string: *lib
                .get(b"ppsdk_free_string\0")
                .expect("ppsdk_free_string 未找到"),
        }
    };
    // 防止 Library 被 drop 导致 .so 卸载
    std::mem::forget(lib);
    api
}

// 辅助: 调用返回 *mut c_char 的函数，转换为 String 后自动释放
fn call_string_fn(f: impl FnOnce() -> *mut c_char, api: &P2pApi) -> Option<String> {
    let ptr = f();
    if ptr.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(|s| s.to_string());
    (api.free_string)(ptr);
    s
}

fn is_connected() -> bool {
    ICE_STATE
        .lock()
        .unwrap()
        .as_deref()
        == Some("COMPLETED")
}

// ── Main ───────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let config_path = args.get(1).map(|s| s.as_str()).unwrap_or("config.json");
    let lib_path = {
        let raw = args.get(2).map(|s| s.as_str()).unwrap_or("libppsdk.so");
        // dlopen 不搜索当前目录，补 ./ 前缀确保可从当前目录加载
        if !raw.starts_with('/') && !raw.starts_with("./") {
            format!("./{raw}")
        } else {
            raw.to_string()
        }
    };

    // 1. 加载 libppsdk.so
    let api = load_api(&lib_path);

    // 2. 注册 C 回调
    (api.register_callbacks)(on_state, on_data, on_log);

    // 3. 读取配置
    let config_text = match std::fs::read_to_string(config_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("读取配置失败: {e}");
            std::process::exit(1);
        }
    };
    let config: serde_json::Value =
        serde_json::from_str(&config_text).unwrap_or_else(|e| {
            eprintln!("解析配置失败: {e}");
            std::process::exit(1);
        });

    // 4. init
    let config_c = CString::new(config_text.clone()).unwrap();
    let code = (api.init)(config_c.as_ptr());
    if code != 0 {
        eprintln!("init 失败: {code}");
        std::process::exit(1);
    }
    println!("[Demo] SDK 已初始化");

    let app_id = config["appId"].as_str().unwrap_or("");
    let user_id = config["userId"].as_str().unwrap_or("");
    let odid = config["odid"].as_str().unwrap_or("");

    // 5. registerIds
    let app_id_c = CString::new(app_id).unwrap();
    let user_id_c = CString::new(user_id).unwrap();
    let odid_c = CString::new(odid).unwrap();
    let empty_c = CString::new("").unwrap();

    print!("[1/3] 注册 IDS...");
    io::stdout().flush().ok();
    let resp = call_string_fn(
        || {
            (api.register_ids)(
                app_id_c.as_ptr(),
                user_id_c.as_ptr(),
                odid_c.as_ptr(),
                empty_c.as_ptr(),
            )
        },
        &api,
    );
    match resp {
        Some(ref s) if !s.contains("error") => println!(" 成功"),
        Some(ref s) => {
            println!(" 失败: {}", s);
            std::process::exit(1);
        }
        None => {
            println!(" 失败");
            std::process::exit(1);
        }
    }

    // 6. queryIds
    print!("[2/3] 查询 IDS...");
    io::stdout().flush().ok();
    let resp = call_string_fn(
        || (api.query_ids)(app_id_c.as_ptr(), user_id_c.as_ptr()),
        &api,
    );
    let peer_id = match resp {
        Some(ref s) => {
            let json: serde_json::Value = serde_json::from_str(s).unwrap_or_default();
            let data = json.get("data").and_then(|d| d.as_array());
            let token = data
                .and_then(|arr| {
                    arr.iter()
                        .find(|r| r.get("type").and_then(|v| v.as_str()) == Some("service"))
                        .and_then(|r| r.get("token").and_then(|v| v.as_str()))
                })
                .unwrap_or("");
            if token.is_empty() {
                println!(" 失败: 未获取到对端地址");
                std::process::exit(1);
            }
            println!(" 成功 (对端: {})", token);
            token.to_string()
        }
        None => {
            println!(" 失败");
            std::process::exit(1);
        }
    };

    // 7. connect
    println!("[3/3] 建立 P2P 连接...");
    let peer_id_c = CString::new(peer_id).unwrap();
    let code = (api.connect)(peer_id_c.as_ptr(), odid_c.as_ptr());
    if code != 0 {
        eprintln!("连接失败: {code}");
        std::process::exit(1);
    }

    // 8. 聊天事件循环
    chat_loop(&api);
}

fn chat_loop(api: &P2pApi) {
    let (data_tx, data_rx) = mpsc::channel::<String>();
    *DATA_TX.lock().unwrap() = Some(data_tx);

    let (stdin_tx, stdin_rx) = mpsc::channel::<String>();
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    // stdin 读取线程
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

    println!("[Demo] 连接中...");
    println!("[Demo] 输入消息按回车发送, /quit 退出, /status 查看状态");

    while running.load(Ordering::Relaxed) {
        // 用户输入
        while let Ok(text) = stdin_rx.try_recv() {
            match text.as_str() {
                "/quit" => {
                    println!("[Demo] 正在退出...");
                    running.store(false, Ordering::Relaxed);
                }
                "/status" => {
                    let state = ICE_STATE
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or_else(|| "无".into());
                    println!("[状态] ICE: {}", state);
                }
                _ => {
                    if is_connected() {
                        let text_c = CString::new(text.clone()).unwrap();
                        let code = (api.send_text)(text_c.as_ptr());
                        if code == 0 {
                            println!("[我] {}", text);
                        } else {
                            eprintln!("[错误] 发送失败: {}", code);
                        }
                    } else {
                        eprintln!("[警告] 连接未建立, 无法发送");
                    }
                }
            }
        }

        // 接收对端消息
        while let Ok(text) = data_rx.try_recv() {
            println!("[对端] {}", text);
        }

        thread::sleep(Duration::from_millis(100));
    }

    (api.close)();
    println!("[Demo] 已退出");
}
