# P2P SDK (Rust) — API 参考文档

> 最后更新: 2026-05-26

---

## 第一部分：Rust Crate 接口（面向 Rust 开发者）

### 公开接口总览

`P2pClient` 是高层 SDK 入口，编排 ICE/STUN/TURN/IDS 全流程。

| 方法 | 签名 | 说明 |
|------|------|------|
| `P2pClient::new()` | `-> Self` | 创建 P2pClient 实例 |
| `init` | `(&mut self, config: Config)` | 初始化 SDK 配置 |
| `on_state_change` | `(&self, cb: Box<dyn Fn(IceState) + Send>)` | 注册 ICE 状态变化回调 |
| `on_data` | `(&self, cb: Box<dyn Fn(Vec<u8>) + Send>)` | 注册数据接收回调（仅数据帧 payload） |
| `register_ids` | `(&self, http, user_id, odid, push_token) -> Result<(), String>` | 向 IDS 注册本端信息 |
| `query_ids` | `(&self, http, user_id) -> Result<IdsRecord, String>` | 查询 IDS 获取对端信息 |
| `connect` | `(&self, peer_addr, odid, heartbeat_secs) -> Result<(), String>` | 一站式建立 P2P 通道（非阻塞，token 内部生成） |
| `send_text` | `(&self, text: &str) -> Result<(), String>` | 通过 P2P 通道发送文本（自动编码+发送） |
| `send_data` | `(&self, data: &[u8]) -> Result<(), String>` | 通过 P2P 通道发送二进制数据（自动发送） |
| `close` | `(&self) -> Result<(), String>` | 停止 ICE 线程、关闭 UDP、释放资源 |

---

### 1.1 p2p-sdk — SDK 门面（P2pClient）

`P2pClient` 是高层 SDK 入口，编排 ICE/STUN/TURN/IDS 全流程。

##### `P2pClient::new() -> Self`

创建 P2pClient 实例。

##### `P2pClient::init(&mut self, config: Config)`

初始化 SDK 配置，内部自动注入 IO 工厂，使 `connect` 等高级方法可用。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| config | `Config` | 是 | SDK 配置 |

**Config 字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| ids_url | `String` | IDS 服务地址（`host:port`） |
| nat_url | `String` | NAT 路由服务 URL |

##### `P2pClient::on_state_change(&self, cb: Box<dyn Fn(IceState) + Send>)`

注册 ICE 状态变化回调。必须在 `connect` 之前调用。

##### `P2pClient::on_data(&self, cb: Box<dyn Fn(Vec<u8>) + Send>)`

注册数据接收回调，仅上报数据帧 payload（心跳帧由 SDK 内部处理）。必须在 `connect` 之前调用。

##### `P2pClient::connect(&self, peer_addr: &str, odid: &str, heartbeat_interval_secs: u32) -> Result<(), String>`

一站式建立 P2P 通道，内部自动完成 Token 生成 → NAT 路由 → 候选收集 → SDP 协商 → ICE 连通性检查。非阻塞，后台线程执行。连接结果通过 `on_state_change` 回调获取，接收数据通过 `on_data` 回调获取。

##### `generate_token() -> String`

生成访问 NAT 服务的 JWT Token（独立函数，非 P2pClient 方法）。Token 在编译时加密嵌入，运行时解密。失败返回空字符串。`connect` 内部自动调用此函数。

##### `P2pClient::send_text(&self, text: &str) -> Result<(), String>`

通过已建立的 P2P 通道发送文本。文本内部自动封装为 P2P 数据帧并通过内部 UDP 发送。

**返回值**：`Ok(())` 成功，`Err(String)` 失败（无 ICE Agent / 无 Nominated Pair）

##### `P2pClient::send_data(&self, data: &[u8]) -> Result<(), String>`

通过已建立的 P2P 通道发送二进制数据，自动通过内部 UDP 发送。

**返回值**：`Ok(())` 成功，`Err(String)` 失败

##### `P2pClient::close(&self) -> Result<(), String>`

停止 ICE tick/recv 线程，关闭 UDP socket，释放所有资源。

##### `P2pClient::register_ids(&self, http, user_id, odid, push_token) -> Result<(), String>`

向 IDS 服务注册本端信息。**同步阻塞 HTTP 调用**（10s 超时）。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| http | `&dyn HttpTransport` | 是 | HTTP 传输实现（定义见 1.10） |
| user_id | `&str` | 是 | 用户 ID |
| odid | `&str` | 是 | 本端设备标识 |
| push_token | `&str` | 是 | 推送 Token |

**返回值**：`Result<(), String>`

##### `P2pClient::query_ids(&self, http, user_id) -> Result<IdsRecord, String>`

查询 IDS 获取对端信息。**同步阻塞 HTTP 调用**（10s 超时）。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| http | `&dyn HttpTransport` | 是 | HTTP 传输实现（定义见 1.10） |
| user_id | `&str` | 是 | 对端用户 ID |

**返回值**：`Result<IdsRecord, String>`

##### `P2pClient::handle_incoming_udp(&self, data, from_ip, from_port) -> HandleDataResult`

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| data | `&[u8]` | 是 | 待发送的二进制数据 |

**返回值**：`Some(IceAction)` 表示有数据待发送，`None` 表示尚未建立通道

##### `P2pClient::handle_incoming_udp(&mut self, data: &[u8], from_ip: &str, from_port: u16) -> HandleDataResult`

处理收到的 UDP 数据。自动处理 STUN 协议消息，返回应用层数据和待发送的响应包。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| data | `&[u8]` | 是 | 收到的原始数据 |
| from_ip | `&str` | 是 | 发送方 IP |
| from_port | `u16` | 是 | 发送方端口 |

**返回值**：`HandleDataResult`

| 字段 | 类型 | 说明 |
|------|------|------|
| app_data | `Option<Vec<u8>>` | 应用层数据（非 STUN 消息时为 Some） |
| actions | `Vec<IceAction>` | 需要发送的响应包 |

##### `P2pClient::parse_received(data: &[u8]) -> Option<ParsedFrame>`

解析收到的 P2P 帧（静态方法）。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| data | `&[u8]` | 是 | `handle_incoming_udp` 返回的 app_data |

**返回值**：`Option<ParsedFrame>`（定义见 1.7），无效帧返回 `None`

##### `P2pClient::ice_state(&self) -> Option<IceState>`

获取当前 ICE 连接状态。

**返回值**：`Some(IceState)`（定义见 1.5），未初始化时返回 `None`

##### `P2pClient::stop_ice(&mut self)`

停止 ICE Agent，关闭所有连接，释放资源。

---

### 内部接口

以下接口为 SDK 内部使用，通常不需要 App 层直接调用。

---

### 1.2 p2p-sdk — NAT 路由与候选收集

##### `P2pClient::resolve_nat_route(&self, http, p2p_token) -> Result<(), String>`

解析 NAT 路由，获取 STUN/TURN 服务器地址。内部执行两次 HTTP POST（type=2 STUN, type=3 TURN），失败不阻塞流程。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| http | `&dyn HttpTransport` | 是 | HTTP 传输实现 |
| p2p_token | `&str` | 是 | 访问 NAT 服务的 JWT Token |

**返回值**：`Result<(), String>`

##### `P2pClient::gather_candidates(&self, http, platform, p2p_token, nat_url) -> Result<CandidateInfo, String>`

收集所有 ICE 候选地址。**同步阻塞调用**，内部执行 HTTP 请求 + STUN/TURN 交互，耗时可达数秒。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| http | `&dyn HttpTransport` | 是 | HTTP 传输实现 |
| platform | `&dyn Platform` | 是 | 平台能力实现 |
| p2p_token | `&str` | 是 | 访问 NAT 服务的 JWT Token |
| nat_url | `&str` | 是 | NAT 路由服务 URL |

**返回值**：`Result<CandidateInfo, String>`（CandidateInfo 定义见 2.10）

##### `P2pClient::setup_ice_and_gather(&self, udp, platform, p2p_token, is_controlling) -> Result<(), String>`

创建 ICE Agent 并收集候选地址。内部调用 `resolve_nat_route` + `gather_candidates`。

##### `P2pClient::connect_via_sdp(&self, http, peer_addr, odid) -> Result<(), String>`

发起 SDP 协商，建立 P2P 通道。向对端 `http://{peer_addr}/api/ice/offer` POST SDP offer。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| http | `&dyn HttpTransport` | 是 | HTTP 传输实现 |
| peer_addr | `&str` | 是 | 对端地址（`ip:port`） |
| odid | `&str` | 是 | 本端设备标识 |

**返回值**：`Result<(), String>`

---

### 1.3 p2p-core — Token 生成（内部）

##### `generate_token() -> String`

生成用于访问 NAT 服务的 JWT Token。

**返回值**：JWT Token 字符串。失败时返回空字符串。

---

### 1.4 p2p-core — STUN/TURN Client

通过 DTLS 加密的 STUN/TURN 交互。使用闭包注入 I/O（Sans-IO）。

##### `get_external_address(send, recv, stun_ip, stun_port, p2p_token) -> Result<StunResult, StunClientError>`

通过 STUN Binding 获取公网映射地址。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| send | `&mut dyn FnMut(&[u8])` | 是 | 发送闭包：将数据发送到 STUN 服务器 |
| recv | `&mut dyn FnMut(u64) -> Option<Vec<u8>>` | 是 | 接收闭包：等待响应，参数为超时毫秒 |
| stun_ip | `&str` | 是 | STUN 服务器 IP |
| stun_port | `u16` | 是 | STUN 服务器端口 |
| p2p_token | `&str` | 是 | 访问 NAT 服务的 JWT Token |

**返回值**：`Result<StunResult, StunClientError>`

**StunResult**：

| 字段 | 类型 | 说明 |
|------|------|------|
| ip | `String` | 公网映射 IP |
| port | `u16` | 公网映射端口 |

##### `get_turn_relay_address(send, recv, turn_ip, turn_port, p2p_token, family) -> Result<TurnResult, StunClientError>`

通过 TURN Allocate 分配中继地址。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| send | `&mut dyn FnMut(&[u8])` | 是 | 发送闭包 |
| recv | `&mut dyn FnMut(u64) -> Option<Vec<u8>>` | 是 | 接收闭包 |
| turn_ip | `&str` | 是 | TURN 服务器 IP |
| turn_port | `u16` | 是 | TURN 服务器端口 |
| p2p_token | `&str` | 是 | 访问 NAT 服务的 JWT Token |
| family | `u8` | 是 | 地址族：`AF_INET`(IPv4) 或 `AF_INET6`(IPv6) |

**返回值**：`Result<TurnResult, StunClientError>`

**TurnResult**：

| 字段 | 类型 | 说明 |
|------|------|------|
| relay_ip | `String` | 中继地址 IP |
| relay_port | `u16` | 中继地址端口 |
| mapped_ip | `String` | 映射地址 IP |
| mapped_port | `u16` | 映射地址端口 |

---

### 1.5 p2p-core — ICE Agent

**Sans-IO** 状态机实现，不持有任何 I/O 资源。

##### `IceAgent::new(config: IceAgentConfig) -> Self`

创建 ICE Agent 实例。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| config | `IceAgentConfig` | 是 | ICE 配置 |

**IceAgentConfig**：

| 字段 | 类型 | 说明 |
|------|------|------|
| is_controlling | `bool` | 是否为 controlling 角色 |

##### `IceAgent::add_host_candidate(&mut self, addr: &str, port: u16)`

添加 host 候选地址。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| addr | `&str` | 是 | 本机 IP 地址 |
| port | `u16` | 是 | 本机端口 |

##### `IceAgent::add_local_candidate(&mut self, cand: IceCandidate)`

添加本地候选（srflx/relay）。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| cand | `IceCandidate` | 是 | 候选地址（定义见下） |

##### `IceAgent::add_remote_candidate(&mut self, line: &str)`

从 `a=candidate:` 行解析并添加远端候选。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| line | `&str` | 是 | SDP candidate 行 |

##### `IceAgent::local_candidates(&self) -> &[IceCandidate]`

获取所有本地候选。

**返回值**：本地候选切片。

##### `IceAgent::local_session_description(&self) -> IceSessionDescription`

获取本地会话描述（ufrag + pwd）。

**返回值**：`IceSessionDescription`

##### `IceAgent::set_remote_session_description(&mut self, desc: &IceSessionDescription)`

设置远端会话描述。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| desc | `&IceSessionDescription` | 是 | 远端会话描述 |

##### `IceAgent::start_checks(&mut self) -> Result<(), String>`

启动连通性检查。

**返回值**：`Ok(())` 成功，`Err(msg)` 失败

##### `IceAgent::tick(&mut self, now_ms: u64) -> Vec<IceAction>`

驱动状态机，返回待发送的数据包。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| now_ms | `u64` | 是 | 当前时间戳（毫秒） |

**返回值**：`Vec<IceAction>`（待发送的数据包列表）

##### `IceAgent::handle_incoming_data(&mut self, data: &[u8], from_ip: &str, from_port: u16) -> HandleDataResult`

处理收到的 UDP 数据。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| data | `&[u8]` | 是 | 收到的原始数据 |
| from_ip | `&str` | 是 | 发送方 IP |
| from_port | `u16` | 是 | 发送方端口 |

**返回值**：`HandleDataResult`

| 字段 | 类型 | 说明 |
|------|------|------|
| app_data | `Option<Vec<u8>>` | 应用层数据（非 STUN 消息时为 Some） |
| actions | `Vec<IceAction>` | 需要发送的响应包 |

##### `IceAgent::send_data(&self, data: &[u8]) -> Option<IceAction>`

通过已提名的候选对发送应用数据。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| data | `&[u8]` | 是 | 待发送的应用数据 |

**返回值**：`Some(IceAction)` 表示有数据待发送，`None` 表示尚未建立通道

##### `IceAgent::state(&self) -> IceState`

获取当前 ICE 状态。

**返回值**：`IceState`

| 变体 | 说明 |
|------|------|
| `New` | 初始化 |
| `Gathering` | 候选收集中 |
| `Connecting` | 连通性检查中 |
| `Connected` | 首个候选对成功 |
| `Completed` | 提名完成 |
| `Failed` | 协商失败 |
| `Disconnected` | 连接断开 |
| `Closed` | 已关闭 |

##### `IceAgent::stop(&mut self)`

停止 ICE Agent。

**核心类型 IceAction**：

| 字段 | 类型 | 说明 |
|------|------|------|
| data | `Vec<u8>` | 待发送的原始数据 |
| target_ip | `String` | 目标 IP |
| target_port | `u16` | 目标端口 |

**核心类型 IceCandidate**：

| 字段 | 类型 | 说明 |
|------|------|------|
| foundation | `String` | 候选基础标识 |
| component_id | `u32` | 组件 ID |
| transport | `String` | 传输协议 |
| priority | `u32` | 优先级 |
| connection_address | `String` | 连接地址 |
| port | `u16` | 连接端口 |
| candidate_type | `CandidateType` | 候选类型：`Host` / `Srflx` / `Relay` |
| related_address | `String` | 关联地址 |
| related_port | `u16` | 关联端口 |

---

### 1.6 p2p-core — SDP

##### `generate_sdp_offer(odid, local_ufrag, local_pwd, candidates, default_ip, default_port) -> String`

生成 SDP offer 字符串。将 `odid` 写入 SDP `o=` 字段。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| odid | `&str` | 是 | 本端设备标识，写入 `o=` 字段 |
| local_ufrag | `&str` | 是 | ICE ufrag |
| local_pwd | `&str` | 是 | ICE password |
| candidates | `&[String]` | 是 | 候选行列表 |
| default_ip | `&str` | 是 | 默认连接 IP |
| default_port | `u16` | 是 | 默认连接端口 |

**返回值**：SDP offer 字符串

##### `parse_sdp_answer(sdp_text: &str) -> SdpAnswerInfo`

解析 SDP answer，提取远端 ufrag、pwd 和候选列表。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| sdp_text | `&str` | 是 | SDP answer 文本 |

**返回值**：`SdpAnswerInfo`

---

### 1.7 p2p-core — Frame

##### `encode_data_frame(text: &str) -> Vec<u8>`

将文本编码为 P2P 数据帧。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| text | `&str` | 是 | 待编码文本 |

**返回值**：编码后的字节序列

##### `encode_heartbeat_reply() -> Vec<u8>`

生成心跳回复帧。

**返回值**：心跳回复帧字节序列

##### `parse_frame(data: &[u8]) -> Option<ParsedFrame>`

解析 P2P 帧。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| data | `&[u8]` | 是 | 待解析的原始数据 |

**返回值**：`Option<ParsedFrame>`，无效帧返回 `None`

**ParsedFrame**：

| 字段 | 类型 | 说明 |
|------|------|------|
| frame_type | `u32` | 帧类型：`0`=无效，`1`=心跳，`2`=数据 |
| payload | `Vec<u8>` | 帧载荷 |

##### `is_stun_message(data: &[u8]) -> bool`

判断数据是否为 STUN 协议消息。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| data | `&[u8]` | 是 | 待判断的原始数据 |

**返回值**：`true` = STUN 消息

---

### 1.8 帧类型常量

| 常量 | 值 | 说明 |
|------|-----|------|
| `TYPE_HEARTBEAT` | `0x00000001` | 心跳帧 |
| `TYPE_DATA` | `0x00000002` | 数据帧 |

### 1.9 地址族常量

| 常量 | 值 | 说明 |
|------|-----|------|
| `AF_INET` | `0x01` | IPv4 |
| `AF_INET6` | `0x02` | IPv6 |

---

### 1.10 p2p-io — I/O Traits（平台抽象）

##### `UdpTransport`

| 方法 | 签名 | 说明 |
|------|------|------|
| `send_to` | `(&self, data: &[u8], ip: &str, port: u16) -> Result<(), IoError>` | 发送 UDP 数据 |
| `recv_from` | `(&self, timeout_ms: u64) -> Result<(Vec<u8>, String, u16), IoError>` | 接收 UDP 数据，超时返回 Err |
| `local_addr` | `(&self) -> Result<(String, u16), IoError>` | 获取本地绑定地址 |
| `close` | `(&self)` | 关闭 socket |

##### `HttpTransport`

| 方法 | 签名 | 说明 |
|------|------|------|
| `post` | `(&self, url: &str, headers: &[(String, String)], body: &[u8]) -> Result<(u16, String), IoError>` | HTTP POST，返回 (状态码, 响应体) |
| `get` | `(&self, url: &str, headers: &[(String, String)]) -> Result<(u16, String), IoError>` | HTTP GET，返回 (状态码, 响应体) |

##### `SignalingTransport`

| 方法 | 签名 | 说明 |
|------|------|------|
| `connect` | `(&mut self, url: &str) -> Result<(), IoError>` | 连接 WebSocket 服务器 |
| `send` | `(&self, data: &str) -> Result<(), IoError>` | 发送 WebSocket 消息 |
| `try_recv` | `(&mut self) -> Result<Option<String>, IoError>` | 非阻塞接收消息 |
| `close` | `(&mut self)` | 关闭连接 |
| `is_connected` | `(&self) -> bool` | 连接是否存活 |

##### `Platform`

| 方法 | 签名 | 说明 |
|------|------|------|
| `get_local_addresses` | `(&self) -> Vec<String>` | 获取本机 IP 地址列表 |
| `random_bytes` | `(&self, buf: &mut [u8])` | 填充随机字节 |
| `log` | `(&self, msg: &str)` | 输出日志 |

---

### 1.11 p2p-tokio — 同步实现

基于标准库的同步阻塞 I/O 实现，匹配 NAPI 同步调用模型。

##### `SyncUdpTransport`

| 方法 | 签名 | 说明 |
|------|------|------|
| `bind` | `(addr: &str, port: u16) -> Result<Self, IoError>` | 绑定到指定地址 |
| `bind_any` | `(port: u16) -> Result<Self, IoError>` | 绑定到 `0.0.0.0:port` |
| `bind_any_v6` | `(port: u16) -> Result<Self, IoError>` | 绑定到 `[::]:port` |

实现 trait：`UdpTransport`（底层：`std::net::UdpSocket`）

##### `SyncHttpTransport`

| 方法 | 签名 | 说明 |
|------|------|------|
| `new` | `() -> Self` | 创建实例（10s 超时） |

实现 trait：`HttpTransport`（底层：`reqwest::blocking::Client`）

##### `SyncSignalingTransport`

| 方法 | 签名 | 说明 |
|------|------|------|
| `new` | `() -> Self` | 创建实例 |

实现 trait：`SignalingTransport`（底层：`tungstenite` WebSocket）

##### `StdPlatform`

| 方法 | 签名 | 说明 |
|------|------|------|
| `new` | `() -> Self` | 创建实例 |

实现 trait：`Platform`（底层：`std::net`）

---

### 1.12 p2p-napi — NAPI 导出

`libppsdk.so` 通过 Raw NAPI 导出函数，通过 `.init_array` 自动注册。

**返回值约定**：

- **整数返回值**：`0` = 成功，负值 = 错误（`-1` 通用错误，`-2` 无 ICE Agent，`-3` 无 UDP socket）
- **对象返回值**：通过 `napi_create_object` + `napi_set_named_property` 构造
- **错误对象**：HTTP 请求失败时返回 `{error: "..."}`

**NAPI 数据转换**：

| ArkTS → Rust | 机制 | Rust → ArkTS | 机制 |
|-------------|------|-------------|------|
| `ArrayBuffer` → `&[u8]` | `napi_get_arraybuffer_info` | `&[u8]` → `ArrayBuffer` | `napi_create_arraybuffer` |
| `string` → `&str` | `napi_get_value_string_utf8` | `JSON Value` → NAPI Object | `napi_create_object` + `napi_set_named_property` |
| `function` → TSFN | `napi_create_threadsafe_function` | `string` → `string` | `napi_create_string_utf8` |
| — | — | `bool` → `boolean` | `napi_get_boolean` |

**导出函数列表**（共 21 个）：

外部接口：

| # | 导出名 | 签名 | 说明 |
|---|--------|------|------|
| 1 | `init(config)` | `(string) → number` | 初始化 SDK（JSON: idsUrl, natUrl） |
| 2 | `registerIds(appId, userId, odid, pushToken)` | `(string, string, string, string) → IdsResponse` | 注册到 IDS |
| 3 | `queryIds(appId, userId)` | `(string, string) → IdsResponse` | 查询 IDS |
| 4 | `connect(peerId, odid, isDevice?, heartbeatInterval?)` | `(string, string, boolean?, number?) → number` | 一站式建立 P2P 通道（非阻塞） |
| 5 | `onStateChange(cb)` | `(function) → void` | 注册通道状态回调 |
| 6 | `sendText(text)` | `(string) → number` | 发送文本（自动封装数据帧） |
| 7 | `sendData(data)` | `(ArrayBuffer) → number` | 发送二进制数据 |
| 8 | `onData(cb)` | `(function) → void` | 注册数据接收回调（仅数据帧 payload） |
| 9 | `close()` | `() → number` | 关闭所有连接 |

内部接口：

| # | 导出名 | 签名 | 说明 |
|---|--------|------|------|
| 10 | `generateToken()` | `() → string` | 生成访问 NAT 服务的 JWT Token |
| 11 | `encodeDataFrame(text)` | `(string) → ArrayBuffer` | 文本 → 数据帧 |
| 12 | `encodeHeartbeatReply()` | `() → ArrayBuffer` | 心跳回复帧 |
| 13 | `parseFrame(data)` | `(ArrayBuffer) → ParsedFrame` | 解析帧 |
| 14 | `isStunMessage(data)` | `(ArrayBuffer) → boolean` | STUN 消息检测 |
| 15 | `onLog(cb)` | `(function) → void` | 注册日志回调 |
| 16 | `onConnectorStateChange(cb)` | `(function) → void` | 注册 Connector 状态回调（开发中） |

开发中接口：

| # | 导出名 | 签名 | 说明 |
|---|--------|------|------|
| 17 | `connectConnector(url, id, auth)` | `(string, string, string) → number` | 连接 WS 信令服务器 |
| 18 | `disconnectConnector()` | `() → number` | 断开 Connector |
| 19 | `isConnectorRegistered()` | `() → number` | Connector 注册状态 |
| 20 | `initiateIce(targetId)` | `(string) → number` | 通过 Connector 发起 ICE |
| 21 | `stopIce()` | `() → number` | 停止 ICE Agent |

---

## 第二部分：ArkTS 调用接口（面向鸿蒙 Next App）

App 通过 `import ppsdk from 'libppsdk.so'` 直接调用所有 SDK 功能，无需 ETS 封装层。类型声明由 `cpp/types/libppsdk/index.d.ts` 提供。

```typescript
import ppsdk from 'libppsdk.so'

// 初始化
ppsdk.init(JSON.stringify({ idsUrl: '...', natUrl: '...' }))
```

> **ArkTS 严格模式注意**：`ppsdk` 返回值为 `any`，接收返回值的变量必须显式标注类型。

### 公开接口总览

| 方法 | 签名 | 说明 |
|------|------|------|
| `init` | `(configJson: string): number` | 初始化 SDK |
| `onStateChange` | `(cb: (state: string) => void): void` | 注册通道状态回调 |
| `onData` | `(cb: (data: ArrayBuffer) => void): void` | 注册数据接收回调（仅数据帧 payload） |
| `registerIds` | `(appId: string, userId: string, odid: string, pushToken: string): IdsResponse` | 注册到 IDS |
| `queryIds` | `(appId: string, userId: string): IdsResponse` | 查询 IDS |
| `connect` | `(peerId: string, odid: string, isDevice?: boolean, heartbeatInterval?: number): number` | 一站式建立 P2P 通道 |
| `sendText` | `(text: string): number` | 发送文本（自动封装数据帧） |
| `sendData` | `(data: ArrayBuffer): number` | 发送二进制数据 |
| `close` | `(): number` | 关闭所有连接 |

---

### 2.1 初始化

##### `init(configJson: string): number`

传入 JSON 字符串形式的配置，初始化 SDK。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| configJson | string | 是 | JSON 字符串，字段见下表 |

**configJson 字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| idsUrl | string | IDS 服务地址（`host:port`） |
| natUrl | string | NAT 路由服务 URL |

**返回值**：`0` = 成功，负值 = 失败

```typescript
ppsdk.init(JSON.stringify({
  idsUrl: 'ids-host:port',
  natUrl: 'https://natservice...',
}))
```

---

### 2.2 IDS 注册

##### `registerIds(appId: string, userId: string, odid: string, pushToken: string): IdsResponse`

向 IDS 服务注册本端信息。**同步阻塞 HTTP 调用**（10s 超时）。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| appId | string | 是 | 应用 ID |
| userId | string | 是 | 用户 ID |
| odid | string | 是 | 本端设备标识 |
| pushToken | string | 是 | 推送 Token |

**返回值**：`IdsResponse`

**IdsResponse**:

| 属性 | 类型 | 说明 |
|------|------|------|
| `code` | `number` | 响应码 |
| `message` | `string` | 响应消息 |
| `error` | `string \| undefined` | 错误信息（仅失败时存在） |
| `data` | `IdsRecord[] \| undefined` | 记录数组（可能不存在） |

**IdsRecord**:

| 属性 | 类型 | 说明 |
|------|------|------|
| `appId` | `string` | 应用 ID |
| `userId` | `string` | 用户 ID |
| `type` | `string` | 记录类型（`'app'`、`'service'` 等） |
| `odid` | `string` | 设备 ODID |
| `token` | `string` | 信令地址或 Push Token |

```typescript
const resp: IdsResponse = ppsdk.registerIds(appId, userId, odid, pushToken)
if (resp.error !== undefined && resp.error.length > 0) {
  // 注册失败
} else {
  // 注册成功
}
```

---

### 2.3 IDS 查询

##### `queryIds(appId: string, userId: string): IdsResponse`

查询 IDS 获取对端信息。**同步阻塞 HTTP 调用**（10s 超时）。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| appId | string | 是 | 应用 ID |
| userId | string | 是 | 对端用户 ID |

**返回值**：`IdsResponse`（定义见 2.2）

```typescript
const resp: IdsResponse = ppsdk.queryIds(appId, userId)
if (resp.data !== undefined && resp.data.length > 0) {
  for (let i = 0; i < resp.data.length; i++) {
    const record: IdsRecord = resp.data[i]
    if (record.type === 'service' && record.token.length > 0) {
      // 找到 service 记录
      break
    }
  }
}
```

---

### 2.4 建立连接

##### `connect(peerId: string, odid: string, isDevice?: boolean, heartbeatInterval?: number): number`

一站式建立 P2P 通道，内部自动串联以下步骤：

1. `generateToken` — 生成访问 NAT 服务的 JWT Token
2. `gatherCandidates` — 收集 ICE 候选地址
3. `iceSdpNegotiate` — 发起 SDP 协商

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| peerId | string | 是 | 从 IDS 查询到的对端 token 值 |
| odid | string | 是 | 本端设备标识 |
| isDevice | boolean | 否 | `false`（缺省）= 对端为云服务，`true` = 对端为 App（预留） |
| heartbeatInterval | number | 否 | 心跳间隔（秒），缺省 30 |

非阻塞，后台线程执行。返回 `0` 表示参数正确并已启动后台线程，实际连接结果通过 `onStateChange` 回调获取。

---

### 2.5 通道状态上报

##### `onStateChange(cb: (state: string) => void): void`

注册 P2P 通道状态变化回调。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| cb | `(state: string) => void` | 是 | 状态变化回调函数 |

**state 取值**：

| 状态 | 说明 |
|------|------|
| `NEW` | 初始化 |
| `CONNECTING` | 连通性检查中 |
| `CONNECTED` | 首个候选对成功 |
| `COMPLETED` | 提名完成，通道可用 |
| `FAILED` | 协商失败 |
| `DISCONNECTED` | 心跳超时，连接断开 |
| `CONNECTOR_REGISTERED` | Connector 注册成功 |
| `CONNECTOR_DISCONNECTED` | Connector 断开 |

---

### 2.6 数据发送

##### `sendText(text: string): number`

通过已建立的 P2P 通道发送文本，内部自动封装为 P2P 数据帧。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| text | string | 是 | 待发送的文本内容 |

**返回值**：`0` = 成功，负值 = 失败

##### `sendData(data: ArrayBuffer): number`

通过已建立的 P2P 通道发送二进制数据。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| data | ArrayBuffer | 是 | 待发送的二进制数据 |

**返回值**：`0` = 成功，负值 = 失败

- **前置**: ICE 状态为 COMPLETED 或 CONNECTED

---

### 2.7 数据接收

##### `onData(cb: (data: ArrayBuffer) => void): void`

注册数据接收回调。仅上报数据帧的 payload（心跳帧由 SDK 内部自动回复，不上报 App 层）。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| cb | `(data: ArrayBuffer) => void` | 是 | 数据接收回调函数，data 为数据帧 payload |

---

### 2.8 关闭

##### `close(): number`

关闭所有连接，停止所有线程，释放资源。

**返回值**：`0` = 成功

---

### 内部接口

以下接口为 SDK 内部使用，通常不需要 App 层直接调用。

---

### 2.9 Token 生成

##### `generateToken(): string`

生成用于访问 NAT 服务的 JWT Token。

**返回值**：JWT Token 字符串。失败时返回空字符串。

```typescript
const token: string = ppsdk.generateToken()
```

---

### 2.10 帧编解码

##### `encodeDataFrame(text: string): ArrayBuffer`

将文本编码为 P2P 数据帧。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| text | string | 是 | 待编码的文本内容 |

**返回值**：编码后的 `ArrayBuffer`

```typescript
const frame: ArrayBuffer = ppsdk.encodeDataFrame('Hello')
ppsdk.send(frame)
```

##### `encodeHeartbeatReply(): ArrayBuffer`

生成心跳回复帧。

**返回值**：心跳回复帧的 `ArrayBuffer`

##### `parseFrame(data: ArrayBuffer): ParsedFrame`

解析收到的 P2P 帧。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| data | ArrayBuffer | 是 | 待解析的原始帧数据 |

**返回值**：`ParsedFrame`

**ParsedFrame**:

| 属性 | 类型 | 说明 |
|------|------|------|
| `type` | `number` | 帧类型：`0`=无效，`1`=心跳，`2`=数据 |
| `payload` | `ArrayBuffer` | 帧载荷 |

```typescript
const frame: ParsedFrame = ppsdk.parseFrame(data)
if (frame.type === 1) {
  const reply: ArrayBuffer = ppsdk.encodeHeartbeatReply()
  ppsdk.send(reply)
} else if (frame.type === 2) {
  const text: string = new util.TextDecoder().decodeToString(new Uint8Array(frame.payload))
}
```

##### `isStunMessage(data: ArrayBuffer): boolean`

判断数据是否为 STUN 协议消息。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| data | ArrayBuffer | 是 | 待判断的原始数据 |

**返回值**：`true` = STUN 消息，`false` = 非 STUN 消息

---

### 2.13 其他回调

##### `onLog(cb: (msg: string) => void): void`

注册日志回调，接收 Rust 侧的调试日志。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| cb | `(msg: string) => void` | 是 | 日志回调函数 |

##### `onConnectorStateChange(cb: (connected: boolean) => void): void`

注册 Connector 连接状态回调。（开发中）

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| cb | `(connected: boolean) => void` | 是 | 连接状态回调，`true`=已连接，`false`=已断开 |

---

### 2.14 常量

| 常量 | 值 | 说明 |
|------|-----|------|
| `TYPE_HEARTBEAT` | `1` | 心跳帧类型 |
| `TYPE_DATA` | `2` | 数据帧类型 |

---

### 2.15 Connector 信令（开发中）

##### `connectConnector(url: string, identifier: string, authToken: string): number`

连接 WebSocket 信令服务器。后台线程自动重连（指数退避 1s→30s）。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| url | string | 是 | WebSocket 信令服务器地址 |
| identifier | string | 是 | 本端标识 |
| authToken | string | 是 | 认证 Token |

**返回值**：`0` = 成功，负值 = 失败

##### `disconnectConnector(): number`

断开 Connector 连接。

**返回值**：`0` = 成功

##### `isConnectorRegistered(): number`

查询 Connector 注册状态。

**返回值**：`1` = 已注册，`0` = 未注册

##### `initiateIce(targetId: string): number`

通过 Connector 信令向对端发起 ICE 协商。后台线程处理 ICE offer/answer。

**参数**：

| 参数 | 类型 | 必选 | 说明 |
|------|------|------|------|
| targetId | string | 是 | 对端标识 |

**返回值**：`0` = 成功，负值 = 失败

##### `stopIce(): number`

停止 ICE Agent。

**返回值**：`0` = 成功

---

## 第三部分：外部服务 API

### 3.1 华为云 NAT 路由服务

**端点**: `https://natservice-drcn.platform.dbankcloud.cn:443/trs/v1/route`

**认证**: Authorization Header = ES256 JWT

| 请求 type | 响应字段 |
|-----------|---------|
| `2` (STUN) | `stunIp`, `stunPort` |
| `3` (TURN) | `turnIp`, `turnPort` |

### 3.2 IDS 服务

| 接口 | 方法 | 路径 |
|------|------|------|
| 注册 | POST | `/api/ids` |
| 查询 | GET | `/api/ids/{appId}/{userId}` |
| SDP Offer | POST | `/api/ice/offer` |

### 3.3 Connector 信令服务（开发调试）

WebSocket 端点: `ws://{host}/ws`

| 消息类型 | 方向 | 说明 |
|---------|------|------|
| `register` | 客户端→服务端 | 注册，包含 auth token |
| `register_ok` | 服务端→客户端 | 注册成功 |
| `register_fail` | 服务端→客户端 | 注册失败 |
| `send` | 客户端→服务端 | 发送消息给目标 |
| `message` | 服务端→客户端 | 转发消息 |
