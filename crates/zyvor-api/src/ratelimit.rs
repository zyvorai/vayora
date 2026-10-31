//! Request rate limiting: token buckets per API key and per client address, so one noisy or
//! compromised caller cannot starve the rest. In memory per replica: the limit is a guard rail,
//! not an accounting system, so each replica enforcing its own bucket is fine.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::{AppState, auth::hash_key};

#[derive(Clone, Copy, Debug)]
pub struct Bucket {
    pub burst: f64,
    pub per_sec: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// One API key.
    pub key: Bucket,
    /// One address, for requests that carry an API key (many keys can share an office).
    pub ip_authed: Bucket,
    /// One address, for requests with no API key (public checkout, junk).
    pub ip_public: Bucket,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            key: Bucket {
                burst: 300.0,
                per_sec: 100.0,
            },
            ip_authed: Bucket {
                burst: 900.0,
                per_sec: 300.0,
            },
            ip_public: Bucket {
                burst: 60.0,
                per_sec: 20.0,
            },
        }
    }
}

pub struct RateLimiter {
    pub limits: Limits,
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

impl RateLimiter {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// `Ok` if a token was available, else the seconds until one is.
    pub fn check(&self, key: String, b: Bucket, now: Instant) -> Result<(), u64> {
        let mut m = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() > 50_000 {
            m.retain(|_, (_, at)| now.duration_since(*at) < Duration::from_secs(600));
        }
        let e = m.entry(key).or_insert((b.burst, now));
        let refill = now.duration_since(e.1).as_secs_f64() * b.per_sec;
        e.0 = (e.0 + refill).min(b.burst);
        e.1 = now;
        if e.0 >= 1.0 {
            e.0 -= 1.0;
            Ok(())
        } else {
            Err(((1.0 - e.0) / b.per_sec).ceil().max(1.0) as u64)
        }
    }
}

/// The caller's address. With `trusted_hops = N` (`ZYVOR_TRUST_PROXY_HOPS`, the number of reverse
/// proxies we run in front) it is the Nth `X-Forwarded-For` entry counted from the right: each of
/// our proxies appends the peer it saw, so everything left of that is client-controlled and
/// ignored. With 0 it is the socket peer.
pub fn client_ip(
    headers: &HeaderMap,
    ext: &axum::http::Extensions,
    trusted_hops: u8,
) -> Option<IpAddr> {
    if trusted_hops > 0 {
        return headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').nth(trusted_hops as usize - 1))
            .and_then(|v| v.trim().parse().ok());
    }
    ext.get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip())
}

/// The visitor's address as vouched for by the console. The console runs on the same host as the people it
/// serves, so without this every visitor looks like the console and shares one budget. It signs the address
/// with the shared `ZYVOR_ACTOR_SECRET`; an address that is unsigned, badly signed or old is ignored, so a
/// caller cannot pick which budget it falls in.
pub fn vouched_client_ip(secret: Option<&[u8]>, headers: &HeaderMap) -> Option<IpAddr> {
    let secret = secret?;
    let ip = headers.get("zyvor-client-ip")?.to_str().ok()?;
    let sig = headers.get("zyvor-client-ip-signature")?.to_str().ok()?;
    if !crate::outbox::verify(
        secret,
        sig,
        ip.as_bytes(),
        chrono::Utc::now().timestamp(),
        300,
    ) {
        return None;
    }
    ip.trim().parse().ok()
}

fn too_many(retry: u64) -> Response {
    let mut r = (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(json!({ "error": { "code": "rate_limited", "message": format!("too many requests; retry in {retry}s") } })),
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&retry.to_string()) {
        r.headers_mut().insert(header::RETRY_AFTER, v);
    }
    r
}

pub async fn limit(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let l = s.limiter.limits;
    let now = Instant::now();
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let ip = vouched_client_ip(s.actor_secret.as_deref(), req.headers())
        .or_else(|| client_ip(req.headers(), req.extensions(), s.trusted_hops));
    // provider webhooks arrive without a key from a handful of addresses; give them the roomier bucket
    let webhook = req.uri().path().contains("/webhooks/");
    let checks: Vec<(String, Bucket)> = match bearer {
        Some(k) => {
            let mut v = vec![(format!("k:{}", &hash_key(k)[..24]), l.key)];
            v.extend(ip.map(|ip| (format!("ipa:{ip}"), l.ip_authed)));
            v
        }
        None => ip
            .map(|ip| {
                vec![(
                    format!("ipp:{ip}"),
                    if webhook { l.ip_authed } else { l.ip_public },
                )]
            })
            .unwrap_or_default(),
    };
    for (k, b) in checks {
        if let Err(retry) = s.limiter.check(k, b, now) {
            return too_many(retry);
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_drains_then_refills() {
        let rl = RateLimiter::new(Limits::default());
        let b = Bucket {
            burst: 3.0,
            per_sec: 1.0,
        };
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(rl.check("a".into(), b, t0).is_ok());
        }
        assert_eq!(rl.check("a".into(), b, t0), Err(1));
        assert!(
            rl.check("other".into(), b, t0).is_ok(),
            "keys are independent"
        );
        assert!(
            rl.check("a".into(), b, t0 + Duration::from_millis(1100))
                .is_ok()
        );
        assert!(
            rl.check("a".into(), b, t0 + Duration::from_millis(1100))
                .is_err()
        );
    }

    #[test]
    fn forwarded_for_uses_the_proxys_entry_not_the_clients() {
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            HeaderValue::from_static("6.6.6.6, 203.0.113.9"),
        );
        let e = axum::http::Extensions::new();
        assert_eq!(client_ip(&h, &e, 1), Some("203.0.113.9".parse().unwrap()));
        assert_eq!(client_ip(&h, &e, 2), Some("6.6.6.6".parse().unwrap()));
        assert_eq!(
            client_ip(&h, &e, 3),
            None,
            "fewer entries than trusted hops"
        );
        assert_eq!(
            client_ip(&h, &e, 0),
            None,
            "ignored unless a proxy is trusted"
        );
    }
}
