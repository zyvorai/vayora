//! Tamper-evident audit trail. Events are append-only already; sealing makes tampering *visible*:
//! each seal's hash commits to a range of events and to the previous seal, so a later edit,
//! deletion or back-dated insert changes a digest and `verify` says exactly where the chain breaks.

use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use crate::{ApiError, AppState, auth::Auth, tenant};

const GENESIS: &str = "genesis";

fn seal_hash(prev: &str, digest: &str, from: i64, to: i64) -> String {
    hex::encode(Sha256::digest(
        format!("{prev}:{digest}:{from}:{to}").as_bytes(),
    ))
}

/// Seal every event older than `min_age_secs` that isn't sealed yet. The delay keeps a transaction
/// that committed late from landing inside an already-sealed range. Returns events sealed.
pub async fn seal_tenant(
    conn: &mut PgConnection,
    tenant_id: &str,
    min_age_secs: i64,
) -> Result<i64, sqlx::Error> {
    // one sealer per tenant at a time
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("audit-seal:{tenant_id}"))
        .execute(&mut *conn)
        .await?;
    let last: Option<(i64, String)> = sqlx::query_as(
        "SELECT to_id, hash FROM audit_seals WHERE tenant_id = $1 ORDER BY to_id DESC LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (after, prev) = last.unwrap_or((0, GENESIS.into()));
    let range: (Option<i64>, Option<i64>, i64) = sqlx::query_as(
        "SELECT min(id), max(id), count(*) FROM audit_events
         WHERE tenant_id = $1 AND id > $2 AND created_at <= now() - make_interval(secs => $3::float8)",
    )
    .bind(tenant_id)
    .bind(after)
    .bind(min_age_secs as f64)
    .fetch_one(&mut *conn)
    .await?;
    let (Some(from), Some(to), n) = range else {
        return Ok(0);
    };
    let digest: String = sqlx::query_scalar("SELECT audit_range_digest($1, $2, $3)")
        .bind(tenant_id)
        .bind(from)
        .bind(to)
        .fetch_one(&mut *conn)
        .await?;
    sqlx::query("INSERT INTO audit_seals (tenant_id, from_id, to_id, row_count, prev_hash, hash) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(tenant_id)
        .bind(from)
        .bind(to)
        .bind(n)
        .bind(&prev)
        .bind(seal_hash(&prev, &digest, from, to))
        .execute(&mut *conn)
        .await?;
    Ok(n)
}

#[derive(Deserialize, Default)]
pub struct SealQuery {
    /// Seal events at least this old (default 120). Lower only if no request is in flight.
    min_age_seconds: Option<i64>,
}

/// `POST /v1/audit_events/seal`
pub async fn seal(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<SealQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = seal_tenant(
        &mut tx,
        &auth.tenant_id,
        q.min_age_seconds.unwrap_or(120).max(0),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "sealed_events": n })))
}

/// `GET /v1/audit_events/verify`: recompute every seal and the links between them.
pub async fn verify(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let seals: Vec<(i64, i64, i64, i64, String, String)> = sqlx::query_as(
        "SELECT id, from_id, to_id, row_count, prev_hash, hash FROM audit_seals WHERE tenant_id = $1 ORDER BY to_id",
    )
    .bind(&auth.tenant_id)
    .fetch_all(&mut *tx)
    .await?;
    let (mut prev, mut sealed) = (GENESIS.to_string(), 0i64);
    let mut last_to = 0i64;
    for (id, from, to, count, stored_prev, hash) in &seals {
        let fail = |why: &str| {
            Json(
                json!({ "ok": false, "seals_checked": seals.iter().position(|x| x.0 == *id).unwrap_or(0), "broken_seal": id, "events_range": [from, to], "reason": why }),
            )
        };
        if *stored_prev != prev {
            return Ok(fail("a seal was removed or reordered"));
        }
        let (n, digest): (i64, String) = sqlx::query_as(
            "SELECT count(*), audit_range_digest($1, $2, $3) FROM audit_events WHERE tenant_id = $1 AND id BETWEEN $2 AND $3",
        )
        .bind(&auth.tenant_id)
        .bind(from)
        .bind(to)
        .fetch_one(&mut *tx)
        .await?;
        if n != *count {
            return Ok(fail("events were added or deleted inside a sealed range"));
        }
        if seal_hash(&prev, &digest, *from, *to) != *hash {
            return Ok(fail("an event was changed after it was sealed"));
        }
        prev = hash.clone();
        sealed += n;
        last_to = *to;
    }
    let unsealed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE tenant_id = $1 AND id > $2")
            .bind(&auth.tenant_id)
            .bind(last_to)
            .fetch_one(&mut *tx)
            .await?;
    Ok(Json(
        json!({ "ok": true, "seals_checked": seals.len(), "sealed_events": sealed, "unsealed_events": unsealed, "head": prev }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_hash_commits_to_every_input() {
        let h = seal_hash("p", "d", 1, 2);
        assert_ne!(h, seal_hash("q", "d", 1, 2));
        assert_ne!(h, seal_hash("p", "e", 1, 2));
        assert_ne!(h, seal_hash("p", "d", 1, 3));
        assert_eq!(h, seal_hash("p", "d", 1, 2));
    }
}
