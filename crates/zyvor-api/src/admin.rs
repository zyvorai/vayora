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
    /// The key stops working at this time.
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// If set, the key only works from these addresses (CIDR or single IP).
    #[serde(default)]
    allowed_cidrs: Vec<String>,
    /// Pin the key to one branch (it then cannot hold the `admin` scope).
    branch_id: Option<String>,
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
    if b.expires_at.is_some_and(|e| e <= chrono::Utc::now()) {
        return Err(ApiError::invalid(
            "invalid_expiry",
            "expires_at must be in the future",
        ));
    }
    if b.allowed_cidrs.len() > 50
        || b.allowed_cidrs
            .iter()
            .any(|c| crate::auth::parse_cidr(c).is_none())
    {
        return Err(ApiError::invalid(
            "invalid_cidr",
            "allowed_cidrs must be at most 50 valid CIDRs or IP addresses",
        ));
    }
    if let Some(br) = &b.branch_id {
        if b.scopes.iter().any(|sc| sc == "admin") {
            return Err(ApiError::invalid(
                "invalid_scopes",
                "a branch key cannot hold the admin scope",
            ));
        }
        // checked as this tenant: foreign keys ignore row-level security
        let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
        let seen: Option<String> = sqlx::query_scalar("SELECT id FROM branches WHERE id = $1")
            .bind(br)
            .fetch_optional(&mut *tx)
            .await?;
        seen.ok_or_else(|| ApiError::not_found("branch"))?;
    }
    let (key, prefix, hash) = mint_key(&auth.env);
    let id = new_id("key");
    sqlx::query("INSERT INTO api_keys (id, tenant_id, name, prefix, key_hash, scopes, expires_at, allowed_cidrs, branch_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)").bind(&id).bind(&auth.tenant_id).bind(&b.name).bind(&prefix).bind(hash).bind(&b.scopes).bind(b.expires_at).bind(&b.allowed_cidrs).bind(&b.branch_id).execute(&s.pool).await?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "api_key.created",
        Some(&id),
        json!({ "name": b.name, "scopes": b.scopes, "expires_at": b.expires_at, "allowed_cidrs": b.allowed_cidrs }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "key": key, "prefix": prefix, "scopes": b.scopes, "expires_at": b.expires_at, "allowed_cidrs": b.allowed_cidrs, "note": "store this key now; it cannot be shown again" }),
        ),
    ))
}

pub async fn list_keys(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    type Ts = Option<chrono::DateTime<chrono::Utc>>;
    let rows: Vec<(String, String, String, Vec<String>, chrono::DateTime<chrono::Utc>, Ts, Ts, Ts, Vec<String>, Option<String>)> =
        sqlx::query_as("SELECT id, name, prefix, scopes, created_at, revoked_at, expires_at, last_used_at, allowed_cidrs, branch_id FROM api_keys WHERE tenant_id = $1 ORDER BY created_at").bind(&auth.tenant_id).fetch_all(&s.pool).await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, name, prefix, scopes, c, r, e, u, cidrs, branch)| json!({ "branch_id": branch, "id": id, "name": name, "prefix": prefix, "scopes": scopes, "created_at": c, "revoked_at": r, "expires_at": e, "last_used_at": u, "allowed_cidrs": cidrs })).collect::<Vec<_>>() }),
    ))
}

#[derive(Deserialize, Default)]
pub struct RotateBody {
    /// How long the old key keeps working so deployments can switch over (default 24, max 720).
    overlap_hours: Option<i64>,
}

/// Mint a replacement with the same name, scopes and allow-list; the old key expires after the overlap.
pub async fn rotate_key(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Option<Json<RotateBody>>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("admin")?;
    let hours = body.and_then(|b| b.0.overlap_hours).unwrap_or(24);
    if !(0..=720).contains(&hours) {
        return Err(ApiError::invalid(
            "invalid_overlap",
            "overlap_hours must be between 0 and 720",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let old: Option<(String, Vec<String>, Vec<String>, Option<String>)> = sqlx::query_as(
        "SELECT name, scopes, allowed_cidrs, branch_id FROM api_keys WHERE id = $1 AND tenant_id = $2 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now()) FOR UPDATE",
    )
    .bind(&id)
    .bind(&auth.tenant_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (name, scopes, cidrs, branch) = old.ok_or_else(|| ApiError::not_found("active key"))?;
    let (key, prefix, hash) = mint_key(&auth.env);
    let new_id_ = new_id("key");
    sqlx::query("INSERT INTO api_keys (id, tenant_id, name, prefix, key_hash, scopes, allowed_cidrs, rotated_from, branch_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)")
        .bind(&new_id_).bind(&auth.tenant_id).bind(&name).bind(&prefix).bind(hash).bind(&scopes).bind(&cidrs).bind(&id).bind(&branch)
        .execute(&mut *tx).await?;
    sqlx::query("UPDATE api_keys SET expires_at = LEAST(COALESCE(expires_at, 'infinity'), now() + make_interval(hours => $2::int)) WHERE id = $1")
        .bind(&id).bind(hours as i32).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "api_key.rotated",
        Some(&id),
        json!({ "new_key": new_id_, "overlap_hours": hours }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": new_id_, "key": key, "prefix": prefix, "scopes": scopes, "rotated_from": id, "old_key_expires_in_hours": hours, "note": "store this key now; it cannot be shown again" }),
        ),
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
    /// Taken from the path on `PUT /v1/provider_accounts/{provider}`.
    #[serde(default)]
    provider: String,
    label: Option<String>,
    /// razorpay: {key_id, key_secret}; cashfree: {client_id, client_secret}; sim: {}
    credentials: Value,
    webhook_secret: String,
    /// Sandbox or mock base URL override.
    base_url: Option<String>,
}

/// What must hold before provider credentials are stored, for a new account or a replacement.
fn check_provider(s: &AppState, auth: &Auth, b: &NewProviderAccount) -> Result<(), ApiError> {
    let required: &[&str] = match b.provider.as_str() {
        "razorpay" => &["key_id", "key_secret"],
        "cashfree" => &["client_id", "client_secret"],
        "custom" => &["api_key"],
        "sim" => &[],
        _ => {
            return Err(ApiError::invalid(
                "unknown_provider",
                "provider must be razorpay, cashfree, custom or sim",
            ));
        }
    };
    if b.provider == "custom" {
        // Your own gateway: it has no default URL, and it must be a public https endpoint.
        let url = b.base_url.as_deref().ok_or_else(|| {
            ApiError::invalid("missing_base_url", "the custom provider needs a base_url")
        })?;
        crate::ssrf::check_url(url, s.allow_private_targets)
            .map_err(|e| ApiError::invalid("bad_base_url", e))?;
    }
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
    if b.webhook_secret.is_empty() {
        return Err(ApiError::invalid(
            "missing_webhook_secret",
            "give the webhook secret the provider signs its callbacks with",
        ));
    }
    Ok(())
}

pub async fn create_provider_account(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewProviderAccount>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("admin")?;
    check_provider(&s, &auth, &b)?;
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

/// `PUT /v1/provider_accounts/{provider}`: store these credentials for the provider, replacing the account if there
/// is one (how a simulator account becomes a real one), enabled. Leave `base_url` out to use the provider's own
/// test or live address for this environment. Secrets are never returned.
pub async fn replace_provider_account(
    State(s): State<AppState>,
    auth: Auth,
    Path(provider): Path<String>,
    Json(mut b): Json<NewProviderAccount>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    b.provider = provider.clone();
    check_provider(&s, &auth, &b)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM provider_accounts WHERE provider = $1")
            .bind(&provider)
            .fetch_optional(&mut *tx)
            .await?;
    let creds = s.crypto.encrypt(b.credentials.to_string().as_bytes());
    let hook = s.crypto.encrypt(b.webhook_secret.as_bytes());
    let (id, action) = match existing {
        Some(id) => {
            sqlx::query("UPDATE provider_accounts SET label = $2, base_url = $3, credentials_enc = $4, webhook_secret_enc = $5, enabled = true WHERE id = $1")
                .bind(&id).bind(b.label.as_deref().unwrap_or("")).bind(&b.base_url).bind(creds).bind(hook)
                .execute(&mut *tx).await?;
            (id, "provider_account.replaced")
        }
        None => {
            let id = new_id("pa");
            sqlx::query("INSERT INTO provider_accounts (id, tenant_id, provider, label, base_url, credentials_enc, webhook_secret_enc) VALUES ($1, $2, $3, $4, $5, $6, $7)")
                .bind(&id).bind(&auth.tenant_id).bind(&provider).bind(b.label.as_deref().unwrap_or("")).bind(&b.base_url).bind(creds).bind(hook)
                .execute(&mut *tx).await?;
            (id, "provider_account.created")
        }
    };
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        action,
        Some(&id),
        json!({ "provider": provider, "base_url": b.base_url }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "id": id, "provider": provider, "enabled": true, "replaced": action.ends_with("replaced") }),
    ))
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
    /// Buyer's GSTIN, used as the default on tax invoices.
    gstin: Option<String>,
    /// Buyer's address, needed only for a draft e-invoice.
    address1: Option<String>,
    location: Option<String>,
    pincode: Option<String>,
}

pub async fn create_customer(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewCustomer>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("customers:write")?;
    if b.gstin
        .as_deref()
        .is_some_and(|g| !crate::gst::valid_gstin(g))
    {
        return Err(ApiError::invalid("invalid_gstin", "GSTIN is not valid"));
    }
    crate::gst::check_address(
        b.address1.as_deref(),
        b.location.as_deref(),
        b.pincode.as_deref(),
    )?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let id = new_id("cus");
    sqlx::query("INSERT INTO customers (id, tenant_id, name, email, phone, external_ref, gstin, address1, location, pincode) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)").bind(&id).bind(&auth.tenant_id).bind(&b.name).bind(&b.email).bind(&b.phone).bind(&b.external_ref).bind(&b.gstin).bind(&b.address1).bind(&b.location).bind(&b.pincode).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "name": b.name, "email": b.email, "phone": b.phone, "external_ref": b.external_ref, "gstin": b.gstin }),
        ),
    ))
}

/// Fields to change on a customer. A field left out stays; an empty string clears it (name cannot be cleared).
#[derive(Deserialize)]
pub struct CustomerPatch {
    name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    gstin: Option<String>,
    address1: Option<String>,
    location: Option<String>,
    pincode: Option<String>,
}

/// `PATCH /v1/customers/{id}`: correct a customer's details. Invoices already issued keep the GSTIN they were
/// issued with (it is copied onto the invoice), so fixing a typo here does not rewrite history.
pub async fn update_customer(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<CustomerPatch>,
) -> Result<Json<Value>, ApiError> {
    auth.require("customers:write")?;
    if b.name.as_deref().is_some_and(|n| n.trim().is_empty()) {
        return Err(ApiError::invalid("invalid_name", "name cannot be empty"));
    }
    let g = b.gstin.as_deref().map(str::trim).filter(|g| !g.is_empty());
    if g.is_some_and(|g| !crate::gst::valid_gstin(g)) {
        return Err(ApiError::invalid("invalid_gstin", "GSTIN is not valid"));
    }
    let nz = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(str::to_string)
    };
    crate::gst::check_address(
        nz(&b.address1).as_deref(),
        nz(&b.location).as_deref(),
        nz(&b.pincode).as_deref(),
    )?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let changed: Vec<&str> = [
        ("name", b.name.is_some()),
        ("email", b.email.is_some()),
        ("phone", b.phone.is_some()),
        ("gstin", b.gstin.is_some()),
        ("address1", b.address1.is_some()),
        ("location", b.location.is_some()),
        ("pincode", b.pincode.is_some()),
    ]
    .into_iter()
    .filter(|(_, c)| *c)
    .map(|(n, _)| n)
    .collect();
    // `$n IS NULL` keeps the column; an empty string becomes NULL
    let done = sqlx::query(
        "UPDATE customers SET name = COALESCE($2, name),
            email = CASE WHEN $3::text IS NULL THEN email ELSE NULLIF(btrim($3), '') END,
            phone = CASE WHEN $4::text IS NULL THEN phone ELSE NULLIF(btrim($4), '') END,
            gstin = CASE WHEN $5::text IS NULL THEN gstin ELSE NULLIF(btrim($5), '') END,
            address1 = CASE WHEN $6::text IS NULL THEN address1 ELSE NULLIF(btrim($6), '') END,
            location = CASE WHEN $7::text IS NULL THEN location ELSE NULLIF(btrim($7), '') END,
            pincode = CASE WHEN $8::text IS NULL THEN pincode ELSE NULLIF(btrim($8), '') END
         WHERE id = $1",
    )
    .bind(&id)
    .bind(b.name.as_deref().map(str::trim))
    .bind(&b.email)
    .bind(&b.phone)
    .bind(&b.gstin)
    .bind(&b.address1)
    .bind(&b.location)
    .bind(&b.pincode)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("customer"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "customer.updated",
        Some(&id),
        json!({ "fields": changed }),
    )
    .await?;
    tx.commit().await?;
    get_customer(State(s), auth, Path(id)).await
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
    let (gstin, address1, location, pincode): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as("SELECT gstin, address1, location, pincode FROM customers WHERE id = $1")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
    Ok(Json(
        json!({ "id": id, "name": name, "email": email, "phone": phone, "external_ref": ext, "gstin": gstin, "address1": address1, "location": location, "pincode": pincode }),
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
    /// Required unless `lines` is given, in which case it is computed (and must match if you send it).
    amount_minor: Option<i64>,
    /// Tax carried for display and matching when you calculate it yourself. Ignored with `lines`.
    tax_minor: Option<i64>,
    external_ref: Option<String>,
    due_date: Option<NaiveDate>,
    /// GST tax invoice lines (HSN/SAC, quantity, unit price, rate). Needs your GSTIN set in settings.
    lines: Option<Vec<crate::gst::LineIn>>,
    /// 2-digit state code; defaults to the buyer GSTIN's state.
    place_of_supply: Option<String>,
    buyer_gstin: Option<String>,
    reverse_charge: Option<bool>,
}

pub async fn create_invoice(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewInvoice>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("invoices:write")?;
    let ccy: zyvor_core::Currency = b
        .currency
        .as_deref()
        .unwrap_or("INR")
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    if let Some(c) = &b.customer_id {
        // foreign keys ignore row-level security, so make sure the customer is this tenant's
        let seen: Option<String> = sqlx::query_scalar("SELECT id FROM customers WHERE id = $1")
            .bind(c)
            .fetch_optional(&mut *tx)
            .await?;
        seen.ok_or_else(|| ApiError::not_found("customer"))?;
    }
    let id = new_id("inv");
    // GST mode: amount and tax come from the lines
    let gst = match &b.lines {
        Some(lines) => {
            if ccy.code() != "INR" {
                return Err(ApiError::invalid(
                    "invalid_currency",
                    "GST invoices are in INR",
                ));
            }
            crate::gst::validate_lines(lines)?;
            let (seller, pos, buyer) = crate::gst::parties(
                &mut tx,
                b.place_of_supply.as_deref(),
                b.buyer_gstin.as_deref(),
                b.customer_id.as_deref(),
            )
            .await?;
            let c = crate::gst::compute(lines, &seller, &pos);
            if b.amount_minor.is_some_and(|a| a != c.total) {
                return Err(ApiError::invalid(
                    "amount_mismatch",
                    format!("the lines add up to {} minor units including tax", c.total),
                ));
            }
            Some((c, pos, buyer))
        }
        None => None,
    };
    let (amount, tax) = match &gst {
        Some((c, _, _)) => (c.total, c.tax),
        None => (b.amount_minor.unwrap_or(0), b.tax_minor.unwrap_or(0)),
    };
    if amount <= 0 || amount > zyvor_core::MAX_AMOUNT_MINOR {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_minor must be positive and within the supported limit",
        ));
    }
    sqlx::query("INSERT INTO invoices (id, tenant_id, customer_id, number, currency, amount_minor, tax_minor, external_ref, due_date, place_of_supply, buyer_gstin, reverse_charge) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)")
        .bind(&id)
        .bind(&auth.tenant_id)
        .bind(&b.customer_id)
        .bind(&b.number)
        .bind(ccy.code())
        .bind(amount)
        .bind(tax)
        .bind(&b.external_ref)
        .bind(b.due_date)
        .bind(gst.as_ref().map(|g| g.1.clone()))
        .bind(gst.as_ref().and_then(|g| g.2.clone()))
        .bind(b.reverse_charge.unwrap_or(false))
        .execute(&mut *tx)
        .await?;
    if let (Some((c, _, _)), Some(lines)) = (&gst, &b.lines) {
        for (n, (l, o)) in lines.iter().zip(&c.lines).enumerate() {
            sqlx::query("INSERT INTO invoice_lines (id, tenant_id, invoice_id, position, description, hsn_sac, quantity, unit_price_minor, taxable_minor, rate_bp, cgst_minor, sgst_minor, igst_minor) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)")
                .bind(new_id("iln")).bind(&auth.tenant_id).bind(&id).bind(n as i32 + 1).bind(l.description.trim()).bind(&l.hsn_sac)
                .bind(l.quantity).bind(l.unit_price_minor).bind(o.taxable).bind(o.rate_bp).bind(o.cgst).bind(o.sgst).bind(o.igst)
                .execute(&mut *tx).await?;
        }
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "invoice.created",
        Some(&id),
        json!({ "number": b.number, "gst": gst.is_some() }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "number": b.number, "amount_minor": amount, "tax_minor": tax, "status": "open" }),
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
    let (customer_id, pos, buyer_gstin, reverse, issued): (Option<String>, Option<String>, Option<String>, bool, NaiveDate) = sqlx::query_as("SELECT customer_id, place_of_supply, buyer_gstin, reverse_charge, (created_at AT TIME ZONE 'Asia/Kolkata')::date FROM invoices WHERE id = $1").bind(&id).fetch_one(&mut *tx).await?;
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
        "customer_id": customer_id, "place_of_supply": pos, "buyer_gstin": buyer_gstin, "reverse_charge": reverse, "issued_on": issued,
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
    /// Payouts up to this amount skip the second person's approval. Default 0: every payout needs one.
    payout_approval_threshold_minor: Option<i64>,
    /// Event types to deliver: exact types, families like `payment.*`, or `*` (the default). See `GET /v1/events/catalogue`.
    webhook_events: Option<Vec<String>>,
    dunning: Option<DunningPolicy>,
    gst: Option<GstProfile>,
    /// Limits that stop a payment: see `GET /v1/settings`.
    risk: Option<crate::risk::Rules>,
}

/// Your GST registration, needed to issue tax invoices.
#[derive(Deserialize)]
pub struct GstProfile {
    gstin: String,
    legal_name: Option<String>,
    /// Your registered address, needed only for a draft e-invoice.
    address1: Option<String>,
    location: Option<String>,
    pincode: Option<String>,
}

/// Automatic reminders for overdue invoices; off until a tenant turns it on.
#[derive(Deserialize)]
pub struct DunningPolicy {
    enabled: Option<bool>,
    channel: Option<String>,
    min_days_overdue: Option<i32>,
    repeat_after_days: Option<i32>,
    /// Draft reminders for a person to approve instead of sending them straight away.
    requires_approval: Option<bool>,
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
    if let Some(r) = &b.risk {
        crate::risk::validate(r)?;
    }
    if let Some(ev) = &b.webhook_events
        && (ev.is_empty() || ev.len() > 50 || ev.iter().any(|p| !crate::events::valid_pattern(p)))
    {
        return Err(ApiError::invalid(
            "invalid_webhook_events",
            "give 1-50 entries from GET /v1/events/catalogue, families like payment.*, or *",
        ));
    }
    if b.payout_approval_threshold_minor.is_some_and(|t| t < 0) {
        return Err(ApiError::invalid(
            "invalid_threshold",
            "threshold must not be negative",
        ));
    }
    if let Some(g) = &b.gst
        && !crate::gst::valid_gstin(&g.gstin)
    {
        return Err(ApiError::invalid(
            "invalid_gstin",
            "gstin is not valid (15 characters, valid state code and check character)",
        ));
    }
    if let Some(g) = &b.gst {
        crate::gst::check_address(
            g.address1.as_deref(),
            g.location.as_deref(),
            g.pincode.as_deref(),
        )?;
    }
    if let Some(u) = &b.outbound_webhook_url {
        crate::ssrf::check_url(u, s.allow_private_targets)
            .map_err(|e| ApiError::invalid("invalid_webhook_url", e))?;
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
    if let Some(r) = &b.risk {
        sqlx::query("UPDATE tenant_settings SET risk_rules = $2 WHERE tenant_id = $1")
            .bind(&auth.tenant_id)
            .bind(serde_json::to_value(r).unwrap_or_default())
            .execute(&mut *tx)
            .await?;
    }
    if let Some(ev) = &b.webhook_events {
        // "*" alone means everything again
        let stored: Option<&Vec<String>> = if ev.iter().any(|p| p == "*") {
            None
        } else {
            Some(ev)
        };
        sqlx::query("UPDATE tenant_settings SET webhook_events = $2 WHERE tenant_id = $1")
            .bind(&auth.tenant_id)
            .bind(stored)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(t) = b.payout_approval_threshold_minor {
        sqlx::query(
            "UPDATE tenant_settings SET payout_approval_threshold_minor = $2 WHERE tenant_id = $1",
        )
        .bind(&auth.tenant_id)
        .bind(t)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(g) = &b.gst {
        sqlx::query("UPDATE tenant_settings SET gstin = $2, gst_legal_name = COALESCE($3, gst_legal_name), gst_address1 = COALESCE($4, gst_address1), gst_location = COALESCE($5, gst_location), gst_pincode = COALESCE($6, gst_pincode) WHERE tenant_id = $1")
            .bind(&auth.tenant_id).bind(&g.gstin).bind(&g.legal_name).bind(&g.address1).bind(&g.location).bind(&g.pincode).execute(&mut *tx).await?;
    }
    if let Some(d) = &b.dunning {
        sqlx::query("UPDATE tenant_settings SET auto_dunning = COALESCE($2, auto_dunning), dunning_channel = COALESCE($3, dunning_channel), dunning_min_days = COALESCE($4, dunning_min_days), dunning_repeat_days = COALESCE($5, dunning_repeat_days), dunning_requires_approval = COALESCE($6, dunning_requires_approval) WHERE tenant_id = $1")
            .bind(&auth.tenant_id).bind(d.enabled).bind(&d.channel).bind(d.min_days_overdue).bind(d.repeat_after_days).bind(d.requires_approval)
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
    let (thr, routing, url, on, ch, min, rep, appr, gstin, legal, ptr, wev, risk): (i64, Value, Option<String>, bool, String, i32, i32, bool, Option<String>, Option<String>, i64, Option<Vec<String>>, Value) = sqlx::query_as("SELECT refund_approval_threshold_minor, routing, outbound_webhook_url, auto_dunning, dunning_channel, dunning_min_days, dunning_repeat_days, dunning_requires_approval, gstin, gst_legal_name, payout_approval_threshold_minor, webhook_events, risk_rules FROM tenant_settings WHERE tenant_id = $1").bind(&auth.tenant_id).fetch_one(&mut *tx).await?;
    let (gaddr, gloc, gpin): (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT gst_address1, gst_location, gst_pincode FROM tenant_settings WHERE tenant_id = $1",
    )
    .bind(&auth.tenant_id)
    .fetch_one(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "refund_approval_threshold_minor": thr, "routing": routing, "outbound_webhook_url": url, "env": auth.env,
                "dunning": { "enabled": on, "channel": ch, "min_days_overdue": min, "repeat_after_days": rep, "requires_approval": appr }, "gst": { "gstin": gstin, "legal_name": legal, "address1": gaddr, "location": gloc, "pincode": gpin }, "payout_approval_threshold_minor": ptr, "risk": risk, "webhook_events": wev.unwrap_or_else(|| vec!["*".to_string()]) }),
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

/// Re-seal every stored secret under the current master key. Returns `(resealed, already_current, unreadable)`.
/// Unreadable blobs (no configured key opens them) are left untouched and counted, never overwritten.
pub async fn rekey_all(
    pool: &sqlx::PgPool,
    crypto: &crate::Crypto,
    dry_run: bool,
) -> Result<(u64, u64, u64), sqlx::Error> {
    let tenants: Vec<String> = sqlx::query_scalar("SELECT id FROM tenants")
        .fetch_all(pool)
        .await?;
    let (mut done, mut current, mut bad) = (0u64, 0u64, 0u64);
    for t in tenants {
        let mut tx = tenant::begin(pool, &t).await?;
        let accounts: Vec<(String, Vec<u8>, Vec<u8>)> =
            sqlx::query_as("SELECT id, credentials_enc, webhook_secret_enc FROM provider_accounts")
                .fetch_all(&mut *tx)
                .await?;
        for (id, cred, hook) in accounts {
            let mut next = [None, None];
            for (i, blob) in [&cred, &hook].into_iter().enumerate() {
                if !crypto.needs_rekey(blob) {
                    current += 1;
                } else if let Some(n) = crypto.rekey(blob) {
                    next[i] = Some(n);
                    done += 1;
                } else {
                    bad += 1;
                }
            }
            if !dry_run && (next[0].is_some() || next[1].is_some()) {
                sqlx::query("UPDATE provider_accounts SET credentials_enc = COALESCE($2, credentials_enc), webhook_secret_enc = COALESCE($3, webhook_secret_enc) WHERE id = $1")
                    .bind(&id).bind(&next[0]).bind(&next[1]).execute(&mut *tx).await?;
            }
        }
        let hook: Option<Option<Vec<u8>>> = sqlx::query_scalar(
            "SELECT outbound_webhook_secret_enc FROM tenant_settings WHERE tenant_id = $1",
        )
        .bind(&t)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(Some(blob)) = hook {
            if !crypto.needs_rekey(&blob) {
                current += 1;
            } else if let Some(n) = crypto.rekey(&blob) {
                done += 1;
                if !dry_run {
                    sqlx::query("UPDATE tenant_settings SET outbound_webhook_secret_enc = $2 WHERE tenant_id = $1").bind(&t).bind(n).execute(&mut *tx).await?;
                }
            } else {
                bad += 1;
            }
        }
        if !dry_run {
            tenant::audit(
                &mut tx,
                &t,
                "system:rekey",
                "master_key.rekeyed",
                None,
                json!({ "resealed": done }),
            )
            .await?;
        }
        tx.commit().await?;
    }
    Ok((done, current, bad))
}
