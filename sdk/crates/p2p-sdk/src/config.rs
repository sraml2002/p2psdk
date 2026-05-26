/// SDK configuration (relatively stable service addresses).
#[derive(Debug, Clone)]
pub struct Config {
    pub ids_url: String,
    pub nat_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ids_url: String::new(),
            nat_url: "https://natservice-drcn.platform.dbankcloud.cn:443/trs/v1/route".into(),
        }
    }
}
