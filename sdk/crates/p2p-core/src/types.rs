//! ICE shared types and constants (RFC 8445)

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct IceCandidate {
    pub foundation: String,
    pub component_id: u8,
    pub transport: String,
    pub priority: u32,
    pub connection_address: String,
    pub port: u16,
    pub candidate_type: CandidateType,
    pub related_address: String,
    pub related_port: u16,
}

impl Default for IceCandidate {
    fn default() -> Self {
        Self {
            foundation: String::new(),
            component_id: 1,
            transport: "UDP".into(),
            priority: 0,
            connection_address: String::new(),
            port: 0,
            candidate_type: CandidateType::Host,
            related_address: String::new(),
            related_port: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IceCredentials {
    pub ufrag: String,
    pub pwd: String,
}

#[derive(Debug, Clone)]
pub struct CandidatePair {
    pub local: IceCandidate,
    pub remote: IceCandidate,
    pub state: CheckState,
    pub nominated: bool,
    pub priority: u64,
    pub retransmit_count: u32,
    pub retransmit_timer_ms: u64,
    pub last_sent_time_ms: u64,
    pub transaction_id: Vec<u8>,
}

impl Default for CandidatePair {
    fn default() -> Self {
        Self {
            local: IceCandidate::default(),
            remote: IceCandidate::default(),
            state: CheckState::Waiting,
            nominated: false,
            priority: 0,
            retransmit_count: 0,
            retransmit_timer_ms: 0,
            last_sent_time_ms: 0,
            transaction_id: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct IceSessionDescription {
    pub ice_ufrag: String,
    pub ice_pwd: String,
    pub is_lite: bool,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IceDataMessage {
    pub action: String,
    #[serde(rename = "iceUfrag")]
    pub ice_ufrag: String,
    #[serde(rename = "icePwd")]
    pub ice_pwd: String,
    #[serde(rename = "isLite")]
    pub is_lite: bool,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IceCandidateMessage {
    pub action: String,
    pub candidate: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectorMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub data: serde_json::Value,
    #[serde(default)]
    pub auth: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub timestamp: u64,
}

// ── Enums ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceState {
    New,
    Gathering,
    Connecting,
    Connected,
    Completed,
    Failed,
    Disconnected,
    Closed,
}

impl IceState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::New => "NEW",
            Self::Gathering => "GATHERING",
            Self::Connecting => "CONNECTING",
            Self::Connected => "CONNECTED",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::Disconnected => "DISCONNECTED",
            Self::Closed => "CLOSED",
        }
    }
}

impl std::fmt::Display for IceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateType {
    Host,
    Srflx,
    Relay,
}

impl CandidateType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Srflx => "srflx",
            Self::Relay => "relay",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "host" => Some(Self::Host),
            "srflx" => Some(Self::Srflx),
            "relay" => Some(Self::Relay),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Waiting,
    InProgress,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceRole {
    Controlling,
    Controlled,
}

// ── Constants ───────────────────────────────────────────────────────────────

// STUN attribute type codes
pub const ATTR_USERNAME: u16 = 0x0006;
pub const ATTR_PRIORITY: u16 = 0x0024;
pub const ATTR_USE_CANDIDATE: u16 = 0x0025;
pub const ATTR_ICE_CONTROLLING: u16 = 0x802A;
pub const ATTR_ICE_CONTROLLED: u16 = 0x8029;
pub const ATTR_FINGERPRINT: u16 = 0x8028;
pub const ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;
pub const ATTR_ERROR_CODE: u16 = 0x0009;
pub const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
pub const ATTR_XOR_RELAYED_ADDRESS: u16 = 0x0016;
pub const ATTR_LIFETIME: u16 = 0x000D;
pub const ATTR_REQUESTED_TRANSPORT: u16 = 0x0019;
pub const ATTR_DONT_FRAGMENT: u16 = 0x001A;
pub const ATTR_RESERVATION_TOKEN: u16 = 0x0022;
pub const ATTR_CHANNEL_NUMBER: u16 = 0x000C;
pub const ATTR_NONCE: u16 = 0x0015;
pub const ATTR_REALM: u16 = 0x0014;
pub const ATTR_SOFTWARE: u16 = 0x8022;
pub const ATTR_P2P_TOKEN: u16 = 0x0081; // Huawei custom attribute
pub const ATTR_REQUESTED_ADDRESS_FAMILY: u16 = 0x0017;
pub const ATTR_XOR_PEER_ADDRESS: u16 = 0x0012;

// STUN message types
pub const STUN_BINDING_REQUEST: u16 = 0x0001;
pub const STUN_BINDING_SUCCESS: u16 = 0x0101;
pub const STUN_BINDING_ERROR: u16 = 0x0111;
pub const STUN_ALLOCATE_REQUEST: u16 = 0x0003;
pub const STUN_ALLOCATE_SUCCESS: u16 = 0x0103;
pub const STUN_ALLOCATE_ERROR: u16 = 0x0113;
pub const STUN_REFRESH_REQUEST: u16 = 0x0004;
pub const STUN_CREATE_PERMISSION_REQUEST: u16 = 0x0008;
pub const STUN_CREATE_PERMISSION_SUCCESS: u16 = 0x0108;
pub const STUN_CREATE_PERMISSION_ERROR: u16 = 0x0118;
pub const STUN_CHANNEL_BIND_REQUEST: u16 = 0x0009;

// ICE error codes
pub const ICE_ERROR_ROLE_CONFLICT: u16 = 487;

// Connector message types
pub const CONNECTOR_TYPE_REGISTER: &str = "register";
pub const CONNECTOR_TYPE_SEND: &str = "send";
pub const CONNECTOR_TYPE_REGISTER_OK: &str = "register_ok";
pub const CONNECTOR_TYPE_REGISTER_FAIL: &str = "register_fail";
pub const CONNECTOR_TYPE_SEND_OK: &str = "send_ok";
pub const CONNECTOR_TYPE_SEND_FAIL: &str = "send_fail";
pub const CONNECTOR_TYPE_MESSAGE: &str = "message";
pub const CONNECTOR_TYPE_ERROR: &str = "error";

// STUN magic cookie
pub const STUN_MAGIC_COOKIE: u32 = 0x2112A442;

// Address family (for XOR-encoded address attributes)
pub const AF_INET: u8 = 0x01;
pub const AF_INET6: u8 = 0x02;

// P2P frame types
pub const TYPE_HEARTBEAT: u32 = 0x00000001;
pub const TYPE_DATA: u32 = 0x00000002;

// TURN transport
pub const TURN_TRANSPORT_UDP: u8 = 17; // UDP protocol number
