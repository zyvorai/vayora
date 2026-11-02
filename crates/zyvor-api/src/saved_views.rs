//! Saved views: named filter sets for a list page. Personal unless shared with the environment.
//! Only filters are stored, and they are checked on the way in, so a stored view can never carry
//! anything the list endpoint would not accept.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{ApiError, AppState, auth::Auth, tenant};
use zyvor_core::new_id;

const MAX_PER_OWNER: i64 = 20;
const STATUSES: [&str; 5] = [
    "requires_payment_method",
    "processing",
    "succeeded",
    "failed",
    "canceled",
];

/// Keep only the filters Payments understands, each checked. Unknown keys are refused, not dropped,
/// so a typo does not become a view that silently shows everything.
fn clean_payments_query(q: &Value) -> Result<Value, ApiError> {
    let bad = |m: &str| ApiError::invalid("invalid_view", m.to_string());
    let obj = q
        .as_object()
        .ok_or_else(|| bad("query must be an object"))?;
    let mut out = Map::new();
    for (k, v) in obj {
        let s = v.as_str().map(str::trim).filter(|s| !s.is_empty());
        let Some(s) = s else {
            return Err(bad(&format!("{k} must be a non-empty string")));
        };
        match k.as_str() {
            "status" if STATUSES.contains(&s) => {}
            "status" => return Err(bad("status is not a payment status")),
            "q" if s.chars().count() <= 100 => {}
            "q" => return Err(bad("q is at most 100 characters")),
            "from" | "to" if NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() => {}
            "from" | "to" => return Err(bad("from and to are dates, YYYY-MM-DD")),
            other => return Err(bad(&format!("unknown filter {other}"))),
        }
        out.insert(k.clone(), json!(s));
    }
    if let (Some(f), Some(t)) = (out.get("from"), out.get("to"))
        && f.as_str() > t.as_str()
    {
        return Err(bad("from is after to"));
    }
    if out.is_empty() {
        return Err(bad("a view needs at least one filter"));
    }
    Ok(Value::Object(out))
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub scope: Option<String>,
}

/// `GET /v1/saved_views?scope=payments`: your own views and the ones shared with the environment.
pub async fn list(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let scope = q.scope.unwrap_or_else(|| "payments".into());
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, Value, String, bool)> = sqlx::query_as(
        "SELECT id, name, query, owner, shared FROM saved_views
         WHERE scope = $1 AND (owner = $2 OR shared) ORDER BY lower(name), id",
    )
    .bind(&scope)
    .bind(&auth.actor)
    .fetch_all(&mut *tx)
    .await?;
    let data: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, query, owner, shared)| {
            json!({ "id": id, "name": name, "query": query, "shared": shared, "mine": owner == auth.actor, "owner": owner })
        })
        .collect();
    Ok(Json(json!({ "data": data })))
}

#[derive(Deserialize)]
pub struct Create {
    pub scope: Option<String>,
    pub name: String,
    pub query: Value,
    pub shared: Option<bool>,
}

/// `POST /v1/saved_views`
pub async fn create(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<Create>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payments:read")?;
    let scope = b.scope.unwrap_or_else(|| "payments".into());
    if scope != "payments" {
        return Err(ApiError::invalid(
            "invalid_view",
            "scope must be payments".to_string(),
        ));
    }
    let name = b.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::invalid(
            "invalid_view",
            "name must be 1 to 60 characters".to_string(),
        ));
    }
    let query = clean_payments_query(&b.query)?;
    let shared = b.shared.unwrap_or(false);
    if shared
        && !(auth
            .scopes
            .iter()
            .any(|x| x == "admin" || x == "payments:write"))
    {
        return Err(ApiError::forbidden(
            "sharing a view needs the payments:write scope".to_string(),
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let have: i64 =
        sqlx::query_scalar("SELECT count(*) FROM saved_views WHERE owner = $1 AND scope = $2")
            .bind(&auth.actor)
            .bind(&scope)
            .fetch_one(&mut *tx)
            .await?;
    if have >= MAX_PER_OWNER {
        return Err(ApiError::conflict(
            "too_many_views",
            format!("at most {MAX_PER_OWNER} saved views each; delete one first"),
        ));
    }
    let id = new_id("sv");
    sqlx::query("INSERT INTO saved_views (id, tenant_id, scope, name, query, owner, shared) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(&id)
        .bind(&auth.tenant_id)
        .bind(&scope)
        .bind(&name)
        .bind(&query)
        .bind(&auth.actor)
        .bind(shared)
        .execute(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => {
                ApiError::conflict("view_exists", "you already have a view with that name")
            }
            e => e.into(),
        })?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "name": name, "query": query, "shared": shared, "mine": true, "owner": auth.actor }),
        ),
    ))
}

/// `DELETE /v1/saved_views/{id}`: your own view, or any view if you are an admin.
pub async fn remove(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let owner: Option<String> = sqlx::query_scalar("SELECT owner FROM saved_views WHERE id = $1")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?;
    let owner = owner.ok_or_else(|| ApiError::not_found("saved view"))?;
    if owner != auth.actor && !auth.scopes.iter().any(|x| x == "admin") {
        return Err(ApiError::forbidden(
            "only the owner or an admin can delete this view".to_string(),
        ));
    }
    sqlx::query("DELETE FROM saved_views WHERE id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({ "ok": true })))
}
