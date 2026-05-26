# P2P SDK (Rust)

P2P SDK 的全 Rust 实现，用于 HarmonyOS 设备端 P2P 通信。提供两种集成方式：ArkTS 应用通过 `libppsdk.so` 直接调用，Rust 程序通过 SDK crate 编译为单一可执行文件。支持一键建链（init → registerIds → queryIds → connect），内部自动完成 NAT 路由、候选收集、SDP 协商、ICE 建立。

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

## 1. Rust 应用集成

Rust 程序直接依赖 SDK crate，编译为单一可执行文件。

### 1.1 构建配置

线下获取访问 NAT 服务的 JWT Token，保存为文件，并将文件绝对路径写入项目根目录的 `build.jwt.path`：

```
#NAT服务认证Token文件路径（线下申请）
/path/to/your/jwt-token
```

### 1.2 构建

```bash
# 构建 OHOS 版本（默认，交叉编译到 aarch64-unknown-linux-ohos）
bash build-rust-demo.sh

# 构建 macOS 版本
bash build-rust-demo.sh --target mac
```

产出目录：`app-test-rust/dist/`，包含 `app-test-rust` 可执行文件和 `config.example.json`。

### 1.3 运行

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

### 1.4 代码示例

> 完整可运行的 Demo 见 `app-test-rust/src/main.rs`，以下为精简版，可直接复制为 `main.rs` 使用。

**Cargo.toml 依赖**：

```toml
[dependencies]
p2p-sdk = { path = "../sdk/crates/p2p-sdk" }
p2p-tokio = { path = "../sdk/crates/p2p-tokio" }
```

**main.rs**：

```rust
use std::sync::mpsc;
use std::time::Duration;

use p2p_sdk::{Config, IceState, P2pClient};
use p2p_tokio::SyncHttpTransport;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config {
        ids_url: "{IDS服务URL}".into(),
        nat_url: "{NAT服务URL}".into(),
    };
    let app_id = "{AppId}";
    let user_id = "{用户ID}";
    let odid = "{设备ODID}";

    // 初始化
    let mut client = P2pClient::new();
    client.init(config);

    // 注册回调（connect 之前）
    let (state_tx, state_rx) = mpsc::channel::<IceState>();
    client.on_state_change(Box::new(move |state: IceState| {
        let _ = state_tx.send(state);
    }));
    client.on_data(Box::new(|payload: Vec<u8>| {
        println!("[对端] {}", String::from_utf8_lossy(&payload));
    }));

    // 注册 + 查询 IDS
    let http = SyncHttpTransport::new();
    client.register_ids(&http, app_id, user_id, odid, "")?;
    let peer = client.query_ids(&http, app_id, user_id)?;
    if peer.token.is_empty() {
        return Err("未找到对端".into());
    }
    println!("对端地址: {}", peer.token);

    // 一键建链（token 由 SDK 内部自动生成，非阻塞后台线程执行）
    client.connect(&peer.token, odid, 30)?;

    // 等待 ICE 完成
    loop {
        match state_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(IceState::Completed | IceState::Connected) => {
                println!("已连接");
                break;
            }
            Ok(IceState::Failed) => return Err("ICE 协商失败".into()),
            Ok(IceState::Disconnected) => return Err("连接断开".into()),
            Ok(_) => continue,
            Err(_) => return Err("ICE 协商超时".into()),
        }
    }

    // 发送文本
    client.send_text("Hello P2P")?;

    // 断开连接
    client.close()?;
    Ok(())
}
```

---

## 2. ArkTS 应用集成

第三方 HarmonyOS 应用通过 `libppsdk.so` 集成 P2P SDK。

### 2.1 构建 libppsdk.so

构建前需配置 JWT Token（同第 1 部分），并将 Token 文件绝对路径写入 `build.jwt.path`。

**方式一：DevEco Studio 构建**

在 DevEco Studio 中打开 `app-test-hmos/` 项目，手动点击 Build > Build Hap(s)/APP(s)。构建过程中，自定义 hvigor 插件（`entry/hvigorfile.ts`）会自动调用 `build-arkts-napi-so.sh` 完成 Rust 编译和 .so 复制。

**方式二：手动编译 .so**

```bash
bash build-arkts-napi-so.sh
```

### 2.2 集成SDK&构建鸿蒙App

集成 P2P SDK 只需 **2 个文件**：

| 文件 | 放置位置 | 说明 |
|------|---------|------|
| `libppsdk.so` | `entry/libs/arm64-v8a/` | Rust 编译产物 |
| `index.d.ts` | `entry/src/main/cpp/types/libppsdk/` | 类型声明文件 |

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

### 2.3 代码示例

> 以下为完整可运行的页面组件，可直接复制为 `P2pPage.ets` 使用。完整 Demo 见 `app-test-hmos/entry/src/main/ets/pages/IdsPage.ets`。

```typescript
import { util } from '@kit.ArkTS'
import { deviceInfo } from '@kit.BasicServicesKit'
import ppsdk from 'libppsdk.so'

interface IdsResponse {
  code: number
  message: string
  error: string | undefined
  data: IdsRecord[] | undefined
}

interface IdsRecord {
  appId: string
  userId: string
  type: string
  odid: string
  token: string
}

@Entry
@Component
struct P2pPage {
  @State connected: boolean = false
  @State inputText: string = ''

  aboutToAppear(): void {
    // TODO: 替换为真实服务地址
    ppsdk.init(JSON.stringify({
      idsUrl: '{IDS服务的URL}',
      natUrl: '{NAT服务的URL}',
    }))

    // 注册回调（connect 之前）
    ppsdk.onStateChange((state: string): void => {
      if (state === 'COMPLETED' || state === 'CONNECTED') {
        this.connected = true
      } else if (state === 'FAILED' || state === 'DISCONNECTED') {
        this.connected = false
      }
    })
    ppsdk.onData((data: ArrayBuffer): void => {
      const text: string = new util.TextDecoder().decodeToString(new Uint8Array(data))
      console.info('[对端] ' + text)
    })
  }

  aboutToDisappear(): void {
    ppsdk.close()
  }

  doConnect(): void {
    // TODO: 替换为真实配置
    const appId: string = '{宿主AppId}'
    const userId: string = '{宿主App的用户ID}'
    const odid: string = deviceInfo.ODID || userId

    // 注册 IDS
    const regResp: IdsResponse = ppsdk.registerIds(appId, userId, odid, '')
    if (regResp.error !== undefined && regResp.error.length > 0) {
      console.error('注册失败: ' + regResp.error)
      return
    }

    // 查询 IDS，提取 service 记录的 token 作为对端地址
    const queryResp: IdsResponse = ppsdk.queryIds(appId, userId)
    let peerAddr: string = ''
    if (queryResp.data !== undefined) {
      for (let i = 0; i < queryResp.data.length; i++) {
        const record: IdsRecord = queryResp.data[i]
        if (record.type === 'service' && record.token.length > 0) {
          peerAddr = record.token
          break
        }
      }
    }
    if (peerAddr.length === 0) {
      console.error('未找到对端')
      return
    }

    // 一键建链（非阻塞，结果通过 onStateChange 回调获取）
    ppsdk.connect(peerAddr, odid)
  }

  build(): void {
    Column({ space: 12 }) {
      Button(this.connected ? '已连接' : '注册 + 查询 + 建链')
        .width('90%')
        .enabled(!this.connected)
        .onClick(() => {
          this.doConnect()
        })

      TextInput({ text: this.inputText, placeholder: '输入消息...' })
        .width('90%')
        .onChange((value: string): void => {
          this.inputText = value
        })

      Button('发送')
        .width('90%')
        .enabled(this.connected && this.inputText.length > 0)
        .onClick(() => {
          ppsdk.sendText(this.inputText)
          this.inputText = ''
        })
    }
    .width('100%')
    .height('100%')
    .padding(20)
  }
}
```

> **注意**：ArkTS 严格模式下，`ppsdk` 的返回值为 `any` 类型，所有接收返回值的变量必须显式标注类型（如 `const resp: IdsResponse = ppsdk.registerIds(...)`），否则触发 `arkts-no-any-unknown` 编译错误。

### 2.4 需要的权限

```json
"requestPermissions": [
  { "name": "ohos.permission.INTERNET" },
  { "name": "ohos.permission.GET_WIFI_INFO" },
  { "name": "ohos.permission.DEVICE_INFO" }
]
```

---

## 3. SDK 概览

### 3.1 Rust Crate 依赖关系

```
p2p-napi (→ libppsdk.so)
  ├── p2p-sdk (SDK 门面)
  │     ├── p2p-core (协议核心)
  │     │     └── dimpl (DER 编码)
  │     ├── p2p-io (I/O traits)
  │     └── p2p-tokio (同步 I/O，内部自动注入)
  ├── p2p-tokio (同步 I/O)
  │     ├── p2p-core (协议核心)
  │     └── p2p-io (I/O traits)
  └── p2p-core (直接使用 STUN/SDP/Frame)

app-test-rust (→ 单一可执行文件)
  ├── p2p-sdk (SDK 门面)
  │     ├── p2p-core (协议核心)
  │     ├── p2p-io (I/O traits)
  │     └── p2p-tokio (同步 I/O，内部自动注入)
  ├── p2p-tokio (同步 I/O，直接使用 HttpTransport)
  └── p2p-io (I/O traits)
```

| Crate | 职责 |
|-------|------|
| **p2p-core** | Sans-IO 协议核心：ICE Agent (RFC 8445)、STUN/TURN 编解码、SDP 生成/解析、P2P 数据帧 |
| **p2p-io** | 平台抽象 traits：`UdpTransport`、`HttpTransport`、`SignalingTransport`、`Platform` |
| **p2p-tokio** | 基于标准库的同步实现：`std::net::UdpSocket`、`reqwest::blocking`、`tungstenite` WebSocket |
| **p2p-sdk** | 高层 SDK 门面：`P2pClient` 统一编排 ICE/STUN/TURN/IDS/Connector 全流程 |
| **p2p-napi** | Raw NAPI FFI 桥接：通过 `.init_array` 自动注册，ThreadsafeFunction 回调到 ArkTS；同时导出 C ABI 供非 NAPI 消费者使用 |

### 3.2 开发环境

| 工具 | 版本要求 | 说明 |
|------|---------|------|
| Rust | stable | `rustup` 安装 |
| OHOS NDK | API 20+ | HarmonyOS OpenHarmony SDK，包含 `aarch64-linux-ohos-clang` 链接器 |
| DevEco Studio | 5.0+ | HarmonyOS 应用开发 IDE |

NDK 路径配置在 `sdk/.cargo/config.toml` 中，默认值为 `~/Library/OpenHarmony/Sdk/20/native/llvm/bin/`。如果 NDK 安装位置或 API 版本不同，需修改此文件。

---

## 4. 技术选型

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
