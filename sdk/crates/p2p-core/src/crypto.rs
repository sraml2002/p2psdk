//! Cryptographic primitives: SHA-256, SHA-1, HMAC-SHA1, CRC32.
//!
//! Replaces the hand-written ArkTS BigInt ECDSA (EcdsaUtil.ets) and
//! SHA-1/HMAC implementations in IceStun.ets with standard Rust crates.

use sha2::{Digest, Sha256};
use sha1::Sha1;
use hmac::{Hmac, Mac};

type HmacSha1 = Hmac<Sha1>;
type HmacSha256 = Hmac<Sha256>;

/// Compute SHA-256 hash.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Compute SHA-1 hash.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut out = [0u8; 20];
    out.copy_from_slice(&result);
    out
}

/// Compute HMAC-SHA1.
pub fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
    let mut mac = HmacSha1::new_from_slice(key).expect("HMAC key error");
    mac.update(data);
    let result = mac.finalize();
    let mut out = [0u8; 20];
    out.copy_from_slice(&result.into_bytes());
    out
}

/// Compute HMAC-SHA256.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key error");
    mac.update(data);
    let result = mac.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result.into_bytes());
    out
}

/// Compute CRC32 (STUN FINGERPRINT attribute uses XOR with 0x5354554E).
pub fn crc32(data: &[u8]) -> u32 {
    crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC).checksum(data)
}

/// STUN FINGERPRINT: CRC32 XOR 0x5354554E.
pub fn stun_fingerprint(data: &[u8]) -> u32 {
    crc32(data) ^ 0x5354554E
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256() {
        let hash = sha256(b"hello");
        // Known SHA-256 of "hello"
        assert_eq!(
            hex::encode(hash),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_sha1() {
        let hash = sha1(b"hello");
        assert_eq!(
            hex::encode(hash),
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
        );
    }

    #[test]
    fn test_hmac_sha1() {
        let mac = hmac_sha1(b"key", b"data");
        // HMAC-SHA1 is deterministic
        let mac2 = hmac_sha1(b"key", b"data");
        assert_eq!(mac, mac2);

        // Different key produces different result
        let mac3 = hmac_sha1(b"other", b"data");
        assert_ne!(mac, mac3);
    }

    #[test]
    fn test_stun_fingerprint() {
        // Fingerprint = CRC32 XOR 0x5354554E
        let data = b"test data";
        let fp = stun_fingerprint(data);
        assert_eq!(fp, crc32(data) ^ 0x5354554E);
    }
}
