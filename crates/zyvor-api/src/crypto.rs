//! Envelope for provider secrets: AES-256-GCM with a random 96-bit nonce prepended to the ciphertext.
//! The master key comes from `ZYVOR_MASTER_KEY` (64 hex chars) and is never stored in the database.

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};

#[derive(Clone)]
pub struct Crypto(Aes256Gcm);

impl Crypto {
    pub fn from_hex(h: &str) -> Result<Self, String> {
        let bytes =
            hex::decode(h.trim()).map_err(|_| "ZYVOR_MASTER_KEY must be hex".to_string())?;
        if bytes.len() != 32 {
            return Err("ZYVOR_MASTER_KEY must be 32 bytes (64 hex chars)".into());
        }
        Ok(Self(
            Aes256Gcm::new_from_slice(&bytes).expect("length checked"),
        ))
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let mut out = nonce.to_vec();
        out.extend(self.0.encrypt(&nonce, plaintext).expect("aes-gcm encrypt"));
        out
    }

    pub fn decrypt(&self, blob: &[u8]) -> Option<Vec<u8>> {
        if blob.len() < 12 + 16 {
            return None;
        }
        let (nonce, ct) = blob.split_at(12);
        self.0.decrypt(Nonce::from_slice(nonce), ct).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_tamper() {
        let c = Crypto::from_hex(&"ab".repeat(32)).unwrap();
        let blob = c.encrypt(b"rzp_secret");
        assert_ne!(&blob[12..], b"rzp_secret");
        assert_eq!(c.decrypt(&blob).unwrap(), b"rzp_secret");
        let mut bad = blob.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(c.decrypt(&bad).is_none());
        assert!(
            Crypto::from_hex(&"cd".repeat(32))
                .unwrap()
                .decrypt(&blob)
                .is_none(),
            "wrong key"
        );
        assert_ne!(c.encrypt(b"x"), c.encrypt(b"x"), "fresh nonce each time");
        assert!(Crypto::from_hex("abcd").is_err());
    }
}
