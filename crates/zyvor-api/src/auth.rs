//! API-key authentication. Keys look like `zk_test_<64 hex>`; only a SHA-256 hash is stored.
//! A key belongs to exactly one tenant (organisation + test/live), so environments never mix.

use axum::{extract::FromRequestParts, http::request::Parts};
use sha2::{Digest, Sha256};

use crate::{AppState, error::ApiError};

pub const ALL_SCOPES: &[&str] = &[
    "payments:read",
    "payments:write",
    "refunds:write",
    "refunds:approve",
    "recon:read",
    "recon:write",
    "invoices:write",
    "customers:write",
    "collections:write",
    "billing:write",
    "disputes:write",
    "payouts:write",
    "payouts:approve",
    "closings:approve",
    "onboarding:write",
    "onboarding:review",
    "admin",
];

#[derive(Debug, Clone)]
pub struct Auth {
    pub tenant_id: String,
    pub key_id: String,
    pub env: String,
    pub scopes: Vec<String>,
    /// Who is acting: `Zyvor-Actor` header (a person, set by trusted UIs) or the key itself.
    pub actor: String,
}

impl Auth {
    pub fn require(&self, scope: &str) -> Result<(), ApiError> {
        if self.scopes.iter().any(|s| s == scope || s == "admin") {
            Ok(())
        } else {
            Err(ApiError::forbidden(format!(
                "this API key lacks the `{scope}` scope"
            )))
        }
    }
}

/// `10.0.0.0/8`, `203.0.113.7` (a single address) or an IPv6 equivalent.
pub fn parse_cidr(c: &str) -> Option<(std::net::IpAddr, u8)> {
    let (a, len) = match c.split_once('/') {
        Some((a, l)) => (a, Some(l.parse::<u8>().ok()?)),
        None => (c, None),
    };
    let ip: std::net::IpAddr = a.parse().ok()?;
    let max = if ip.is_ipv4() { 32 } else { 128 };
    let len = len.unwrap_or(max);
    (len <= max).then_some((ip, len))
}

pub fn cidr_contains(c: &str, ip: std::net::IpAddr) -> bool {
    let Some((net, len)) = parse_cidr(c) else {
        return false;
    };
    let (a, b): (Vec<u8>, Vec<u8>) = match (net, ip) {
        (std::net::IpAddr::V4(n), std::net::IpAddr::V4(i)) => {
            (n.octets().to_vec(), i.octets().to_vec())
        }
        (std::net::IpAddr::V6(n), std::net::IpAddr::V6(i)) => {
            (n.octets().to_vec(), i.octets().to_vec())
        }
        _ => return false,
    };
    let (full, rem) = ((len / 8) as usize, len % 8);
    a[..full] == b[..full] && (rem == 0 || (a[full] ^ b[full]) >> (8 - rem) == 0)
}

pub fn hash_key(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

/// Returns `(plaintext, prefix, hash)`. The plaintext is shown to the caller exactly once.
pub fn mint_key(env: &str) -> (String, String, String) {
    let secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let key = format!("zk_{env}_{secret}");
    let prefix = key[..key.len().min(14)].to_string();
    let hash = hash_key(&key);
    (key, prefix, hash)
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let key = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| ApiError::unauthorized("missing bearer API key"))?;
        let row: Option<(String, String, String, Vec<String>, Vec<String>)> = sqlx::query_as(
            "SELECT k.id, k.tenant_id, t.env, k.scopes, k.allowed_cidrs FROM api_keys k JOIN tenants t ON t.id = k.tenant_id
             WHERE k.key_hash = $1 AND k.revoked_at IS NULL AND (k.expires_at IS NULL OR k.expires_at > now())",
        )
        .bind(hash_key(key))
        .fetch_optional(&state.pool)
        .await?;
        let (key_id, tenant_id, env, scopes, cidrs) =
            row.ok_or_else(|| ApiError::unauthorized("invalid, expired or revoked API key"))?;
        if !cidrs.is_empty() {
            let ip =
                crate::ratelimit::client_ip(&parts.headers, &parts.extensions, state.trusted_hops);
            if !ip.is_some_and(|ip| cidrs.iter().any(|c| cidr_contains(c, ip))) {
                return Err(ApiError::forbidden(
                    "this API key is not allowed from this address",
                ));
            }
        }
        // best effort and at most once a minute per key, so reads stay reads
        let _ = sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')")
            .bind(&key_id)
            .execute(&state.pool)
            .await;
        let actor = parts
            .headers
            .get("zyvor-actor")
            .and_then(|v| v.to_str().ok())
            .filter(|a| !a.is_empty() && a.len() <= 120)
            .map(|a| format!("user:{a}"))
            .unwrap_or_else(|| format!("key:{key_id}"));
        Ok(Auth {
            tenant_id,
            key_id,
            env,
            scopes,
            actor,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_random_prefixed_and_hash_stable() {
        let (k1, p1, h1) = mint_key("test");
        let (k2, _, h2) = mint_key("live");
        assert!(k1.starts_with("zk_test_") && k2.starts_with("zk_live_"));
        assert_ne!(k1, k2);
        assert_eq!(hash_key(&k1), h1);
        assert_ne!(h1, h2);
        assert!(k1.starts_with(&p1) && p1.len() == 14);
    }

    #[test]
    fn cidr_matching() {
        let ip = |s: &str| s.parse().unwrap();
        assert!(cidr_contains("10.0.0.0/8", ip("10.200.1.1")));
        assert!(!cidr_contains("10.0.0.0/8", ip("11.0.0.1")));
        assert!(cidr_contains("203.0.113.7", ip("203.0.113.7")));
        assert!(!cidr_contains("203.0.113.7", ip("203.0.113.8")));
        assert!(cidr_contains("192.168.4.0/22", ip("192.168.7.255")));
        assert!(!cidr_contains("192.168.4.0/22", ip("192.168.8.0")));
        assert!(cidr_contains("2001:db8::/32", ip("2001:db8:1::1")));
        assert!(
            !cidr_contains("10.0.0.0/8", ip("2001:db8::1")),
            "families never mix"
        );
        assert!(parse_cidr("10.0.0.0/33").is_none() && parse_cidr("nope").is_none());
    }

    #[test]
    fn scope_checks() {
        let a = Auth {
            tenant_id: "t".into(),
            key_id: "k".into(),
            env: "test".into(),
            scopes: vec!["payments:read".into()],
            actor: "key:k".into(),
        };
        assert!(a.require("payments:read").is_ok());
        assert!(a.require("refunds:write").is_err());
        let admin = Auth {
            scopes: vec!["admin".into()],
            ..a
        };
        assert!(admin.require("refunds:approve").is_ok());
    }
}
