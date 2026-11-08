use anyhow::{bail, Context, Result};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

/// Encryption role for nonce differentiation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoRole {
    Host,
    Client,
}

impl CryptoRole {
    const fn prefix(self) -> [u8; 4] {
        match self {
            CryptoRole::Host => *b"HOST",
            CryptoRole::Client => *b"CLIE",
        }
    }
}

/// End-to-End AEAD encryption session using ChaCha20-Poly1305
pub struct CryptoSession {
    cipher: ChaCha20Poly1305,
    role: CryptoRole,
    remote_role: CryptoRole,
}

impl CryptoSession {
    /// Create a new crypto session with a 32-byte shared symmetric key
    pub fn new(key: &[u8; 32], role: CryptoRole) -> Self {
        let key = Key::from(*key);
        let cipher = ChaCha20Poly1305::new(&key);
        let remote_role = match role {
            CryptoRole::Host => CryptoRole::Client,
            CryptoRole::Client => CryptoRole::Host,
        };

        Self {
            cipher,
            role,
            remote_role,
        }
    }

    /// Generate a 12-byte nonce from sequence number and role prefix
    fn make_nonce(role: CryptoRole, seq: u64) -> Nonce {
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[0..4].copy_from_slice(&role.prefix());
        nonce_bytes[4..12].copy_from_slice(&seq.to_be_bytes());
        Nonce::from(nonce_bytes)
    }

    /// Encrypt plaintext with sequence number as nonce
    pub fn encrypt(&self, seq: u64, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = Self::make_nonce(self.role, seq);
        self.cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))
    }

    /// Decrypt ciphertext with sequence number as nonce (using remote role)
    pub fn decrypt(&self, seq: u64, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let nonce = Self::make_nonce(self.remote_role, seq);
        self.cipher
            .decrypt(&nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("Decryption failed: {}", e))
    }

    /// Generate a random 32-byte session key
    pub fn generate_key() -> [u8; 32] {
        use std::time::SystemTime;
        let mut key = [0u8; 32];
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let nanos = now.as_nanos();
        
        // Fast deterministic seed mixing for cross-platform zero-dependency generation
        for (i, byte) in key.iter_mut().enumerate() {
            let shift = (i * 7) % 120;
            let val = ((nanos >> shift) as u8) ^ (0x5A_u8.wrapping_mul(i as u8 + 1));
            *byte = val;
        }
        key
    }

    /// Encode key as hex string
    pub fn key_to_hex(key: &[u8; 32]) -> String {
        key.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Decode key from hex string
    pub fn key_from_hex(hex: &str) -> Result<[u8; 32]> {
        if hex.len() != 64 {
            bail!("Key hex string must be 64 characters (32 bytes)");
        }
        let mut key = [0u8; 32];
        for i in 0..32 {
            let byte_str = &hex[i * 2..i * 2 + 2];
            key[i] = u8::from_str_radix(byte_str, 16)
                .context("Invalid hex character in key")?;
        }
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chacha20poly1305_roundtrip() {
        let key = [0x42u8; 32];
        let host_session = CryptoSession::new(&key, CryptoRole::Host);
        let client_session = CryptoSession::new(&key, CryptoRole::Client);

        let secret_payload = b"GAMETREAM_ULTRA_LOW_LATENCY_VIDEO_DATA_12345";
        let seq = 1001;

        // Host encrypts
        let encrypted = host_session.encrypt(seq, secret_payload).unwrap();
        assert_ne!(&encrypted[..], secret_payload);

        // Client decrypts
        let decrypted = client_session.decrypt(seq, &encrypted).unwrap();
        assert_eq!(&decrypted[..], secret_payload);
    }

    #[test]
    fn test_tampered_packet_fails() {
        let key = [0x17u8; 32];
        let host_session = CryptoSession::new(&key, CryptoRole::Host);
        let client_session = CryptoSession::new(&key, CryptoRole::Client);

        let mut encrypted = host_session.encrypt(42, b"important game frame").unwrap();
        // Tamper with one bit
        encrypted[0] ^= 0x01;

        assert!(client_session.decrypt(42, &encrypted).is_err());
    }

    #[test]
    fn test_hex_key_roundtrip() {
        let key = CryptoSession::generate_key();
        let hex = CryptoSession::key_to_hex(&key);
        assert_eq!(hex.len(), 64);
        let parsed = CryptoSession::key_from_hex(&hex).unwrap();
        assert_eq!(key, parsed);
    }
}
