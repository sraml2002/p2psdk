# P2P SDK (Rust)

P2P SDK 的全 Rust 实现，用于 HarmonyOS 设备端 P2P 通信。提供两种集成方式：ArkTS 应用通过 `libppsdk.so` 直接调用，Rust 程序通过 SDK crate 编译为单一可执行文件。支持一键建链（init → registerIds → queryIds → p2pConnect），内部自动完成 NAT 路由、候选收集、SDP 协商、ICE 建立。

## 项目结构

```
p2psdk_rust/
├── build-arkts-napi-so.sh          # 构建 libppsdk.so：cargo build + 复制 .so
├── build-rust-demo.sh             # 构建 Rust Demo App（单一可执行文件）
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
│   ├── Cargo.toml                 # path 依赖 SDK crate，编译为单一可执行文件
│   ├── config.example.json        # 配置样例（脱敏）
│   └── src/main.rs                # 直接调用 p2p-sdk crate，一键建链 + 聊天
└── app-test-hmos/                 # HarmonyOS 测试应用（非交付件）
    ├── entry/
    │   ├── hvigorfile.ts          # 自定义 hvigor 插件，构建时自动调用 build-arkts-napi-so.sh
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

---

## 1. Rust应用集成

命令行 P2P 建链程序，直接基于 SDK crate 源码编译为可执行文件，实现与对端建链 + 聊天功能，通过配置文件设定对端 URL 信息。

### 构建配置

线下获取访问 NAT 服务的 JWT Token，保存为文件，并将文件绝对路径写入项目根目录的 `build.jwt.path`：

```
#NAT服务认证Token文件路径（线下申请）
/path/to/your/jwt-token
```

### 构建

```bash
# 构建 OHOS 版本（默认，交叉编译到 aarch64-unknown-linux-ohos）
bash build-rust-demo.sh

# 构建 macOS 版本
bash build-rust-demo.sh --target mac
```

产出目录：`app-test-rust/dist/`，包含 `app-test-rust` 可执行文件和 `config.example.json`。

### 运行配置

```json
{
  "idsUrl": "{IDS服务的URL}",
  "natUrl": "{NAT服务的URL}",
  "appId": "{宿主AppId}",
  "userId": "{宿主App的用户ID}",
  "odid": "{宿主App ODID}"
}
```

运行前复制 `config.example.json` 为 `config.json`，填入真实服务地址。

### 运行

**OHOS 设备：**

```bash
hdc file send app-test-rust /data/local/tmp/
hdc file send config.json /data/local/tmp/
hdc shell chmod +x /data/local/tmp/app-test-rust
hdc shell "cd /data/local/tmp && ./app-test-rust config.json"
```

**macOS：**

```bash
cd app-test-rust/dist
cp config.example.json config.json  # 编辑填入真实配置
./app-test-rust config.json
```

### 运行流程

一键建链流程：`init → registerIds → queryIds → p2pConnect`，其中 `p2pConnect` 内部自动完成 NAT 路由解析 → 候选收集 → SDP 协商 → ICE 建立。

```
[1/3] 注册 IDS... 成功
[2/3] 查询 IDS... 找到对端: 81.71.29.250:34848
[3/3] 建立 P2P 连接... 等待 ICE 协商...
[ICE] CONNECTING
[ICE] CONNECTED
已连接，输入消息按回车发送，/quit 退出
```

### 交互命令

- 输入文本按回车发送消息
- `/status` 查看 ICE 连接状态
- `/quit` 退出

### 代码示例

```rust
use p2p_sdk::P2pClient;

// 初始化（配置从 config.json 读取）
let mut client = P2pClient::new();
client.init(config);

// 注册 + 查询 IDS
client.register_ids(&http, &user_id, &odid, "")?;
let peer = client.query_ids(&http, &user_id)?;

// 一键建链（内部自动完成 NAT 路由 → 候选收集 → SDP 协商 → ICE 建立）
let runner = IceRunner::connect(&token, &config, &peer.token)?;

// 发送文本
runner.send_text("Hello P2P")?;

// 接收对端消息（通过 channel 异步回调）
if let Ok(text) = data_rx.recv_timeout(Duration::from_secs(5)) {
    println!("[对端] {}", text);
}

// 断开连接
runner.stop();
```

---

## 2. SDK 概览

### Rust Crate 依赖关系

```
p2p-napi (→ libppsdk.so)
  ├── p2p-sdk (SDK 门面)
  │     ├── p2p-core (协议核心)
  │     │     └── dimpl (DER 编码)
  │     └── p2p-io (I/O traits)
  ├── p2p-tokio (同步 I/O)
  │     └── p2p-io (I/O traits)
  └── p2p-core (直接使用 STUN/SDP/Frame)

app-test-rust (→ 单一可执行文件)
  ├── p2p-sdk (SDK 门面)
  ├── p2p-tokio (同步 I/O)
  ├── p2p-core (协议核心)
  └── p2p-io (I/O traits)
```

| Crate | 职责 |
|-------|------|
| **p2p-core** | Sans-IO 协议核心：ICE Agent (RFC 8445)、STUN/TURN 编解码、SDP 生成/解析、P2P 数据帧 |
| **p2p-io** | 平台抽象 traits：`UdpTransport`、`HttpTransport`、`SignalingTransport`、`Platform` |
| **p2p-tokio** | 基于标准库的同步实现：`std::net::UdpSocket`、`reqwest::blocking`、`tungstenite` WebSocket |
| **p2p-sdk** | 高层 SDK 门面：`P2pClient` 统一编排 ICE/STUN/TURN/IDS/Connector 全流程 |
| **p2p-napi** | Raw NAPI FFI 桥接：通过 `.init_array` 自动注册，ThreadsafeFunction 回调到 ArkTS；同时导出 C ABI 供非 NAPI 消费者使用 |

### 开发环境

| 工具 | 版本要求 | 说明 |
|------|---------|------|
| Rust | stable | `rustup` 安装 |
| OHOS NDK | API 20+ | HarmonyOS OpenHarmony SDK，包含 `aarch64-linux-ohos-clang` 链接器 |
| DevEco Studio | 5.0+ | HarmonyOS 应用开发 IDE |

NDK 路径配置在 `sdk/.cargo/config.toml` 中，默认值为 `~/Library/OpenHarmony/Sdk/20/native/llvm/bin/`。如果 NDK 安装位置或 API 版本不同，需修改此文件。

---

## 3. 构建 libppsdk.so

构建前需配置 JWT Token（同第 1 部分），并将 Token 文件绝对路径写入 `build.jwt.path`。

### 方式一：DevEco Studio 构建

在 DevEco Studio 中打开 `app-test-hmos/` 项目，手动点击 Build > Build Hap(s)/APP(s)。构建过程中，自定义 hvigor 插件（`entry/hvigorfile.ts`）会自动调用 `build-arkts-napi-so.sh` 完成 Rust 编译和 .so 复制。

### 方式二：手动编译 .so

```bash
bash build-arkts-napi-so.sh
```

---

## 4. ArkTS应用集成

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

**3. 代码示例**

```typescript
import ppsdk from 'libppsdk.so'

// 初始化（配置从 config.json 读取）
ppsdk.init(JSON.stringify(config))

// 注册 + 查询 IDS
const resp: IdsResponse = ppsdk.registerIds(appId, userId, odid, pushToken)
const peer: IdsResponse = ppsdk.queryIds(appId, userId)

// 一键建链（内部自动完成 NAT 路由 → 候选收集 → SDP 协商 → ICE 建立）
ppsdk.connect(signalingUrl, odid)

// 发送文本
ppsdk.send(ppsdk.encodeDataFrame('Hello P2P'))

// 接收对端消息（通过回调）
ppsdk.onDataReceived((data: ArrayBuffer): void => {
  const text: string = new util.TextDecoder().decodeToString(new Uint8Array(data))
})

// 断开连接
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

---

## 5. 技术选型

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
