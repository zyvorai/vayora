//! Idempotency-Key handling inside the request's tenant-scoped transaction.
//! The key row is inserted with the work; concurrent duplicates block on the primary key, then replay.

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
    tenant_id: String,
    scope: &'static str,
    key: String,
}

pub enum Begin {
    /// Same key, same request: the stored response (and the resource it created, if any).
    Replay {
        status: StatusCode,
        body: Option<serde_json::Value>,
        resource_id: Option<String>,
    },
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
    tenant_id: &str,
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

    let inserted = sqlx::query("INSERT INTO idempotency_keys (tenant_id, scope, key, request_hash) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING")
        .bind(tenant_id)
        .bind(scope)
        .bind(&key)
        .bind(request_hash)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    if inserted == 1 {
        return Ok(Begin::Proceed(Key {
            tenant_id: tenant_id.into(),
            scope,
            key,
        }));
    }

    let (hash, status, body, resource_id): (String, Option<i32>, Option<serde_json::Value>, Option<String>) =
        sqlx::query_as("SELECT request_hash, response_status, response_body, resource_id FROM idempotency_keys WHERE tenant_id = $1 AND scope = $2 AND key = $3")
            .bind(tenant_id)
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
    Ok(Begin::Replay {
        status: status
            .and_then(|s| StatusCode::from_u16(s as u16).ok())
            .unwrap_or(StatusCode::CREATED),
        body,
        resource_id,
    })
}

impl Key {
    /// Remember which resource this key created, so a replay can return its *current* state.
    pub async fn set_resource(
        &self,
        conn: &mut PgConnection,
        resource_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE idempotency_keys SET resource_id = $4 WHERE tenant_id = $1 AND scope = $2 AND key = $3")
            .bind(&self.tenant_id)
            .bind(self.scope)
            .bind(&self.key)
            .bind(resource_id)
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn finish<T: Serialize>(
        self,
        conn: &mut PgConnection,
        status: StatusCode,
        body: &T,
    ) -> Result<Response, ApiError> {
        let value = serde_json::to_value(body).expect("response serialises");
        sqlx::query("UPDATE idempotency_keys SET response_status = $4, response_body = $5 WHERE tenant_id = $1 AND scope = $2 AND key = $3")
            .bind(&self.tenant_id)
            .bind(self.scope)
            .bind(&self.key)
            .bind(status.as_u16() as i32)
            .bind(&value)
            .execute(conn)
            .await?;
        Ok((status, Json(value)).into_response())
    }
}

/// Turn a stored replay into a response (marked so clients can tell).
pub fn replay_response(status: StatusCode, body: serde_json::Value) -> Response {
    let mut res = (status, Json(body)).into_response();
    res.headers_mut()
        .insert("idempotent-replayed", "true".parse().unwrap());
    res
}
