fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let project_root = std::path::Path::new(&manifest_dir).join("..");
    let config_path = project_root.join("build.jwt.path");

    if !config_path.exists() {
        panic!(
            "build.jwt.path not found at '{}'. Create it at project root with the absolute path to your JWT token file.",
            config_path.display()
        );
    }

    let config_content = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|e| panic!("Failed to read '{}': {}", config_path.display(), e));

    let token_file = match config_content.lines().find(|l| {
        let trimmed = l.trim();
        !trimmed.is_empty() && !trimmed.starts_with('#')
    }) {
        Some(line) => line.trim().to_string(),
        None => panic!(
            "build.jwt.path ('{}') has no valid path.",
            config_path.display()
        ),
    };

    let token_path = std::path::Path::new(&token_file);
    if !token_path.exists() {
        panic!("Token file '{}' does not exist.", token_path.display());
    }

    let token = std::fs::read_to_string(token_path)
        .unwrap_or_else(|e| panic!("Failed to read '{}': {}", token_path.display(), e))
        .trim()
        .to_string();

    if token.is_empty() {
        panic!("Token file '{}' is empty.", token_path.display());
    }

    // 加密嵌入（复用 p2p-napi 相同的方案）
    use sha2::Digest;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
    let seed = format!("p2psdk-embedded-token{}", ts);
    let aes_key = sha2::Sha256::digest(seed.as_bytes());

    let iv_bytes = {
        let mut iv = [0u8; 12];
        for i in 0..12 {
            iv[i] = aes_key[i] ^ aes_key[i + 12] ^ aes_key[i + 20];
        }
        iv
    };

    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
    let cipher = Aes256Gcm::new_from_slice(&aes_key).unwrap();
    let nonce = Nonce::from_slice(&iv_bytes);
    let encrypted = cipher.encrypt(nonce, token.as_bytes()).expect("encryption failed");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let path = std::path::Path::new(&out_dir).join("embedded_token.rs");
    std::fs::write(
        &path,
        format!(
            "static EMBEDDED_IV: &[u8] = &{:?};\nstatic EMBEDDED_CIPHER: &[u8] = &{:?};\nstatic EMBEDDED_TS: &str = \"{}\";\n",
            iv_bytes.as_slice(),
            encrypted.as_slice(),
            ts
        ),
    )
    .unwrap();

    println!("cargo:rerun-if-changed={}", config_path.display());
    println!("cargo:rerun-if-changed={}", token_path.display());
    eprintln!("Embedded token: {} bytes encrypted", encrypted.len());
}
