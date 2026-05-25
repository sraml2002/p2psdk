# P2P SDK (Rust)

P2P SDK 的全 Rust 实现，用于 HarmonyOS 设备端 P2P 通信。通过 raw NAPI 桥接导出为 `libppsdk.so`，供 ArkTS 应用直接调用；同时导出 C ABI，支持 Rust 程序通过 dlopen 集成。

## 项目结构

```
p2psdk_rust/
├── build-so.sh                    # 构建 libppsdk.so：cargo build + 复制 .so
├── build-rust-demo.sh             # 构建 Rust Demo App（.so + 二进制）
├── docs/                          # 文档
│   ├── p2psdk-rust-system-design.md
│   └── p2psdk-rust-api.md
├── sdk/                           # Rust 源码（Cargo workspace）
│   ├── Cargo.toml / Cargo.lock
│   ├── .cargo/config.toml         # ohos linker 配置
│   └── crates/
│       ├── p2p-core/              # 协议核心：ICE/STUN/TURN/SDP/Frame
│       ├── p2p-io/                # 平台 I/O traits（UDP/HTTP/WS/DTLS）
│       ├── p2p-tokio/             # 同步 I/O 实现（std::net / reqwest / tungstenite）
│       ├── p2p-sdk/               # 高层 SDK 门面（P2pClient / ConnectorClient）
│       ├── p2p-napi/              # NAPI 桥接层 → libppsdk.so（含 C ABI 导出）
│       │   └── src/c_export.rs    # C ABI 导出：ppsdk_init / ppsdk_connect 等
│       └── dimpl/                 # 第三方依赖（DER 编码）
├── app-test-rust/                 # Rust Demo App — 命令行 P2P 聊天（非交付件）
│   ├── Cargo.toml                 # 独立 crate，仅依赖 libloading / serde
│   ├── config.example.json        # 配置样例（脱敏）
│   └── src/main.rs                # dlopen 加载 libppsdk.so，C FFI 调用
└── app-test-hmos/                 # HarmonyOS 测试应用（非交付件）
    ├── entry/
    │   ├── hvigorfile.ts          # 自定义 hvigor 插件，构建时自动调用 build-so.sh
    │   ├── libs/arm64-v8a/        # 编译产物 libppsdk.so（gitignore）
    │   └── src/main/
    │       ├── cpp/               # 原生模块配置
    │       │   ├── CMakeLists.txt # 注册预构建 .so，使构建系统发现类型声明
    │       │   └── types/libppsdk/
    │       │       └── index.d.ts # 类型声明文件
    │       ├── ets/
    │       │   └── pages/
    │       │       ├── Index.ets  # Connector 信令模式页面
    │       │       └── IdsPage.ets # IDS + SDP 模式页面
    │       └── resources/
    └── build-profile.json5
```

## Rust Crate 依赖关系

```
p2p-napi (→ libppsdk.so)
  ├── p2p-sdk (SDK 门面)
  │     ├── p2p-core (协议核心)
  │     │     └── dimpl (DER 编码)
  │     └── p2p-io (I/O traits)
  ├── p2p-tokio (同步 I/O)
  │     └── p2p-io (I/O traits)
  └── p2p-core (直接使用 STUN/SDP/Frame)
```

| Crate | 职责 |
|-------|------|
| **p2p-core** | Sans-IO 协议核心：ICE Agent (RFC 8445)、STUN/TURN 编解码、SDP 生成/解析、P2P 数据帧 |
| **p2p-io** | 平台抽象 traits：`UdpTransport`、`HttpTransport`、`SignalingTransport`、`Platform` |
| **p2p-tokio** | 基于标准库的同步实现：`std::net::UdpSocket`、`reqwest::blocking`、`tungstenite` WebSocket |
| **p2p-sdk** | 高层 SDK 门面：`P2pClient` 统一编排 ICE/STUN/TURN/IDS/Connector 全流程 |
| **p2p-napi** | Raw NAPI FFI 桥接：通过 `.init_array` 自动注册，ThreadsafeFunction 回调到 ArkTS；同时导出 C ABI 供非 NAPI 消费者使用 |

## 开发环境

### 前置条件

| 工具 | 版本要求 | 说明 |
|------|---------|------|
| Rust | stable | `rustup` 安装 |
| OHOS NDK | API 20+ | HarmonyOS OpenHarmony SDK，包含 `aarch64-linux-ohos-clang` 链接器 |
| DevEco Studio | 5.0+ | HarmonyOS 应用开发 IDE |

### 配置 NDK 路径

Rust 交叉编译到 HarmonyOS 需要 OHOS NDK 提供的 Clang 链接器（用于链接阶段和 C 依赖的交叉编译）。NDK 路径配置在 `sdk/.cargo/config.toml` 中，默认值为：

```
~/Library/OpenHarmony/Sdk/20/native/llvm/bin/aarch64-unknown-linux-ohos-clang
```

**编译前必须确认路径正确**。如果 OHOS NDK 安装位置或 API 版本不同，需修改 `sdk/.cargo/config.toml` 中所有 `/Users/sram/Library/OpenHarmony/Sdk/20` 为实际路径。

## 构建

构建前需准备 NAT 服务认证所需的 JWT Token，传递给 `build-so.sh`。两种可选方式：

```bash
# 方式一：命令行 --token 参数（推荐）
bash build-so.sh --token "eyJhbGciOiJFUzI1NiIs..."

# 方式二：手动创建 sdk/crates/p2p-napi/build.jwt.nogit，写入 Token 内容，然后构建
echo "eyJhbGciOiJFUzI1NiIs..." > sdk/crates/p2p-napi/build.jwt.nogit
bash build-so.sh
```

> 方式二适用于 DevEco Studio 构建场景（无法传递命令行参数，需提前准备好文件）。

### 方式一：DevEco Studio 构建

在 DevEco Studio 中打开 `app-test-hmos/` 项目，手动点击 Build > Build Hap(s)/APP(s)。构建过程中，自定义 hvigor 插件（`entry/hvigorfile.ts`）会自动调用 `build-so.sh` 完成 Rust 编译和 .so 复制。

### 方式二：手动编译 .so

如果只需要编译 `libppsdk.so`（不构建完整 HAP），可以从项目根目录执行：

```bash
bash build-so.sh --token "<JWT_TOKEN>"
```

## 集成 SDK

### ArkTS 应用集成

第三方 HarmonyOS 应用集成 P2P SDK 只需 **2 个文件**：

| 文件 | 放置位置 | 说明 |
|------|---------|------|
| `libppsdk.so` | `entry/libs/arm64-v8a/` | Rust 编译产物 |
| `index.d.ts` | `entry/src/main/cpp/types/libppsdk/` | 类型声明文件 |

#### 集成步骤

**1. 复制文件**

将 `libppsdk.so` 放入 `entry/libs/arm64-v8a/`，将类型声明文件放入 `entry/src/main/cpp/types/libppsdk/index.d.ts`。

**2. 创建 CMakeLists.txt**

在 `entry/src/main/cpp/` 下创建 `CMakeLists.txt`，使 HarmonyOS 构建系统发现类型声明：

```cmake
cmake_minimum_required(VERSION 3.5.1)
project(ppsdk)

add_library(ppsdk SHARED IMPORTED GLOBAL)
set_target_properties(ppsdk PROPERTIES
    IMPORTED_LOCATION "${CMAKE_CURRENT_SOURCE_DIR}/../libs/${OHOS_ARCH}/libppsdk.so"
)
```

**3. 在 ArkTS 中调用**

```typescript
import ppsdk from 'libppsdk.so'

// 初始化
ppsdk.init(JSON.stringify({
  idsUrl: 'ids-host:port',
  natUrl: 'https://natservice-drcn.platform.dbankcloud.cn:443/trs/v1/route',
  appId: 'YourAppId',
}))

// 注册回调
ppsdk.onStateChange((state: string): void => { /* ICE 状态变化 */ })
ppsdk.onDataReceived((data: ArrayBuffer): void => { /* 接收数据 */ })
ppsdk.onLog((msg: string): void => { /* 调试日志 */ })

// 生成 Token（构建时加密嵌入，运行时解密）
const token: string = ppsdk.generateToken()

// 候选收集（同步阻塞）
const info: CandidateInfo = ppsdk.gatherCandidates(token)

// 连接
ppsdk.connect(peerId, odid)

// 发送数据
const frame: ArrayBuffer = ppsdk.encodeDataFrame('Hello P2P')
ppsdk.send(frame)

// 关闭
ppsdk.close()
```

> **注意**：ArkTS 严格模式下，`ppsdk` 的返回值为 `any` 类型，所有接收返回值的变量必须显式标注类型（如 `const info: CandidateInfo = ppsdk.gatherCandidates(token)`），否则触发 `arkts-no-any-unknown` 编译错误。

#### 需要的权限

```json
"requestPermissions": [
  { "name": "ohos.permission.INTERNET" },
  { "name": "ohos.permission.GET_WIFI_INFO" },
  { "name": "ohos.permission.DEVICE_INFO" }
]
```

### Rust 应用集成

Rust 程序通过 C FFI 动态加载 `libppsdk.so`，调用导出的 C ABI 函数。Token 在构建时加密嵌入 `libppsdk.so` 内部，Rust 程序不需要单独处理。

#### 所需文件

| 文件 | 说明 |
|------|------|
| `libppsdk.so` | 包含 NAPI + C ABI 导出的共享库 |

#### 配置文件

配置文件为 JSON 格式，包含 IDS 服务和 NAT 服务的连接信息：

```json
{
  "idsUrl": "{ids服务url}",
  "natUrl": "{nat服务url}",
  "appId": "{宿主AppId}",
  "userId": "{宿主用户ID}",
  "odid": "{宿主App ODID}"
}
```

#### Cargo.toml

```toml
[dependencies]
libloading = "0.8"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

#### 集成步骤

**1. 加载 libppsdk.so**

```rust
use libloading::Library;
use std::ffi::{c_char, c_int, CStr, CString};

let lib = unsafe { Library::new("./libppsdk.so").expect("加载失败") };
let ppsdk_init: extern "C" fn(*const c_char) -> c_int =
    unsafe { *lib.get(b"ppsdk_init\0").unwrap() };
// ... 获取其他符号
std::mem::forget(lib); // 防止 .so 被卸载
```

**2. 注册回调并初始化**

```rust
extern "C" fn on_state(state: *const c_char) {
    let s = unsafe { CStr::from_ptr(state) }.to_str().unwrap_or("");
    eprintln!("[ICE] {}", s);
}

extern "C" fn on_data(data: *const u8, len: usize) {
    // 帧格式: [4B payload len][4B frame type][payload...]
    if len > 8 {
        let text = String::from_utf8_lossy(
            unsafe { std::slice::from_raw_parts(data, len) }.get(8..).unwrap_or(&[])
        );
        println!("[对端] {}", text);
    }
}

extern "C" fn on_log(msg: *const c_char) {
    eprintln!("[SDK] {}", unsafe { CStr::from_ptr(msg) }.to_str().unwrap_or(""));
}

// 注册回调（必须在 init 之前调用）
ppsdk_register_callbacks(on_state, on_data, on_log);

// 初始化，传入完整配置 JSON
let config_c = CString::new(config_json).unwrap();
ppsdk_init(config_c.as_ptr());
```

**3. 注册、查询、连接**

```rust
// 注册设备到 IDS
ppsdk_register_ids(app_id, user_id, odid, "");

// 查询对端 IDS（返回 JSON，需调用 ppsdk_free_string 释放）
let result = ppsdk_query_ids(app_id, user_id);
// 从返回 JSON 的 data 数组中找到 type="service" 的记录，取其 token 作为 peer_id
let peer_id = parse_service_token(result);
ppsdk_free_string(result);

// 一键连接（内部自动完成: token 解密 → NAT 路由 → 候选收集 → SDP 协商）
ppsdk_connect(peer_id, odid);
```

**4. 发送数据与关闭**

```rust
// 发送文本
let text_c = CString::new("Hello P2P").unwrap();
ppsdk_send_text(text_c.as_ptr());

// 关闭连接
ppsdk_close();
```

#### C ABI 接口参考

`libppsdk.so` 除 NAPI 接口外，还导出以下 C ABI 函数：

| 函数 | 参数 | 返回值 | 说明 |
|------|------|--------|------|
| `ppsdk_register_callbacks` | `on_state, on_data, on_log` | `c_int` | 注册 C 回调函数指针，必须在 init 前调用 |
| `ppsdk_init` | `config_json: *const c_char` | `c_int` | 初始化 SDK，config_json 含 idsUrl/natUrl |
| `ppsdk_register_ids` | `app_id, user_id, odid, push_token` | `*mut c_char` | 注册设备到 IDS，返回 JSON（需 free） |
| `ppsdk_query_ids` | `app_id, user_id` | `*mut c_char` | 查询 IDS 记录，返回 JSON（需 free） |
| `ppsdk_connect` | `peer_id, odid` | `c_int` | 一键连接：token + 候选收集 + SDP 协商 |
| `ppsdk_send_text` | `text: *const c_char` | `c_int` | 发送文本消息 |
| `ppsdk_send` | `data: *const u8, len: usize` | `c_int` | 发送二进制数据 |
| `ppsdk_close` | 无 | `c_int` | 关闭连接，释放资源 |
| `ppsdk_free_string` | `s: *mut c_char` | 无 | 释放 register_ids/query_ids 返回的字符串 |

**回调函数签名：**

| 回调 | 签名 | 触发时机 |
|------|------|----------|
| `on_state` | `extern "C" fn(*const c_char)` | ICE 状态变化（CONNECTING/CONNECTED/COMPLETED/FAILED） |
| `on_data` | `extern "C" fn(*const u8, usize)` | 收到对端数据帧（心跳帧已由 SDK 自动处理） |
| `on_log` | `extern "C" fn(*const c_char)` | SDK 内部日志 |

## 运行测试应用

### ArkTS 应用 (app-test-hmos)

1. 在 DevEco Studio 中打开 `app-test-hmos/` 项目
2. 连接 HarmonyOS 设备或启动模拟器
3. 点击 Run 运行应用

测试应用提供两个 P2P 演示页面：
- **Index**：Connector 信令模式（开发调试用）
- **IdsPage**：IDS + SDP 模式（设备↔云服务场景）

### Rust Demo App (app-test-rust)

Rust 命令行 P2P 聊天程序，通过 C FFI 动态加载 `libppsdk.so`，复现 ArkTS 应用的一键连接 + 聊天功能。

**构建：**

```bash
# 一键构建（libppsdk.so + app-test-rust）
bash build-rust-demo.sh

# 仅重建 app-test-rust，复用已有 libppsdk.so
bash build-rust-demo.sh --no-so
```

产出目录：`app-test-rust/dist/`，包含 `libppsdk.so`、`app-test-rust`、`config.example.json`。

**运行：**

```bash
# 1. 推送到设备
hdc file send libppsdk.so /data/local/tmp/
hdc file send app-test-rust /data/local/tmp/
hdc file send config.json /data/local/tmp/

# 2. 准备配置文件（复制 config.example.json 为 config.json，填入真实服务地址）
# 3. 运行
hdc shell "cd /data/local/tmp && chmod +x app-test-rust && ./app-test-rust config.json ./libppsdk.so"
```

**交互命令：**
- 输入文本按回车发送消息
- `/status` 查看 ICE 连接状态
- `/quit` 退出

## 技术选型

| 决策 | 原因 |
|------|------|
| **全 Rust 协议实现** | 替代原 ArkTS + C++ 方案，统一技术栈，避免跨语言桥接复杂性 |
| **Raw NAPI (.init_array)** | 不依赖 napi-ohos crate，直接用 NAPI C FFI，减少依赖和编译开销 |
| **Sans-IO 架构 (p2p-core)** | 协议逻辑与 I/O 完全分离，可在桌面端测试，便于移植到其他平台 |
| **同步阻塞 I/O (p2p-tokio)** | 匹配 NAPI 同步调用模型，避免异步运行时与 NAPI 事件循环冲突 |
| **ThreadsafeFunction 回调** | Rust 后台线程通过 TSFN 安全回调到 ArkTS 主线程 |
| **全局单例 (Arc&lt;Mutex&lt;Inner&gt;&gt;)** | NAPI 模块级单例，所有调用共享同一状态，线程安全 |
| **ArrayBuffer 直传** | send 直接接收 ArrayBuffer，无需 hex 编码中转 |
| **NAPI Object 返回** | gatherCandidates/registerIds/queryIds 返回 NAPI Object，无需 JSON 解析 |

## 相关文档

- [系统设计文档](docs/p2psdk-rust-system-design.md) — 架构、模块划分、数据流
- [API 参考文档](docs/p2psdk-rust-api.md) — NAPI 导出接口 + ArkTS 调用方式
