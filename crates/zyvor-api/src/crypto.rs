//! Envelope for provider secrets: AES-256-GCM with a random 96-bit nonce.
//!
//! The master key comes from `ZYVOR_MASTER_KEY` (64 hex chars) and is never stored in the database.
//! To rotate: set the new key as `ZYVOR_MASTER_KEY`, move the old one to `ZYVOR_MASTER_KEY_PREVIOUS`
//! (comma separated if there are several), restart, run `zyvor-api rekey`, then drop the old key.
//!
//! Stored form: `Z 0x01 | key id (1 byte) | nonce (12) | ciphertext+tag`. The key id is a fingerprint
//! of the key, so a blob says which key sealed it. Blobs from before rotation existed (no header)
//! are still readable and are upgraded by `rekey`.

use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, AeadCore, Generate, KeyInit},
};
use sha2::{Digest, Sha256};

/// The 96-bit nonce of AES-256-GCM. The stored form is unchanged from earlier releases; only the library API moved.
type Nonce = aes_gcm::Nonce<<Aes256Gcm as AeadCore>::NonceSize>;

const MAGIC: [u8; 2] = [b'Z', 0x01];
const HEADER: usize = 3;

#[derive(Clone)]
pub struct Crypto {
    current: (u8, Aes256Gcm),
    /// Every key that may decrypt, current first.
    all: Vec<(u8, Aes256Gcm)>,
}

fn parse(h: &str, name: &str) -> Result<(u8, Aes256Gcm), String> {
    let bytes = hex::decode(h.trim()).map_err(|_| format!("{name} must be hex"))?;
    if bytes.len() != 32 {
        return Err(format!("{name} must be 32 bytes (64 hex chars)"));
    }
    let kid = Sha256::digest(&bytes)[0];
    Ok((
        kid,
        Aes256Gcm::new_from_slice(&bytes).expect("length checked"),
    ))
}

impl Crypto {
    pub fn from_hex(h: &str) -> Result<Self, String> {
        let k = parse(h, "ZYVOR_MASTER_KEY")?;
        Ok(Self {
            current: k.clone(),
            all: vec![k],
        })
    }

    /// Also accept these keys (comma separated hex) for decrypting, never for encrypting.
    pub fn with_previous(mut self, hexes: &str) -> Result<Self, String> {
        for h in hexes.split(',').map(str::trim).filter(|h| !h.is_empty()) {
            self.all.push(parse(h, "ZYVOR_MASTER_KEY_PREVIOUS")?);
        }
        Ok(self)
    }

    /// `ZYVOR_MASTER_KEY` plus the optional `ZYVOR_MASTER_KEY_PREVIOUS`.
    pub fn from_env() -> Result<Self, String> {
        let cur = std::env::var("ZYVOR_MASTER_KEY")
            .map_err(|_| "ZYVOR_MASTER_KEY must be set".to_string())?;
        let c = Self::from_hex(&cur)?;
        match std::env::var("ZYVOR_MASTER_KEY_PREVIOUS") {
            Ok(p) => c.with_previous(&p),
            Err(_) => Ok(c),
        }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        let nonce = Nonce::generate();
        let mut out = MAGIC.to_vec();
        out.push(self.current.0);
        out.extend_from_slice(&nonce);
        out.extend(
            self.current
                .1
                .encrypt(&nonce, plaintext)
                .expect("aes-gcm encrypt"),
        );
        out
    }

    pub fn decrypt(&self, blob: &[u8]) -> Option<Vec<u8>> {
        if blob.len() >= HEADER + 12 + 16 && blob[..2] == MAGIC {
            let kid = blob[2];
            let (nonce, ct) = blob[HEADER..].split_at(12);
            // the id is only a hint: try matching keys first, then the rest
            for (_, k) in self
                .all
                .iter()
                .filter(|(id, _)| *id == kid)
                .chain(self.all.iter().filter(|(id, _)| *id != kid))
            {
                if let Ok(p) = k.decrypt(&Nonce::try_from(nonce).ok()?, ct) {
                    return Some(p);
                }
            }
        }
        // legacy: nonce || ciphertext, no header
        if blob.len() >= 12 + 16 {
            let (nonce, ct) = blob.split_at(12);
            for (_, k) in &self.all {
                if let Ok(p) = k.decrypt(&Nonce::try_from(nonce).ok()?, ct) {
                    return Some(p);
                }
            }
        }
        None
    }

    /// True if this blob is not sealed under the current key (old key, or the pre-rotation format).
    pub fn needs_rekey(&self, blob: &[u8]) -> bool {
        !(blob.len() >= HEADER && blob[..2] == MAGIC && blob[2] == self.current.0)
    }

    /// Re-seal under the current key. `None` if no known key can open it (never overwrite such data).
    pub fn rekey(&self, blob: &[u8]) -> Option<Vec<u8>> {
        self.decrypt(blob).map(|p| self.encrypt(&p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_tamper() {
        let c = Crypto::from_hex(&"ab".repeat(32)).unwrap();
        let blob = c.encrypt(b"rzp_secret");
        assert!(!blob.windows(10).any(|w| w == b"rzp_secret"));
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

    /// Sealed by aes-gcm 0.10.3 (the library before the 0.11 upgrade), key `ab` x 32, nonce 01..0c, no header.
    /// Stored provider secrets look like this or like the headered form, so the upgrade must still open them.
    #[test]
    fn data_sealed_by_the_previous_library_version_still_opens() {
        let legacy = hex::decode("0102030405060708090a0b0c324b3eecad0b209c25b057eb898660c2b0599699bf4744d9a76de2b412d1c9").unwrap();
        let c = Crypto::from_hex(&"ab".repeat(32)).unwrap();
        assert_eq!(
            c.decrypt(&legacy).as_deref(),
            Some(b"provider-secret".as_ref())
        );
        // and the headered form: the same sealed bytes behind `Z 01 <key id>`
        let mut headered = MAGIC.to_vec();
        headered.push(Sha256::digest(hex::decode("ab".repeat(32)).unwrap())[0]);
        headered.extend_from_slice(&legacy);
        assert_eq!(
            c.decrypt(&headered).as_deref(),
            Some(b"provider-secret".as_ref())
        );
        assert!(!c.needs_rekey(&headered));
    }

    #[test]
    fn rotation_reads_old_data_and_rekey_moves_it_to_the_new_key() {
        let (old_hex, new_hex) = ("ab".repeat(32), "cd".repeat(32));
        let old = Crypto::from_hex(&old_hex).unwrap();
        let sealed_old = old.encrypt(b"secret");
        // legacy blob from before the header existed
        let legacy = {
            let nonce = Nonce::generate();
            let k = Aes256Gcm::new_from_slice(&hex::decode(&old_hex).unwrap()).unwrap();
            let mut v = nonce.to_vec();
            v.extend(k.encrypt(&nonce, b"legacy".as_ref()).unwrap());
            v
        };
        let new_only = Crypto::from_hex(&new_hex).unwrap();
        assert!(
            new_only.decrypt(&sealed_old).is_none(),
            "without the previous key it is unreadable"
        );

        let rotating = Crypto::from_hex(&new_hex)
            .unwrap()
            .with_previous(&old_hex)
            .unwrap();
        assert_eq!(rotating.decrypt(&sealed_old).unwrap(), b"secret");
        assert_eq!(rotating.decrypt(&legacy).unwrap(), b"legacy");
        assert!(rotating.needs_rekey(&sealed_old) && rotating.needs_rekey(&legacy));
        let moved = rotating.rekey(&sealed_old).unwrap();
        assert!(!rotating.needs_rekey(&moved));
        assert_eq!(
            new_only.decrypt(&moved).unwrap(),
            b"secret",
            "readable with the new key alone"
        );
        assert_eq!(
            new_only.decrypt(&rotating.rekey(&legacy).unwrap()).unwrap(),
            b"legacy"
        );
        assert!(
            rotating
                .rekey(b"garbage that is long enough to look like a blob")
                .is_none()
        );
        assert!(
            Crypto::from_hex(&new_hex)
                .unwrap()
                .with_previous("nothex")
                .is_err()
        );
    }
}
