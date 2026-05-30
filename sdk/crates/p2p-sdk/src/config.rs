/// SDK configuration (relatively stable service addresses).
#[derive(Debug, Clone)]
pub struct Config {
    pub ids_url: String,
    pub nat_url: String,
    /// NAT token generation service URL. SDK fetches a fresh token from this
    /// endpoint at runtime on each connect.
    pub nat_token_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ids_url: String::new(),
            nat_url: "https://natservice-drcn.platform.dbankcloud.cn:443/trs/v1/route".into(),
            nat_token_url: String::new(),
        }
    }
}
