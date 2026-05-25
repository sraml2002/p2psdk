//! Synchronous HTTP transport using reqwest::blocking.

use p2p_io::traits::{IoError, HttpTransport};

pub struct SyncHttpTransport {
    client: reqwest::blocking::Client,
}

impl SyncHttpTransport {
    pub fn new() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }
}

impl Default for SyncHttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport for SyncHttpTransport {
    fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Result<(u16, String), IoError> {
        let mut req = self.client.post(url).body(body.to_vec());
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .send()
            .map_err(|e| IoError::Other(format!("HTTP POST failed: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .map_err(|e| IoError::Other(format!("read body: {e}")))?;
        Ok((status, text))
    }

    fn get(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<(u16, String), IoError> {
        let mut req = self.client.get(url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .send()
            .map_err(|e| IoError::Other(format!("HTTP GET failed: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .map_err(|e| IoError::Other(format!("read body: {e}")))?;
        Ok((status, text))
    }
}
