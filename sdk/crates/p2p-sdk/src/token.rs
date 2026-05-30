mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_token.rs"));

    pub fn decrypt() -> Option<String> {
        use aes_gcm::aead::Aead;
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
        use sha2::Digest;

        let seed = format!("p2psdk-embedded-token{}", EMBEDDED_TS);
        let aes_key = sha2::Sha256::digest(seed.as_bytes());
        let cipher = Aes256Gcm::new_from_slice(&aes_key).ok()?;
        #[allow(deprecated)]
        let nonce = Nonce::from_slice(EMBEDDED_IV);
        let plain = cipher.decrypt(nonce, EMBEDDED_CIPHER).ok()?;
        String::from_utf8(plain).ok()
    }
}

/// Fetch a NAT token from the remote token generation service.
///
/// Expected response: `{"serviceType":"nat","serviceId":"...","token":"eyJ..."}`
fn fetch_remote_token(nat_token_url: &str) -> Result<String, String> {
    if nat_token_url.is_empty() {
        return Err("nat_token_url is empty".into());
    }

    let url = format!(
        "{}{}serviceId=unspecified&serviceType=nat",
        nat_token_url,
        if nat_token_url.contains('?') { "&" } else { "?" }
    );

    let response = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| format!("token request failed: {e}"))?;

    let body: serde_json::Value = response
        .into_json()
        .map_err(|e| format!("token response parse failed: {e}"))?;

    body.get("token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "token field missing in response".into())
}

/// Generate a P2P token by fetching from the remote token service.
///
/// Returns an error if the URL is empty or the request fails.
pub fn generate_token_with_url(nat_token_url: &str) -> Result<String, String> {
    fetch_remote_token(nat_token_url)
}

/// Generate a P2P token using the embedded (build-time) token.
pub fn generate_token() -> String {
    embedded::decrypt().unwrap_or_default()
}
