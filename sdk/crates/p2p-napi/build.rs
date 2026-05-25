use sha2::Digest;

fn main() {
    // HarmonyOS NDK 链接
    let sdk_path = std::env::var("OHOS_NDK_HOME")
        .unwrap_or_else(|_| "/Users/sram/Library/OpenHarmony/Sdk/20".to_string());
    println!("cargo:rustc-link-search=native={}/native/sysroot/usr/lib/aarch64-linux-ohos", sdk_path);
    println!("cargo:rustc-link-lib=ace_napi.z");
    println!("cargo:rustc-link-lib=hilog_ndk.z");

    // 嵌入 JWT Token：从 build.jwt.path 配置文件读取 token 文件路径 → 读取 token → AES-256-GCM 加密 → 生成 Rust 源码
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let config_path = std::path::Path::new(&manifest_dir).join("../../../build.jwt.path");

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
            "build.jwt.path ('{}') has no valid path. Add the absolute path to your JWT token file.",
            config_path.display()
        ),
    };

    let token_path = std::path::Path::new(&token_file);
    if !token_path.exists() {
        panic!(
            "Token file '{}' (from build.jwt.path) does not exist. Check the path is correct.",
            token_path.display()
        );
    }
    if !token_path.is_file() {
        panic!(
            "Token path '{}' (from build.jwt.path) is not a file.",
            token_path.display()
        );
    }

    let jwt = std::fs::read_to_string(token_path)
        .unwrap_or_else(|e| panic!("Failed to read token file '{}': {}", token_path.display(), e))
        .trim()
        .to_string();

    if jwt.is_empty() || jwt == "change-me-to-valid-jwt-token" {
        panic!(
            "Token file '{}' contains placeholder or is empty. Provide a valid JWT token.",
            token_path.display()
        );
    }

    // 密钥派生：SHA-256(seed + timestamp)
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
    let seed = format!("p2psdk-embedded-token{}", ts);
    let aes_key = sha2::Sha256::digest(seed.as_bytes());

    // IV（基于 aes_key 字节派生，避免引入 rand 依赖）
    let iv_bytes = {
        let mut iv = [0u8; 12];
        for i in 0..12 {
            iv[i] = aes_key[i] ^ aes_key[i + 12] ^ aes_key[i + 20];
        }
        iv
    };

    // AES-256-GCM 加密
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
    let cipher = Aes256Gcm::new_from_slice(&aes_key).unwrap();
    let nonce = Nonce::from_slice(&iv_bytes);
    let encrypted = cipher
        .encrypt(nonce, jwt.as_bytes())
        .expect("AES-GCM encryption failed");

    // 生成 Rust 源码到 OUT_DIR
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

    println!("cargo:rerun-if-changed=../../../build.jwt.path");
    println!("cargo:rerun-if-changed={}", token_path.display());
    eprintln!("Embedded token: {} bytes encrypted, ts={}", encrypted.len(), ts);
}
