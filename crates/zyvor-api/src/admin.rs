//! Merchant administration: organisations, scoped API keys, provider accounts, customers, invoices,
//! settings and the audit trail.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use zyvor_core::new_id;

use crate::{
    AppState,
    auth::{ALL_SCOPES, Auth, mint_key},
    crypto::Crypto,
    error::ApiError,
    payments::{ListQuery, refresh_invoice_status},
    tenant,
};

pub struct Bootstrapped {
    pub organisation_id: String,
    pub test_tenant: String,
    pub live_tenant: String,
    pub test_key: String,
    pub live_key: String,
}

/// Create an organisation with separate test and live tenants, each with an admin key.
/// The plaintext keys are returned once and never stored.
pub async fn bootstrap_org(pool: &PgPool, name: &str) -> Result<Bootstrapped, sqlx::Error> {
    let org = new_id("org");
    sqlx::query("INSERT INTO organisations (id, name) VALUES ($1, $2)")
        .bind(&org)
        .bind(name)
        .execute(pool)
        .await?;
    let mut out = Vec::new();
    for env in ["test", "live"] {
        let tid = new_id("ten");
        sqlx::query("INSERT INTO tenants (id, organisation_id, env) VALUES ($1, $2, $3)")
            .bind(&tid)
            .bind(&org)
            .bind(env)
            .execute(pool)
            .await?;
        let mut tx = tenant::begin(pool, &tid).await?;
        sqlx::query("INSERT INTO tenant_settings (tenant_id) VALUES ($1)")
            .bind(&tid)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let (key, prefix, hash) = mint_key(env);
        sqlx::query("INSERT INTO api_keys (id, tenant_id, name, prefix, key_hash, scopes) VALUES ($1, $2, 'bootstrap admin', $3, $4, ARRAY['admin'])").bind(new_id("key")).bind(&tid).bind(prefix).bind(hash).execute(pool).await?;
        out.push((tid, key));
    }
    let (live_tenant, live_key) = out.pop().unwrap();
    let (test_tenant, test_key) = out.pop().unwrap();
    Ok(Bootstrapped {
        organisation_id: org,
        test_tenant,
        live_tenant,
        test_key,
        live_key,
    })
}

// ---------- API keys ----------

#[derive(Deserialize)]
pub struct NewKey {
    name: String,
    scopes: Vec<String>,
}

pub async fn create_key(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewKey>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("admin")?;
    if b.scopes.is_empty() || b.scopes.iter().any(|sc| !ALL_SCOPES.contains(&sc.as_str())) {
        return Err(ApiError::invalid(
            "invalid_scopes",
            format!("scopes must be a non-empty subset of {ALL_SCOPES:?}"),
        ));
    }
    let (key, prefix, hash) = mint_key(&auth.env);
    let id = new_id("key");
    sqlx::query("INSERT INTO api_keys (id, tenant_id, name, prefix, key_hash, scopes) VALUES ($1, $2, $3, $4, $5, $6)").bind(&id).bind(&auth.tenant_id).bind(&b.name).bind(&prefix).bind(hash).bind(&b.scopes).execute(&s.pool).await?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "api_key.created",
        Some(&id),
        json!({ "name": b.name, "scopes": b.scopes }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "key": key, "prefix": prefix, "scopes": b.scopes, "note": "store this key now; it cannot be shown again" }),
        ),
    ))
}

pub async fn list_keys(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let rows: Vec<(String, String, String, Vec<String>, chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as("SELECT id, name, prefix, scopes, created_at, revoked_at FROM api_keys WHERE tenant_id = $1 ORDER BY created_at").bind(&auth.tenant_id).fetch_all(&s.pool).await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, name, prefix, scopes, c, r)| json!({ "id": id, "name": name, "prefix": prefix, "scopes": scopes, "created_at": c, "revoked_at": r })).collect::<Vec<_>>() }),
    ))
}

pub async fn revoke_key(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let n = sqlx::query("UPDATE api_keys SET revoked_at = now() WHERE id = $1 AND tenant_id = $2 AND revoked_at IS NULL").bind(&id).bind(&auth.tenant_id).execute(&s.pool).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("active key"));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "api_key.revoked",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "revoked": true })))
}

// ---------- provider accounts ----------

#[derive(Deserialize)]
pub struct NewProviderAccount {
    provider: String,
    label: Option<String>,
    /// razorpay: {key_id, key_secret}; cashfree: {client_id, client_secret}; sim: {}
    credentials: Value,
    webhook_secret: String,
    /// Sandbox or mock base URL override.
    base_url: Option<String>,
}

pub async fn create_provider_account(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewProviderAccount>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("admin")?;
    let required: &[&str] = match b.provider.as_str() {
        "razorpay" => &["key_id", "key_secret"],
        "cashfree" => &["client_id", "client_secret"],
        "sim" => &[],
        _ => {
            return Err(ApiError::invalid(
                "unknown_provider",
                "provider must be razorpay, cashfree or sim",
            ));
        }
    };
    if auth.env == "live" && b.provider == "sim" {
        return Err(ApiError::invalid(
            "sim_in_live",
            "the simulator cannot be used in a live environment",
        ));
    }
    if required
        .iter()
        .any(|k| b.credentials[k].as_str().is_none_or(str::is_empty))
    {
        return Err(ApiError::invalid(
            "missing_credentials",
            format!("credentials need {required:?}"),
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let id = new_id("pa");
    sqlx::query("INSERT INTO provider_accounts (id, tenant_id, provider, label, base_url, credentials_enc, webhook_secret_enc) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(&id)
        .bind(&auth.tenant_id)
        .bind(&b.provider)
        .bind(b.label.as_deref().unwrap_or(""))
        .bind(&b.base_url)
        .bind(s.crypto.encrypt(b.credentials.to_string().as_bytes()))
        .bind(s.crypto.encrypt(b.webhook_secret.as_bytes()))
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "provider_account.created",
        Some(&id),
        json!({ "provider": b.provider }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "provider": b.provider, "webhook_url": format!("/v1/webhooks/{}/{}", b.provider, auth.tenant_id) }),
        ),
    ))
}

pub async fn list_provider_accounts(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, bool, Option<String>)> = sqlx::query_as(
        "SELECT id, provider, label, enabled, base_url FROM provider_accounts ORDER BY provider",
    )
    .fetch_all(&mut *tx)
    .await?;
    // Secrets are never returned, only the capability matrix the router will use.
    let mut out = Vec::new();
    for (id, provider, label, enabled, base_url) in rows {
        let caps = crate::registry::load(&s, &mut tx, &auth.tenant_id, &auth.env, &provider)
            .await
            .map(|p| serde_json::to_value(p.capabilities()).unwrap())
            .unwrap_or(Value::Null);
        out.push(json!({ "id": id, "provider": provider, "label": label, "enabled": enabled, "base_url": base_url, "capabilities": caps, "webhook_url": format!("/v1/webhooks/{provider}/{}", auth.tenant_id) }));
    }
    Ok(Json(json!({ "data": out })))
}

#[derive(Deserialize)]
pub struct Toggle {
    enabled: bool,
}

pub async fn set_provider_enabled(
    State(s): State<AppState>,
    auth: Auth,
    Path(provider): Path<String>,
    Json(b): Json<Toggle>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("UPDATE provider_accounts SET enabled = $2 WHERE provider = $1")
        .bind(&provider)
        .bind(b.enabled)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("provider account"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "provider_account.toggled",
        Some(&provider),
        json!({ "enabled": b.enabled }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "provider": provider, "enabled": b.enabled })))
}

// ---------- customers, branches, users ----------

#[derive(Deserialize)]
pub struct NewCustomer {
    name: String,
    email: Option<String>,
    phone: Option<String>,
    external_ref: Option<String>,
}

pub async fn create_customer(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewCustomer>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("customers:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let id = new_id("cus");
    sqlx::query("INSERT INTO customers (id, tenant_id, name, email, phone, external_ref) VALUES ($1, $2, $3, $4, $5, $6)").bind(&id).bind(&auth.tenant_id).bind(&b.name).bind(&b.email).bind(&b.phone).bind(&b.external_ref).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "name": b.name, "email": b.email, "phone": b.phone, "external_ref": b.external_ref }),
        ),
    ))
}

pub async fn get_customer(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let r: Option<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as("SELECT id, name, email, phone, external_ref FROM customers WHERE id = $1")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?;
    let (id, name, email, phone, ext) = r.ok_or_else(|| ApiError::not_found("customer"))?;
    Ok(Json(
        json!({ "id": id, "name": name, "email": email, "phone": phone, "external_ref": ext }),
    ))
}

#[derive(Deserialize)]
pub struct NewBranch {
    name: String,
}

pub async fn create_branch(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewBranch>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let id = new_id("br");
    sqlx::query("INSERT INTO branches (id, tenant_id, name) VALUES ($1, $2, $3)")
        .bind(&id)
        .bind(&auth.tenant_id)
        .bind(&b.name)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "name": b.name })),
    ))
}

pub async fn list_branches(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, name FROM branches ORDER BY name")
        .fetch_all(&mut *tx)
        .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>() }),
    ))
}

#[derive(Deserialize)]
pub struct NewUser {
    email: String,
    role: String,
}

pub async fn create_user(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewUser>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("admin")?;
    if !matches!(
        b.role.as_str(),
        "owner" | "finance" | "support" | "developer"
    ) {
        return Err(ApiError::invalid(
            "invalid_role",
            "role must be owner, finance, support or developer",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let id = new_id("usr");
    sqlx::query("INSERT INTO users (id, tenant_id, email, role) VALUES ($1, $2, $3, $4)")
        .bind(&id)
        .bind(&auth.tenant_id)
        .bind(&b.email)
        .bind(&b.role)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "user.created",
        Some(&id),
        json!({ "role": b.role }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "email": b.email, "role": b.role })),
    ))
}

// ---------- invoices ----------

#[derive(Deserialize)]
pub struct NewInvoice {
    customer_id: Option<String>,
    number: String,
    currency: Option<String>,
    amount_minor: i64,
    /// Calculated by the billing/GST product; carried here for display and matching only.
    tax_minor: Option<i64>,
    external_ref: Option<String>,
    due_date: Option<NaiveDate>,
}

pub async fn create_invoice(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewInvoice>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("invoices:write")?;
    if b.amount_minor <= 0 {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_minor must be positive",
        ));
    }
    let ccy: zyvor_core::Currency = b
        .currency
        .as_deref()
        .unwrap_or("INR")
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let id = new_id("inv");
    sqlx::query("INSERT INTO invoices (id, tenant_id, customer_id, number, currency, amount_minor, tax_minor, external_ref, due_date) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)")
        .bind(&id)
        .bind(&auth.tenant_id)
        .bind(&b.customer_id)
        .bind(&b.number)
        .bind(ccy.code())
        .bind(b.amount_minor)
        .bind(b.tax_minor.unwrap_or(0))
        .bind(&b.external_ref)
        .bind(b.due_date)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "invoice.created",
        Some(&id),
        json!({ "number": b.number }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "number": b.number, "amount_minor": b.amount_minor, "status": "open" }),
        ),
    ))
}

pub async fn get_invoice(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    refresh_invoice_status(&mut tx, &id).await?;
    let r: Option<(String, String, String, i64, i64, Option<String>, String, Option<NaiveDate>)> = sqlx::query_as("SELECT id, number, currency, amount_minor, tax_minor, external_ref, status, due_date FROM invoices WHERE id = $1").bind(&id).fetch_optional(&mut *tx).await?;
    let (id, number, currency, amount, tax, ext, status, due) =
        r.ok_or_else(|| ApiError::not_found("invoice"))?;
    let allocs: Vec<(String, i64, String)> = sqlx::query_as("SELECT intent_id, amount_minor, reference FROM invoice_allocations WHERE invoice_id = $1 ORDER BY id").bind(&id).fetch_all(&mut *tx).await?;
    let paid: i64 = allocs.iter().map(|a| a.1).sum();
    let credited: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_minor), 0)::BIGINT FROM credit_notes WHERE invoice_id = $1",
    )
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    Ok(Json(json!({
        "id": id, "number": number, "currency": currency, "amount_minor": amount, "tax_minor": tax, "external_ref": ext, "status": status, "due_date": due,
        "paid_minor": paid, "credited_minor": credited, "outstanding_minor": amount - paid - credited,
        "allocations": allocs.into_iter().map(|(i, a, r)| json!({ "intent_id": i, "amount_minor": a, "reference": r })).collect::<Vec<_>>(),
    })))
}

pub async fn list_invoices(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, i64, String, Option<NaiveDate>, i64)> = sqlx::query_as(
        "SELECT id, number, amount_minor, status, due_date, invoice_settled(invoices.id)
         FROM invoices WHERE ($1::text IS NULL OR status = $1) ORDER BY created_at DESC LIMIT $2",
    )
    .bind(&q.status)
    .bind(q.limit.unwrap_or(50).clamp(1, 200))
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, number, amount, status, due, paid)| json!({ "id": id, "number": number, "amount_minor": amount, "paid_minor": paid, "outstanding_minor": amount - paid, "status": status, "due_date": due })).collect::<Vec<_>>() }),
    ))
}

// ---------- settings and audit ----------

#[derive(Deserialize)]
pub struct Settings {
    refund_approval_threshold_minor: Option<i64>,
    routing: Option<Value>,
    outbound_webhook_url: Option<String>,
    outbound_webhook_secret: Option<String>,
    dunning: Option<DunningPolicy>,
}

/// Automatic reminders for overdue invoices; off until a tenant turns it on.
#[derive(Deserialize)]
pub struct DunningPolicy {
    enabled: Option<bool>,
    channel: Option<String>,
    min_days_overdue: Option<i32>,
    repeat_after_days: Option<i32>,
}

pub async fn put_settings(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<Settings>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    if matches!(b.refund_approval_threshold_minor, Some(t) if t < 0) {
        return Err(ApiError::invalid(
            "invalid_threshold",
            "threshold must not be negative",
        ));
    }
    if let Some(r) = &b.routing {
        serde_json::from_value::<crate::registry::Routing>(r.clone())
            .map_err(|e| ApiError::invalid("invalid_routing", e.to_string()))?;
    }
    if let Some(d) = &b.dunning {
        let ok = d
            .channel
            .as_deref()
            .is_none_or(|c| ["email", "sms", "whatsapp", "call", "other"].contains(&c))
            && d.min_days_overdue.is_none_or(|n| n >= 0)
            && d.repeat_after_days.is_none_or(|n| (0..=365).contains(&n));
        if !ok {
            return Err(ApiError::invalid(
                "invalid_dunning",
                "channel must be email, sms, whatsapp, call or other; min_days_overdue >= 0; repeat_after_days 0..365",
            ));
        }
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    if let Some(d) = &b.dunning {
        sqlx::query("UPDATE tenant_settings SET auto_dunning = COALESCE($2, auto_dunning), dunning_channel = COALESCE($3, dunning_channel), dunning_min_days = COALESCE($4, dunning_min_days), dunning_repeat_days = COALESCE($5, dunning_repeat_days) WHERE tenant_id = $1")
            .bind(&auth.tenant_id).bind(d.enabled).bind(&d.channel).bind(d.min_days_overdue).bind(d.repeat_after_days)
            .execute(&mut *tx).await?;
    }
    sqlx::query(
        "UPDATE tenant_settings SET refund_approval_threshold_minor = COALESCE($2, refund_approval_threshold_minor), routing = COALESCE($3, routing),
                outbound_webhook_url = COALESCE($4, outbound_webhook_url), outbound_webhook_secret_enc = COALESCE($5, outbound_webhook_secret_enc) WHERE tenant_id = $1",
    )
    .bind(&auth.tenant_id)
    .bind(b.refund_approval_threshold_minor)
    .bind(&b.routing)
    .bind(&b.outbound_webhook_url)
    .bind(b.outbound_webhook_secret.as_ref().map(|sec| s.crypto.encrypt(sec.as_bytes())))
    .execute(&mut *tx)
    .await?;
    tenant::audit(&mut tx, &auth.tenant_id, &auth.actor, "settings.updated", None, json!({ "threshold": b.refund_approval_threshold_minor, "routing_changed": b.routing.is_some(), "webhook_url": b.outbound_webhook_url, "dunning_changed": b.dunning.is_some() })).await?;
    tx.commit().await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn get_settings(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (thr, routing, url, on, ch, min, rep): (i64, Value, Option<String>, bool, String, i32, i32) = sqlx::query_as("SELECT refund_approval_threshold_minor, routing, outbound_webhook_url, auto_dunning, dunning_channel, dunning_min_days, dunning_repeat_days FROM tenant_settings WHERE tenant_id = $1").bind(&auth.tenant_id).fetch_one(&mut *tx).await?;
    Ok(Json(
        json!({ "refund_approval_threshold_minor": thr, "routing": routing, "outbound_webhook_url": url, "env": auth.env,
                "dunning": { "enabled": on, "channel": ch, "min_days_overdue": min, "repeat_after_days": rep } }),
    ))
}

pub async fn audit_log(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(i64, String, String, Option<String>, Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as("SELECT id, actor, action, subject, detail, created_at FROM audit_events ORDER BY id DESC LIMIT $1").bind(q.limit.unwrap_or(50).clamp(1, 200)).fetch_all(&mut *tx).await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, actor, action, subject, detail, at)| json!({ "id": id, "actor": actor, "action": action, "subject": subject, "detail": detail, "created_at": at })).collect::<Vec<_>>() }),
    ))
}

/// Everything this tenant owns, as one JSON document. Portability of business records.
pub async fn export(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mut out = serde_json::Map::new();
    // Table names are fixed literals; row-level security scopes each to this tenant.
    for (name, sql) in [
        (
            "customers",
            "SELECT to_jsonb(t) FROM customers t ORDER BY id",
        ),
        ("invoices", "SELECT to_jsonb(t) FROM invoices t ORDER BY id"),
        (
            "payment_intents",
            "SELECT to_jsonb(t) FROM payment_intents t ORDER BY id",
        ),
        (
            "payment_attempts",
            "SELECT to_jsonb(t) - 'raw' FROM payment_attempts t ORDER BY id",
        ),
        ("refunds", "SELECT to_jsonb(t) FROM refunds t ORDER BY id"),
        (
            "ledger_entries",
            "SELECT to_jsonb(t) FROM ledger_entries t ORDER BY id",
        ),
        (
            "settlements",
            "SELECT to_jsonb(t) FROM settlements t ORDER BY id",
        ),
        (
            "recon_cases",
            "SELECT to_jsonb(t) FROM recon_cases t ORDER BY id",
        ),
        (
            "audit_events",
            "SELECT to_jsonb(t) FROM audit_events t ORDER BY id",
        ),
    ] {
        let rows: Vec<Value> = sqlx::query_scalar(sql).fetch_all(&mut *tx).await?;
        out.insert(name.to_string(), Value::Array(rows));
    }
    out.insert("exported_at".into(), json!(chrono::Utc::now()));
    out.insert("tenant_id".into(), json!(auth.tenant_id));
    Ok(Json(Value::Object(out)))
}

pub fn _unused(_: &Crypto) {}

// ---------- outbound webhook inspector and replay ----------

pub async fn list_outbox(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, i32, Option<String>, Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, type, status, attempts, last_error, payload, created_at FROM outbox_events WHERE ($1::text IS NULL OR status = $1) ORDER BY created_at DESC, id DESC LIMIT $2",
    )
    .bind(&q.status)
    .bind(q.limit.unwrap_or(50).clamp(1, 200))
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, ty, status, attempts, err, payload, at)| json!({ "id": id, "type": ty, "status": status, "attempts": attempts, "last_error": err, "payload": payload, "created_at": at })).collect::<Vec<_>>() }),
    ))
}

/// Re-send an event (even a delivered one). Consumers de-duplicate on the event id, so replay is safe.
pub async fn replay_outbox(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("UPDATE outbox_events SET status = 'pending', attempts = 0, next_attempt_at = now(), last_error = NULL WHERE id = $1").bind(&id).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("event"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "outbox.replayed",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "pending" })))
}

/// What the platform has received from providers (the inbound inspector).
pub async fn list_inbox(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(i64, String, String, String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as("SELECT id, provider, event_id, kind, outcome, received_at FROM webhook_inbox ORDER BY id DESC LIMIT $1").bind(q.limit.unwrap_or(50).clamp(1, 200)).fetch_all(&mut *tx).await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, provider, event_id, kind, outcome, at)| json!({ "id": id, "provider": provider, "event_id": event_id, "kind": kind, "outcome": outcome, "received_at": at })).collect::<Vec<_>>() }),
    ))
}
