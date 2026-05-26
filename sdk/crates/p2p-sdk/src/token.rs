mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_token.rs"));

    pub fn decrypt() -> Option<String> {
        use aes_gcm::aead::Aead;
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
        use sha2::Digest;

        let seed = format!("p2psdk-embedded-token{}", EMBEDDED_TS);
        let aes_key = sha2::Sha256::digest(seed.as_bytes());
        let cipher = Aes256Gcm::new_from_slice(&aes_key).ok()?;
        let nonce = Nonce::from_slice(EMBEDDED_IV);
        let plain = cipher.decrypt(nonce, EMBEDDED_CIPHER).ok()?;
        String::from_utf8(plain).ok()
    }
}

pub fn generate_token() -> String {
    embedded::decrypt().unwrap_or_default()
}
