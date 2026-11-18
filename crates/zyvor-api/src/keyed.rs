//! Keyed hashes for values that must be compared but not kept: phone numbers, e-mail addresses, device tokens,
//! network addresses, card references, UPI ids, account numbers.
//!
//! Each business has its own random secret (`risk_keys`), stored encrypted with the master key. A hash is
//! HMAC-SHA256 over the dimension and the normalised value, cut to 128 bits. Without the secret a stored hash cannot
//! be checked against a guessed phone number, and a hash from one business cannot be compared with another's. The
//! secret is independent of the master key's rotation, so stored allow/block lists keep matching after a rotation.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::PgConnection;

use crate::{ApiError, Crypto};

/// The business's secret, created on first use.
pub async fn tenant_key(
    conn: &mut PgConnection,
    crypto: &Crypto,
    tenant_id: &str,
) -> Result<[u8; 32], ApiError> {
    for _ in 0..2 {
        let stored: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT key_enc FROM risk_keys WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_optional(&mut *conn)
                .await?;
        if let Some(blob) = stored {
            let raw = crypto.decrypt(&blob).ok_or_else(ApiError::internal)?;
            return raw.try_into().map_err(|_| ApiError::internal());
        }
        let mut k = [0u8; 32];
        k[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        k[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        sqlx::query(
            "INSERT INTO risk_keys (tenant_id, key_enc) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(tenant_id)
        .bind(crypto.encrypt(&k))
        .execute(&mut *conn)
        .await?;
    }
    Err(ApiError::internal())
}

/// Lower-case and trim; phone numbers keep digits only (so `+91 98765 43210` and `9876543210` agree on the last ten).
pub fn normalise(dimension: &str, value: &str) -> String {
    let v = value.trim().to_lowercase();
    match dimension {
        "phone" => {
            let digits: String = v.chars().filter(char::is_ascii_digit).collect();
            let n = digits.len();
            if n > 10 {
                digits[n - 10..].to_string()
            } else {
                digits
            }
        }
        _ => v,
    }
}

/// 32 hex characters.
pub fn hash(key: &[u8; 32], dimension: &str, value: &str) -> String {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("any key length");
    m.update(dimension.as_bytes());
    m.update(b"\n");
    m.update(normalise(dimension, value).as_bytes());
    hex::encode(&m.finalize().into_bytes()[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_depend_on_key_dimension_and_normalised_value() {
        let (a, b) = ([1u8; 32], [2u8; 32]);
        assert_eq!(
            hash(&a, "email", "Ravi@Example.com "),
            hash(&a, "email", "ravi@example.com")
        );
        assert_ne!(hash(&a, "email", "x@y.z"), hash(&b, "email", "x@y.z"));
        assert_ne!(hash(&a, "email", "x"), hash(&a, "vpa", "x"));
        assert_eq!(
            hash(&a, "phone", "+91 98765 43210"),
            hash(&a, "phone", "9876543210")
        );
        assert_eq!(hash(&a, "phone", "9876543210").len(), 32);
    }
}
