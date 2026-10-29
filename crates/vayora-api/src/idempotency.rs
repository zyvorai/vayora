//! Idempotency-Key handling, executed inside the request's DB transaction.
//!
//! The key row is inserted in the same transaction as the work. A concurrent
//! duplicate blocks on the primary key until the first request commits, then
//! replays its stored response. If the first request fails, nothing is stored
//! and the key stays usable.

use axum::{
    Json,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use crate::error::ApiError;

pub struct Key {
    scope: &'static str,
    key: String,
}

pub enum Begin {
    Replay(Response),
    Proceed(Key),
}

pub fn fingerprint(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_be_bytes());
        h.update(p.as_bytes());
    }
    hex::encode(h.finalize())
}

pub async fn begin(
    conn: &mut PgConnection,
    scope: &'static str,
    headers: &HeaderMap,
    request_hash: &str,
) -> Result<Begin, ApiError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|k| !k.is_empty() && k.len() <= 255)
        .ok_or_else(|| {
            ApiError::bad_request(
                "idempotency_key_required",
                "Idempotency-Key header (1-255 chars) is required",
            )
        })?
        .to_owned();

    let inserted = sqlx::query(
        "INSERT INTO idempotency_keys (scope, key, request_hash) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(scope)
    .bind(&key)
    .bind(request_hash)
    .execute(&mut *conn)
    .await?
    .rows_affected();

    if inserted == 1 {
        return Ok(Begin::Proceed(Key { scope, key }));
    }

    let (hash, status, body): (String, Option<i32>, Option<serde_json::Value>) = sqlx::query_as(
        "SELECT request_hash, response_status, response_body FROM idempotency_keys WHERE scope = $1 AND key = $2",
    )
    .bind(scope)
    .bind(&key)
    .fetch_one(&mut *conn)
    .await?;

    if hash != request_hash {
        return Err(ApiError::invalid(
            "idempotency_key_reused",
            "this Idempotency-Key was already used with different parameters",
        ));
    }
    match (status, body) {
        (Some(s), Some(b)) => {
            let status = StatusCode::from_u16(s as u16).unwrap_or(StatusCode::OK);
            let mut res = (status, Json(b)).into_response();
            res.headers_mut()
                .insert("idempotent-replayed", "true".parse().unwrap());
            Ok(Begin::Replay(res))
        }
        _ => Err(ApiError::conflict(
            "request_in_progress",
            "a request with this Idempotency-Key is still in progress",
        )),
    }
}

/// Store the response so duplicates replay it. Returns the response to send.
pub async fn finish<T: Serialize>(
    conn: &mut PgConnection,
    key: Key,
    status: StatusCode,
    body: &T,
) -> Result<Response, ApiError> {
    let value = serde_json::to_value(body).expect("response serialises");
    sqlx::query("UPDATE idempotency_keys SET response_status = $3, response_body = $4 WHERE scope = $1 AND key = $2")
        .bind(key.scope)
        .bind(&key.key)
        .bind(status.as_u16() as i32)
        .bind(&value)
        .execute(conn)
        .await?;
    Ok((status, Json(value)).into_response())
}
