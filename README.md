# P2P SDK (Rust)

P2P SDK 的全 Rust 实现，用于 HarmonyOS 设备端 P2P 通信。通过 raw NAPI 桥接导出为 `libppsdk.so`，供 ArkTS 应用直接调用。

## 项目结构

```
p2psdk_rust/
├── build-so.sh                    # 唯一构建脚本：cargo build + 复制 .so
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
│       ├── p2p-napi/              # NAPI 桥接层 → libppsdk.so
│       └── dimpl/                 # 第三方依赖（DER 编码）
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
| **p2p-napi** | Raw NAPI FFI 桥接：通过 `.init_array` 自动注册，ThreadsafeFunction 回调到 ArkTS |

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
~/Library/OpenHarmony/Sdk/20/native/llvm/bin/aarch64-linux-ohos-clang
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

第三方 HarmonyOS 应用集成 P2P SDK 只需 **2 个文件**：

| 文件 | 放置位置 | 说明 |
|------|---------|------|
| `libppsdk.so` | `entry/libs/arm64-v8a/` | Rust 编译产物 |
| `index.d.ts` | `entry/src/main/cpp/types/libppsdk/` | 类型声明文件 |

### 集成步骤

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

**3. 在 ArkTS 中直接调用**

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

### 需要的权限

```json
"requestPermissions": [
  { "name": "ohos.permission.INTERNET" },
  { "name": "ohos.permission.GET_WIFI_INFO" },
  { "name": "ohos.permission.DEVICE_INFO" }
]
```

## 运行测试应用

1. 在 DevEco Studio 中打开 `app-test-hmos/` 项目
2. 连接 HarmonyOS 设备或启动模拟器
3. 点击 Run 运行应用

测试应用提供两个 P2P 演示页面：
- **Index**：Connector 信令模式（开发调试用）
- **IdsPage**：IDS + SDP 模式（设备↔云服务场景）

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
