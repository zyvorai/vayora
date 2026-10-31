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
    "mandates:read",
    "mandates:write",
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
    /// Set when the key is pinned to one branch.
    pub branch_id: Option<String>,
}

impl Auth {
    /// The branch something should be created in or read from. A branch-pinned key may only use its own
    /// branch (naming another is refused) and gets it by default; other keys get what they asked for.
    pub fn pin_branch(&self, requested: Option<String>) -> Result<Option<String>, ApiError> {
        match (&self.branch_id, requested) {
            (Some(own), Some(r)) if *own != r => Err(ApiError::forbidden(
                "this API key is limited to a different branch".to_string(),
            )),
            (Some(own), _) => Ok(Some(own.clone())),
            (None, r) => Ok(r),
        }
    }

    /// Check a scope. A branch-limited key is refused here: only handlers that enforce the branch call
    /// `require_branch`, so everything else is closed to such keys by default.
    pub fn require(&self, scope: &str) -> Result<(), ApiError> {
        if self.branch_id.is_some() {
            return Err(ApiError::forbidden(
                "a branch-limited API key can only use the branch endpoints (payments, payment links, cash, closings, summaries)".to_string(),
            ));
        }
        self.require_branch(scope)
    }

    /// Like `require`, for handlers that confine a branch-limited key to its own branch (via `pin_branch`).
    pub fn require_branch(&self, scope: &str) -> Result<(), ApiError> {
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

/// The path as the client sent it (the router has already stripped the `/v1` it is nested under).
fn original_path(parts: &Parts) -> &str {
    parts
        .extensions
        .get::<axum::extract::OriginalUri>()
        .map_or_else(|| parts.uri.path(), |u| u.0.path())
}

/// What a business whose live mode was taken away may still do: read, and refund its customers.
fn frozen_may(method: &axum::http::Method, path: &str) -> bool {
    use axum::http::Method;
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    let p = path.strip_prefix("/v1").unwrap_or(path);
    *method == Method::POST
        && ((p.starts_with("/payment_intents/") && p.ends_with("/refunds"))
            || p.starts_with("/refunds/")
            // settling what is already in flight
            || (p.starts_with("/payment_attempts/")
                && (p.ends_with("/refresh") || p.ends_with("/cancel"))))
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
        #[allow(clippy::type_complexity)]
        let row: Option<(String, String, String, Vec<String>, Vec<String>, Option<String>, bool)> = sqlx::query_as(
            "SELECT k.id, k.tenant_id, t.env, k.scopes, k.allowed_cidrs, k.branch_id,
                    (t.env = 'live' AND EXISTS (SELECT 1 FROM live_access l WHERE l.organisation_id = t.organisation_id AND l.status = 'revoked'))
             FROM api_keys k JOIN tenants t ON t.id = k.tenant_id
             WHERE k.key_hash = $1 AND k.revoked_at IS NULL AND (k.expires_at IS NULL OR k.expires_at > now())",
        )
        .bind(hash_key(key))
        .fetch_optional(&state.pool)
        .await?;
        let (key_id, tenant_id, env, scopes, cidrs, branch_id, frozen) =
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
        // Live mode taken away: the merchant can still read everything and refund, nothing else.
        if frozen && !frozen_may(&parts.method, original_path(parts)) {
            return Err(ApiError::forbidden(
                "Zyvor has taken live mode away from this business, so this is stopped; reading and refunds still work. See the Onboarding page.",
            ));
        }
        // best effort and at most once a minute per key, so reads stay reads
        let _ = sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')")
            .bind(&key_id)
            .execute(&state.pool)
            .await;
        let claimed = parts
            .headers
            .get("zyvor-actor")
            .and_then(|v| v.to_str().ok())
            .filter(|a| !a.is_empty() && a.len() <= 120);
        // With an actor secret set, only the console (which holds it) can name a person.
        let believed = match (&state.actor_secret, claimed) {
            (None, c) => c,
            (Some(secret), Some(c)) => parts
                .headers
                .get("zyvor-actor-signature")
                .and_then(|v| v.to_str().ok())
                .filter(|sig| {
                    crate::outbox::verify(
                        secret,
                        sig,
                        c.as_bytes(),
                        chrono::Utc::now().timestamp(),
                        300,
                    )
                })
                .map(|_| c),
            (Some(_), None) => None,
        };
        let actor = believed
            .map(|a| format!("user:{a}"))
            .unwrap_or_else(|| format!("key:{key_id}"));
        Ok(Auth {
            tenant_id,
            key_id,
            env,
            scopes,
            actor,
            branch_id,
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
    fn a_frozen_business_can_read_and_refund_and_nothing_else() {
        use axum::http::Method;
        assert!(frozen_may(&Method::GET, "/v1/payments"));
        assert!(frozen_may(
            &Method::POST,
            "/v1/payment_intents/pi_1/refunds"
        ));
        assert!(frozen_may(&Method::POST, "/v1/refunds/rf_1/approve"));
        assert!(!frozen_may(&Method::POST, "/v1/payment_intents"));
        assert!(!frozen_may(
            &Method::POST,
            "/v1/payment_intents/pi_1/attempts"
        ));
        assert!(frozen_may(
            &Method::POST,
            "/v1/payment_attempts/pa_1/refresh"
        ));
        assert!(!frozen_may(
            &Method::POST,
            "/v1/payment_attempts/pa_1/capture"
        ));
        assert!(!frozen_may(&Method::POST, "/v1/payouts"));
        assert!(!frozen_may(&Method::DELETE, "/v1/refunds/rf_1"));
        assert!(!frozen_may(&Method::PUT, "/v1/onboarding"));
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
            branch_id: None,
        };
        assert!(a.require("payments:read").is_ok());
        assert!(a.require("refunds:write").is_err());
        let admin = Auth {
            scopes: vec!["admin".into()],
            ..a
        };
        assert!(admin.require("refunds:approve").is_ok());
    }

    #[test]
    fn a_branch_key_stays_in_its_branch() {
        let mut a = Auth {
            tenant_id: "t".into(),
            key_id: "k".into(),
            env: "test".into(),
            scopes: vec![],
            actor: "key:k".into(),
            branch_id: Some("br_1".into()),
        };
        assert_eq!(a.pin_branch(None).unwrap().as_deref(), Some("br_1"));
        assert_eq!(
            a.pin_branch(Some("br_1".into())).unwrap().as_deref(),
            Some("br_1")
        );
        assert!(a.pin_branch(Some("br_2".into())).is_err());
        a.branch_id = None;
        assert_eq!(
            a.pin_branch(Some("br_2".into())).unwrap().as_deref(),
            Some("br_2")
        );
        assert_eq!(a.pin_branch(None).unwrap(), None);
    }
}
