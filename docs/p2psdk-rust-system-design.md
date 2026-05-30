# P2P SDK (Rust) — 系统设计文档

> 最后更新: 2026-05-27

---

## 一、系统上下文

### 1.1 系统定位

**P2P SDK** 是一个用纯 Rust 实现的 P2P 连接建立 SDK。它的核心目标是：为上层应用提供与对端（终端或云服务）基于 ICE 协议快速建立 P2P 连接的能力，同时屏蔽 ICE/STUN/TURN/DTLS/SDP/NAT 等底层协议的复杂实现。

**交付件与集成方式：**

| 集成方式 | 交付件 | 适用场景 |
|---------|--------|---------|
| **ArkTS 应用** | `libppsdk.so` + `index.d.ts` | HarmonyOS 应用通过 NAPI 调用 |
| **Rust 应用** | SDK crate 依赖 | Rust 程序直接依赖，编译为单一可执行文件 |

两种集成方式共享同一套 Rust 协议实现（p2p-core），上层接口语义一致。

**核心能力：**

- **ICE 协议**：对终端进行 Full ICE 协商，对云服务基于 ICE-Lite 完成协商
- **NAT 穿透**：对接外部 STUN/TURN 云服务，支持 host/srflx/relay 全候选类型
- **DTLS 加密**：STUN/TURN 通信通过 DTLS 1.2 加密
- **数据传输**：ICE 协商完成后，通过 UDP 建立数据通道
- **节点发现**：通过 IDS 服务注册和对端查询

### 1.2 外部服务交互关系

```
┌──────────────────────────────────────────────────────────────────────────┐
│                          App 层                                          │
│                                                                         │
│  ┌──────────────────────────┐    ┌──────────────────────────────────┐   │
│  │  ArkTS 应用              │    │  Rust 应用                       │   │
│  │  import ppsdk from       │    │  use p2p_sdk::P2pClient;        │   │
│  │    'libppsdk.so'         │    │  直接依赖 SDK crate              │   │
│  └──────────┬───────────────┘    └──────────────┬───────────────────┘   │
│             │  NAPI (libppsdk.so)               │  crate 依赖           │
├─────────────┼───────────────────────────────────┼───────────────────────┤
│             │                                   │                       │
│  ┌──────────▼───────────────────────────────────▼────────────────────┐  │
│  │                      P2P SDK (核心交付件)                          │  │
│  │                                                                   │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌─────────────────────┐     │  │
│  │  │  p2p-napi    │  │  p2p-sdk     │  │  p2p-tokio          │     │  │
│  │  │  NAPI 桥接    │  │  SDK 门面    │  │  同步 I/O           │     │  │
│  │  │  TSFN 回调   │  │  P2pClient   │  │  UDP/HTTP/WS        │     │  │
│  │  └──────┬───────┘  └──────┬───────┘  └──────────┬──────────┘     │  │
│  │         │                 │                      │                │  │
│  │  ┌──────▼─────────────────▼──────────────────────▼──────────────┐ │  │
│  │  │                      p2p-core                                │ │  │
│  │  │  Sans-IO 协议核心                                            │ │  │
│  │  │  IceAgent / STUN / TURN / SDP / Frame / Crypto              │ │  │
│  │  └─────────────────────────────────────────────────────────────┘ │  │
│  │                                                                  │  │
│  │  ┌──────────────────┐  ┌──────────────────┐                     │  │
│  │  │    p2p-io         │  │    dimpl          │                     │  │
│  │  │  I/O traits       │  │  DER 编码         │                     │  │
│  │  └──────────────────┘  └──────────────────┘                     │  │
│  └──────────────────────────────────────────────────────────────────┘  │
└────────┬─────────────┬──────────────┬──────────────┬──────────────────┘
         │             │              │              │
┌────────▼─────┐ ┌────▼───────┐ ┌───▼──────────┐ ┌▼───────────────┐
│ NAT 路由服务  │ │ STUN/TURN  │ │ IDS 身份服务  │ │ Connector WS   │
│              │ │ 服务器     │ │              │ │ (开发调试)     │
└──────────────┘ └────────────┘ └────────────┘ └────────────────┘
```

### 1.3 外部服务清单

| # | 外部服务 | 协议 | 用途 |
|---|---------|------|------|
| 1 | NAT 路由服务 | HTTPS POST | 获取 STUN/TURN 服务器地址 |
| 2 | STUN 服务器 | UDP + DTLS 1.2 | NAT 探测，获取公网映射地址 (srflx) |
| 3 | TURN 服务器 | UDP + DTLS 1.2 | 分配中继地址 (relay) |
| 4 | IDS 身份服务 | HTTP REST | 设备注册 & 对端查询 |
| 5 | Connector 信令 | WebSocket | 开发调试阶段 ICE 信令交换 |

---

## 二、模块划分

### 2.1 Crate 依赖关系

```
                 p2p-napi (libppsdk.so)
                 ├── client_napi.rs    ← NAPI 导出的函数实现
                 ├── napi_bridge.rs    ← Raw NAPI C FFI + TSFN + NAPI Object 构造
                 └── hilog.rs          ← HarmonyOS 日志
                       │
          ┌────────────┼────────────┐
          ▼            ▼            ▼
      p2p-sdk      p2p-tokio     p2p-core
    (SDK 门面)    (同步 I/O)    (协议核心)
          │            │            │
          ├────────────┤            │
          ▼            ▼            │
       p2p-io ◄────────────────────┘
    (I/O traits)               dimpl
                             (DER 编码)
```

### 2.2 各 Crate 职责

#### p2p-core — Sans-IO 协议核心

纯算法实现，不涉及任何 I/O 操作，可在任意平台（桌面/设备）上运行和测试。

| 模块 | 职责 |
|------|------|
| `ice::agent` | ICE Agent 状态机 (RFC 8445)：候选收集、连接检查、提名、角色冲突 |
| `ice::candidate` | 候选地址解析与格式化 (a=candidate:...) |
| `ice::check_list` | 检查列表构建、优先级计算 (RFC 8445 §5.1.2) |
| `ice::stun_codec` | ICE STUN Binding 协议编解码 |
| `stun::client` | DTLS 加密 STUN/TURN 客户端（通过闭包注入 I/O） |
| `stun::codec` | STUN 消息编解码工具（XOR-ADDRESS、事务 ID 等） |
| `stun::message` | STUN Binding/Allocate 消息构建与解析 |
| `sdp` | SDP offer 生成 & answer 解析 |
| `frame` | P2P 数据帧编解码（长度 + 类型 + payload） |
| `crypto` | 加密原语（SHA-256、SHA-1、HMAC、CRC32、STUN Fingerprint） |
| `types` | 共享类型和常量 |

**Sans-IO 设计**：`IceAgent` 不持有任何 socket 或线程，通过 `tick()` 返回 `Vec<IceAction>`（需要发送的数据），通过 `handle_incoming_data()` 接收外部数据。调用者负责实际的 UDP 收发。

```
IceAgent (Sans-IO)
    │
    │  tick(now_ms) → Vec<IceAction>        驱动状态机，返回待发送数据
    │  handle_incoming_data(data) → Result   处理收到的 UDP 数据
    │  send_data(data) → Option<IceAction>   通过已提名候选对发送数据
    │
    ▼
调用者负责 UDP socket 收发
```

#### p2p-io — 平台 I/O Traits

定义平台无关的 I/O 接口，解耦协议逻辑与具体实现。

| Trait | 方法 | 说明 |
|-------|------|------|
| `UdpTransport` | `send_to`, `recv_from`, `local_addr`, `close` | UDP 收发 |
| `HttpTransport` | `post`, `get` | HTTP 请求 |
| `SignalingTransport` | `connect`, `send`, `try_recv`, `close` | WebSocket 信令 |
| `Platform` | `get_local_addresses`, `random_bytes`, `log` | 平台能力 |
| `DtlsTransport` | `handshake_step`, `encrypt`, `decrypt` | DTLS 加解密 |

#### p2p-tokio — 同步 I/O 实现

基于标准库的同步阻塞 I/O 实现，匹配 NAPI 同步调用模型。

| 结构体 | 实现的 Trait | 底层实现 |
|--------|-------------|---------|
| `SyncUdpTransport` | `UdpTransport` | `std::net::UdpSocket` |
| `SyncHttpTransport` | `HttpTransport` | `reqwest::blocking::Client`（10s 超时） |
| `SyncSignalingTransport` | `SignalingTransport` | `tungstenite` WebSocket |
| `StdPlatform` | `Platform` | `std::net` UDP 连接技巧获取本地 IP |

#### p2p-sdk — SDK 门面

高层 API，编排 ICE/STUN/TURN/IDS/Connector 全流程。

`P2pClient` 统一管理：ICE Agent、候选收集、STUN/TURN 交互、Connector 信令、IDS 注册查询。

#### p2p-napi — NAPI 桥接层

通过 Raw NAPI C FFI 将 Rust 函数导出为 `libppsdk.so` 模块，供 ArkTS 直接调用。

**关键设计：**

- **`.init_array` 注册**：不依赖 napi-ohos crate，通过 `#[link_section = ".init_array"]` 在模块加载时自动调用 `napi_register_module_v1`
- **ThreadsafeFunction (TSFN)**：Rust 后台线程通过 TSFN 安全回调到 ArkTS 主线程
- **全局单例**：`Arc<Mutex<Inner>>` 模块级单例，所有 NAPI 调用共享同一状态
- **后台线程**：ICE tick/recv、Connector loop、SDP 连接均运行在 Rust 后台线程
- **ArrayBuffer 直传**：`send` 直接接收 ArkTS ArrayBuffer，通过 `napi_get_arraybuffer_info` 提取原始字节
- **NAPI Object 返回**：`gatherCandidates`/`registerIds`/`queryIds` 通过 `napi_create_object` + `napi_set_named_property` 返回结构化对象，无需 JSON 字符串中转
- **帧编解码导出**：`encodeDataFrame`/`encodeHeartbeatReply`/`parseFrame`/`isStunMessage` 直接通过 NAPI 暴露，无需 ArkTS 侧实现

### 2.3 NAPI 数据传递机制

ArkTS 应用通过 `import ppsdk from 'libppsdk.so'` 直接调用 SDK 功能，类型声明由 `cpp/types/libppsdk/index.d.ts` 提供。数据在 NAPI 层直接转换，无需中间封装层。

**数据转换机制：**

| 方向 | 转换方式 |
|------|---------|
| ArkTS → Rust（二进制） | `ArrayBuffer` → `napi_get_arraybuffer_info` → `&[u8]` |
| ArkTS → Rust（字符串） | `string` → `napi_get_value_string_utf8` → `&str` |
| Rust → ArkTS（二进制） | `&[u8]` → `napi_create_arraybuffer` → `ArrayBuffer` |
| Rust → ArkTS（对象） | `napi_create_object` + `napi_set_named_property` → 结构化对象 |
| Rust → ArkTS（回调） | TSFN (`napi_create_threadsafe_function`) → ArkTS function |

**ArkTS 严格模式兼容：**

由于 ArkTS 严格模式（`arkts-no-any-unknown`）下 `ppsdk` 的所有返回值被推断为 `any`，App 代码中接收返回值的变量必须显式标注类型：

```typescript
const info: CandidateInfo = ppsdk.gatherCandidates(token)
const frame: ParsedFrame = ppsdk.parseFrame(data)
const code: number = ppsdk.connectViaSdp(url, peerId)
const reply: ArrayBuffer = ppsdk.encodeHeartbeatReply()
```

---

## 三、关键数据流

### 3.1 获取对端地址

通过 IDS 服务完成设备注册和对端查询，获取对端的信令地址。

```
ppsdk.registerIds(appId, userId, odid, pushToken)
  │
  ├── HTTP POST /api/ids
  │     body: { appId, userId, type: "app", odid, token: pushToken }
  │     → 注册本端信息
  │
  ▼
ppsdk.queryIds(appId, userId)
  │
  ├── HTTP GET /api/ids/{appId}/{userId}
  │     → 返回对端记录列表
  │
  ├── 遍历记录，查找 type="service" 的条目
  │     └── record.token 即为对端信令地址
  │
  ▼  获得对端信令地址（peerId），用于后续 P2P 建链
```

### 3.2 候选地址收集

```
ppsdk.gatherCandidates(token)
  │
  │  NAPI 同步调用 (JS 线程阻塞)
  ▼
napi_bridge::gather_candidates()
  │  ArrayBuffer → 无（token 是 string）
  │  调用 client_napi::gather_candidates()
  ▼
client_napi::gather_candidates(token)
  │
  ├── 1. NAT 路由服务 HTTP 请求
  │     ├── POST /trs/v1/route (type=2) → STUN IP/Port
  │     └── POST /trs/v1/route (type=3) → TURN IP/Port
  │
  ├── 2. 获取本地 IP 地址
  │     └── StdPlatform::get_local_addresses()
  │
  ├── 3. UDP socket 绑定 (IPv4 + IPv6)
  │
  ├── 4. STUN Binding (DTLS 加密)
  │     └── get_external_address() → srflx 候选
  │
  ├── 5. TURN Allocate (DTLS 加密)
  │     └── get_turn_relay_address() → relay 候选
  │
  ├── 6. 构建 host/srflx/relay 候选列表
  │
  └── 7. 创建 IceAgent 并添加所有候选
        └── 返回 JSON string
  │
  ▼  napi_bridge: return_json_object()
  │  JSON → napi_create_object + napi_set_named_property
  │  返回 NAPI Object { candidateLines, localAddresses, ... }
```

### 3.3 P2P建链协商（设备↔云服务）

```
ppsdk.connectViaSdp(url, peerId)
  │
  ▼
NAPI 同步调用 → client_napi::connect_via_sdp()
  │  立即返回 0，后台线程执行：
  ▼
connect_via_sdp_bg_inner()
  │
  ├── 1. 锁定 Inner，获取 ICE Agent 本地描述
  ├── 2. generate_sdp_offer(ufrag, pwd, candidates, ip, port)
  ├── 3. HTTP POST /api/ice/offer/{peerId} → SDP answer
  ├── 4. parse_sdp_answer() → 远端 ufrag/pwd/candidates
  ├── 5. start_ice_threads_inner() → 启动 tick + recv 线程
  └── 6. set_remote_session_description() + start_checks()
        │
        ▼  ICE tick 线程 (50ms 循环)
        ├── agent.tick(now_ms) → Vec<IceAction>
        ├── UDP 发送 STUN Binding Request
        ├── 状态变化 → TSFN fire_state() → ArkTS 回调
        └── 连通性检查通过 → USE-CANDIDATE 提名
              │
              ▼  TSFN 回调到 ArkTS
              onStateChange("COMPLETED") → this.connected = true → 聊天 UI 出现
```

### 3.4 P2P建链协商（设备↔设备，开发调试）

```
设备A (Initiator)                          设备B (Responder)
     │                                           │
     │  ppsdk.connectConnector(wsUrl, odid, token)
     │  ──── WS register ────▶                  │
     │  ◀── register_ok ────                    │
     │                                           │
     │  ppsdk.initiateIce(targetId)              │
     │  后台线程：                                │
     │  ├─ 创建 IceAgent (controlling)           │
     │  ├─ 收集候选                              │
     │  └─ ConnectorClient.sendTo(ice-offer)     │
     │  ──── send(ice-offer) ────▶              │
     │                          handleConnectorMsg()
     │                          ├─ 创建 IceAgent (controlled)
     │                          ├─ setRemoteSessionDescription
     │                          └─ sendTo(ice-answer)
     │  ◀── message(ice-answer) ──               │
     │  handleConnectorMsg()                     │
     │  ├─ setRemoteSessionDescription           │
     │  └─ startChecks()                         │
     │                          startChecks()
     │                                           │
     │  ◀──────── STUN Binding Checks ──────────▶│
     │  ◀──────── USE-CANDIDATE ────────────────▶│
     │                                           │
     │  ICE COMPLETED            ICE COMPLETED   │
```

---

## 四、线程模型

```
ArkTS 主线程 (UI)
    │
    │  NAPI 同步调用
    ├── gatherCandidates()  ─── 同步阻塞（HTTP + STUN + TURN）
    ├── connectViaSdp()     ─── 立即返回，后台线程执行
    ├── initiateIce()       ─── 立即返回，后台线程执行
    └── 其他调用            ─── 立即返回
          │
          ▼  TSFN 回调
    onStateChange()  ←── ICE tick 线程
    onDataReceived() ←── ICE recv 线程
    onLog()          ←── 任意后台线程
    onConnectorStateChange() ←── Connector 线程

Rust 后台线程：
    ├── ICE tick 线程     50ms 循环，驱动状态机 + 发送 STUN
    ├── ICE recv 线程     200ms 超时收包，处理 STUN 响应 + 应用数据
    ├── Connector 线程    50ms 循环，WS 收发 + 重连
    └── SDP 连接线程      一次性，HTTP POST + 启动 ICE 线程
```

**线程安全**：
- 所有线程通过 `Arc<Mutex<Inner>>` 共享状态
- 停止线程时先释放锁再 join，避免死锁
- TSFN 句柄使用 `AtomicPtr` + `Acquire/Release` ordering 跨线程传递
- `close()` 先停止所有线程，再释放 TSFN 句柄

---

## 五、技术选型说明

| 决策 | 原因 |
|------|------|
| **全 Rust** | 统一技术栈，单一语言覆盖协议实现、NAPI 桥接、I/O 全链路 |
| **Sans-IO 架构** | 协议逻辑不持有 I/O 资源，桌面端可直接单元测试全流程 |
| **Raw NAPI (.init_array)** | 不依赖 napi-ohos crate，减少编译依赖，更可控的内存管理 |
| **同步阻塞 I/O** | 匹配 NAPI 同步调用语义，避免引入 async runtime 与 NAPI 事件循环冲突 |
| **ThreadsafeFunction** | Rust 后台线程安全回调到 ArkTS JS 线程，NAPI 标准机制 |
| **ArrayBuffer 直传** | send 通过 `napi_get_arraybuffer_info` 直接传递二进制，无编码开销 |
| **NAPI Object 返回** | 通过 `napi_create_object` + `napi_set_named_property` 返回结构化对象，App 无需 JSON 解析 |
| **帧编解码 NAPI 导出** | p2p-core 的帧编解码直接通过 NAPI 暴露，App 层无需重复实现 |
| **全局单例** | NAPI 模块只有一个实例，`once_cell::Lazy<Arc<Mutex<Inner>>>` |
