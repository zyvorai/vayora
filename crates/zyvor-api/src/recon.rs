//! Reconciliation: orders → payments → refunds → provider settlements → bank statements.
//! Every difference that can't be explained becomes a case; every settlement can explain itself.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::{Currency, new_id};
use zyvor_providers::{Method, SettlementItem, SettlementItemKind, SettlementRecord};

use crate::{
    AppState, auth::Auth, cases::open_case, error::ApiError, payments::ListQuery, registry, tenant,
};

fn kind_str(k: &SettlementItemKind) -> &'static str {
    match k {
        SettlementItemKind::Payment => "payment",
        SettlementItemKind::Refund => "refund",
        SettlementItemKind::Fee => "fee",
        SettlementItemKind::Adjustment => "adjustment",
    }
}

fn money(minor: i64) -> String {
    format!("₹{}", zyvor_core::minor_to_decimal(minor, Currency::Inr))
}

/// Store a provider settlement, link its lines to our records, then evaluate it. Idempotent per (provider, ref).
pub async fn ingest(
    conn: &mut PgConnection,
    tenant_id: &str,
    provider: &str,
    rec: &SettlementRecord,
) -> Result<String, ApiError> {
    let sum = |k: SettlementItemKind| {
        rec.items
            .iter()
            .filter(|i| i.kind == k)
            .map(|i| i.amount_minor)
            .sum::<i64>()
    };
    let (gross, refunds, fees, adj) = (
        sum(SettlementItemKind::Payment),
        -sum(SettlementItemKind::Refund),
        -sum(SettlementItemKind::Fee),
        sum(SettlementItemKind::Adjustment),
    );
    let id = new_id("stl");
    let inserted: Option<String> = sqlx::query_scalar(
        "INSERT INTO settlements (id, tenant_id, provider, provider_ref, utr, currency, gross_minor, refunds_minor, fee_minor, adjustment_minor, net_minor, settled_on)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) ON CONFLICT (tenant_id, provider, provider_ref) DO NOTHING RETURNING id",
    )
    .bind(&id)
    .bind(tenant_id)
    .bind(provider)
    .bind(&rec.provider_ref)
    .bind(&rec.utr)
    .bind(rec.currency.code())
    .bind(gross)
    .bind(refunds)
    .bind(fees)
    .bind(adj)
    .bind(rec.net_minor)
    .bind(rec.settled_on)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(id) = inserted else {
        return sqlx::query_scalar(
            "SELECT id FROM settlements WHERE provider = $1 AND provider_ref = $2",
        )
        .bind(provider)
        .bind(&rec.provider_ref)
        .fetch_one(conn)
        .await
        .map_err(Into::into);
    };

    for it in &rec.items {
        let r = it.provider_ref.as_deref().unwrap_or("");
        let attempt: Option<String> = if matches!(
            it.kind,
            SettlementItemKind::Payment | SettlementItemKind::Fee | SettlementItemKind::Refund
        ) {
            sqlx::query_scalar("SELECT id FROM payment_attempts WHERE provider = $1 AND (provider_payment_ref = $2 OR id = $2 OR provider_order_ref = $2) LIMIT 1").bind(provider).bind(r).fetch_optional(&mut *conn).await?
        } else {
            None
        };
        let refund: Option<String> = if it.kind == SettlementItemKind::Refund {
            let direct: Option<String> = sqlx::query_scalar(
                "SELECT id FROM refunds WHERE id = $1 OR provider_refund_ref = $1 LIMIT 1",
            )
            .bind(r)
            .fetch_optional(&mut *conn)
            .await?;
            match (direct, &attempt) {
                (Some(d), _) => Some(d),
                // providers that report the order, not the refund: match on payment + amount, once
                (None, Some(a)) => {
                    sqlx::query_scalar(
                        "SELECT id FROM refunds WHERE attempt_id = $1 AND amount_minor = $2 AND status = 'processed'
                           AND id NOT IN (SELECT refund_id FROM settlement_items WHERE refund_id IS NOT NULL) LIMIT 1",
                    )
                    .bind(a)
                    .bind(-it.amount_minor)
                    .fetch_optional(&mut *conn)
                    .await?
                }
                _ => None,
            }
        } else {
            None
        };
        // a refund line that names our refund (not the payment) still belongs to that refund's payment
        let attempt = match (&attempt, &refund) {
            (None, Some(rf)) => {
                sqlx::query_scalar("SELECT attempt_id FROM refunds WHERE id = $1")
                    .bind(rf)
                    .fetch_optional(&mut *conn)
                    .await?
            }
            _ => attempt,
        };
        sqlx::query("INSERT INTO settlement_items (tenant_id, settlement_id, kind, provider_ref, attempt_id, refund_id, amount_minor, description) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
            .bind(tenant_id)
            .bind(&id)
            .bind(kind_str(&it.kind))
            .bind(&it.provider_ref)
            .bind(&attempt)
            .bind(&refund)
            .bind(it.amount_minor)
            .bind(&it.description)
            .execute(&mut *conn)
            .await?;
    }
    evaluate(conn, tenant_id, &id).await?;
    Ok(id)
}

/// Check one settlement against our records and the bank. Opens cases for anything unexplained.
pub async fn evaluate(
    conn: &mut PgConnection,
    tenant_id: &str,
    settlement_id: &str,
) -> Result<(), ApiError> {
    let (provider, net, utr, settled_on, ccy): (String, i64, Option<String>, NaiveDate, String) =
        sqlx::query_as(
            "SELECT provider, net_minor, utr, settled_on, currency FROM settlements WHERE id = $1",
        )
        .bind(settlement_id)
        .fetch_one(&mut *conn)
        .await?;
    let detail = json!({ "settlement_id": settlement_id });
    let routing = registry::routing(conn, tenant_id).await?;

    // 1. arithmetic: the parts must add up to what the provider says it paid
    let (g, r, f, a): (i64, i64, i64, i64) = sqlx::query_as("SELECT gross_minor, refunds_minor, fee_minor, adjustment_minor FROM settlements WHERE id = $1").bind(settlement_id).fetch_one(&mut *conn).await?;
    if g - r - f + a != net {
        open_case(conn, tenant_id, "settlement_arithmetic", "settlement", settlement_id, Some(g - r - f + a), Some(net), "The provider's net amount does not equal payments minus refunds, fees and adjustments.", detail.clone()).await?;
    }

    // 2. each line against our own records
    let items: Vec<(String, Option<String>, Option<String>, i64, i64)> = sqlx::query_as(
        "SELECT kind, provider_ref, attempt_id, amount_minor, (SELECT amount_minor FROM payment_attempts a WHERE a.id = settlement_items.attempt_id) FROM settlement_items WHERE settlement_id = $1 AND kind IN ('payment', 'refund')",
    )
    .bind(settlement_id)
    .fetch_all(&mut *conn)
    .await
    .map(|rows: Vec<(String, Option<String>, Option<String>, i64, Option<i64>)>| rows.into_iter().map(|(k, p, a, m, am)| (k, p, a, m, am.unwrap_or(-1))).collect())?;
    let mut seen = std::collections::BTreeSet::new();
    for (kind, pref, attempt, amount, attempt_amount) in &items {
        let subject = format!("{settlement_id}:{}", pref.as_deref().unwrap_or("?"));
        if !seen.insert((kind.clone(), pref.clone())) {
            open_case(
                conn,
                tenant_id,
                "duplicate_settlement_item",
                "settlement",
                &subject,
                None,
                Some(*amount),
                &format!(
                    "{} {} appears twice in this settlement.",
                    kind,
                    pref.as_deref().unwrap_or("?")
                ),
                detail.clone(),
            )
            .await?;
        }
        match (kind.as_str(), attempt) {
            (_, None) => {
                open_case(
                    conn,
                    tenant_id,
                    "unknown_settlement_item",
                    "settlement",
                    &subject,
                    None,
                    Some(*amount),
                    &format!(
                        "Settlement includes a {kind} ({}) that matches none of our records.",
                        pref.as_deref().unwrap_or("?")
                    ),
                    detail.clone(),
                )
                .await?;
            }
            ("payment", Some(att)) => {
                if amount != attempt_amount {
                    let note = if amount < attempt_amount {
                        "a partial payment was settled"
                    } else {
                        "more was settled than we recorded"
                    };
                    open_case(
                        conn,
                        tenant_id,
                        "settled_amount_mismatch",
                        "payment_attempt",
                        att,
                        Some(*attempt_amount),
                        Some(*amount),
                        &format!(
                            "We recorded {} but the provider settled {}: {note}.",
                            money(*attempt_amount),
                            money(*amount)
                        ),
                        detail.clone(),
                    )
                    .await?;
                }
                let st: String =
                    sqlx::query_scalar("SELECT status FROM payment_attempts WHERE id = $1")
                        .bind(att)
                        .fetch_one(&mut *conn)
                        .await?;
                if st != "captured" {
                    open_case(
                        conn,
                        tenant_id,
                        "settled_but_not_captured",
                        "payment_attempt",
                        att,
                        None,
                        Some(*amount),
                        &format!("The provider settled this payment but we have it as {st}."),
                        detail.clone(),
                    )
                    .await?;
                }
            }
            _ => {}
        }
    }

    // 3. fees against the configured schedule
    let rows: Vec<(String, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT a.provider, a.method, a.amount_minor, (SELECT -SUM(f.amount_minor)::BIGINT FROM settlement_items f WHERE f.settlement_id = $1 AND f.kind = 'fee' AND f.attempt_id = a.id)
         FROM settlement_items p JOIN payment_attempts a ON a.id = p.attempt_id WHERE p.settlement_id = $1 AND p.kind = 'payment'",
    )
    .bind(settlement_id)
    .fetch_all(&mut *conn)
    .await?;
    let configured: Vec<i64> = rows
        .iter()
        .filter_map(|(prov, method, amt, _)| {
            routing
                .fees
                .get(prov)?
                .get(&method.parse::<Method>().ok()?)
                .map(|fee| fee.estimate(*amt))
        })
        .collect();
    if !configured.is_empty() {
        let est: i64 = configured.iter().sum();
        // configured fees are pre-tax; allow for GST on the fee plus rounding
        let tolerance = est * 20 / 100 + 100 * configured.len() as i64;
        if (f - est).abs() > tolerance {
            open_case(conn, tenant_id, "fee_variance", "settlement", settlement_id, Some(est), Some(f), &format!("The provider deducted {} in fees against about {} expected from your configured rates.", money(f), money(est)), detail.clone()).await?;
        }
    }

    // 4. bank statement
    let bank: Option<(i64, i64)> = match &utr {
        Some(u) => sqlx::query_as("SELECT id, amount_minor FROM bank_statement_lines WHERE utr = $1 AND matched_settlement_id IS NULL").bind(u).fetch_optional(&mut *conn).await?,
        None => None,
    };
    let bank = match bank {
        Some(b) => Some(b),
        None => {
            sqlx::query_as(
                "SELECT id, amount_minor FROM bank_statement_lines WHERE matched_settlement_id IS NULL AND amount_minor = $1 AND currency = $2 AND posted_on BETWEEN $3 - 3 AND $3 + 3 ORDER BY posted_on LIMIT 1",
            )
            .bind(net)
            .bind(&ccy)
            .bind(settled_on)
            .fetch_optional(&mut *conn)
            .await?
        }
    };
    if let Some((line, bank_amount)) = bank {
        sqlx::query("UPDATE bank_statement_lines SET matched_settlement_id = $2 WHERE id = $1")
            .bind(line)
            .bind(settlement_id)
            .execute(&mut *conn)
            .await?;
        sqlx::query("UPDATE settlements SET bank_line_id = $2 WHERE id = $1")
            .bind(settlement_id)
            .bind(line)
            .execute(&mut *conn)
            .await?;
        if bank_amount != net {
            open_case(
                conn,
                tenant_id,
                "bank_amount_mismatch",
                "settlement",
                settlement_id,
                Some(net),
                Some(bank_amount),
                &format!(
                    "{provider} says it paid {} but the bank statement shows {} for the same UTR.",
                    money(net),
                    money(bank_amount)
                ),
                detail.clone(),
            )
            .await?;
        }
    }

    let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recon_cases WHERE status <> 'resolved' AND detail->>'settlement_id' = $1").bind(settlement_id).fetch_one(&mut *conn).await?;
    let matched: bool =
        sqlx::query_scalar("SELECT bank_line_id IS NOT NULL FROM settlements WHERE id = $1")
            .bind(settlement_id)
            .fetch_one(&mut *conn)
            .await?;
    let status = if open > 0 {
        "exception"
    } else if matched {
        "matched"
    } else {
        "imported"
    };
    sqlx::query("UPDATE settlements SET status = $2 WHERE id = $1")
        .bind(settlement_id)
        .bind(status)
        .execute(conn)
        .await?;
    Ok(())
}

/// Captured payments that never showed up in any settlement after `older_than_days`.
pub async fn scan_missing(
    conn: &mut PgConnection,
    tenant_id: &str,
    older_than_days: i32,
) -> Result<usize, ApiError> {
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT a.id, a.amount_minor, a.provider FROM payment_attempts a
         WHERE a.status = 'captured' AND a.updated_at < now() - make_interval(days => $1)
           AND NOT EXISTS (SELECT 1 FROM settlement_items s WHERE s.attempt_id = a.id AND s.kind = 'payment')",
    )
    .bind(older_than_days)
    .fetch_all(&mut *conn)
    .await?;
    let mut n = 0;
    for (id, amount, provider) in rows {
        if open_case(conn, tenant_id, "missing_settlement", "payment_attempt", &id, Some(amount), Some(0), &format!("Captured {older_than_days}+ days ago but not in any {provider} settlement yet ({} expected).", money(amount)), json!({ "provider": provider })).await? {
            n += 1;
        }
    }
    Ok(n)
}

// ---------- endpoints ----------

#[derive(Deserialize)]
pub struct ImportItem {
    kind: SettlementItemKind,
    provider_ref: Option<String>,
    amount_minor: i64,
    description: Option<String>,
}

#[derive(Deserialize)]
pub struct ImportSettlement {
    provider: String,
    provider_ref: String,
    utr: Option<String>,
    currency: Option<String>,
    settled_on: NaiveDate,
    /// What the provider says it paid. Defaults to the sum of the items.
    net_minor: Option<i64>,
    items: Vec<ImportItem>,
}

pub async fn import_settlement(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<ImportSettlement>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let items: Vec<SettlementItem> = b
        .items
        .into_iter()
        .map(|i| SettlementItem {
            kind: i.kind,
            provider_ref: i.provider_ref,
            amount_minor: i.amount_minor,
            description: i.description.unwrap_or_default(),
        })
        .collect();
    let computed: i64 = items.iter().map(|i| i.amount_minor).sum();
    let rec = SettlementRecord {
        provider_ref: b.provider_ref,
        utr: b.utr,
        currency: b
            .currency
            .as_deref()
            .unwrap_or("INR")
            .parse()
            .map_err(|e: String| ApiError::invalid("invalid_currency", e))?,
        settled_on: b.settled_on,
        net_minor: b.net_minor.unwrap_or(computed),
        items,
    };
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let id = ingest(&mut tx, &auth.tenant_id, &b.provider, &rec).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "settlement.imported",
        Some(&id),
        json!({ "provider": b.provider }),
    )
    .await?;
    let out = explain_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct SyncRequest {
    provider: String,
    date: NaiveDate,
}

/// Pull settlements for a day from the provider itself.
pub async fn sync_settlements(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<SyncRequest>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    if b.provider == "sim" {
        return sync_simulated(&s, &auth, b.date).await;
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let provider = registry::load(&s, &mut tx, &auth.tenant_id, &auth.env, &b.provider).await?;
    tx.commit().await?;
    let records = provider.fetch_settlements(b.date).await.map_err(|e| {
        ApiError::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "provider_error",
            e.to_string(),
        )
    })?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mut ids = Vec::new();
    for r in &records {
        ids.push(ingest(&mut tx, &auth.tenant_id, &b.provider, r).await?);
    }
    tx.commit().await?;
    Ok(Json(json!({ "imported": ids })))
}

/// The simulator has no settlement file, so build one from this organisation's own simulated payments: the payments
/// captured that day (IST), less a flat 2% fee on each, less the refunds processed that day. One settlement per day,
/// stored once; payments captured after it was synced are not added to it. The fee is invented, not a real tariff.
async fn sync_simulated(
    s: &AppState,
    auth: &Auth,
    date: NaiveDate,
) -> Result<Json<Value>, ApiError> {
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let paid: Vec<(String, i64)> = sqlx::query_as(
        "SELECT a.id, a.amount_minor FROM payment_attempts a
         JOIN ledger_transactions l ON l.reference = 'capture:' || a.id
         WHERE a.provider = 'sim' AND a.status = 'captured' AND a.currency = 'INR'
           AND (l.created_at AT TIME ZONE 'Asia/Kolkata')::date = $1 ORDER BY a.id",
    )
    .bind(date)
    .fetch_all(&mut *tx)
    .await?;
    let refunded: Vec<(String, i64)> = sqlx::query_as(
        "SELECT r.id, r.amount_minor FROM refunds r JOIN payment_attempts a ON a.id = r.attempt_id
         WHERE a.provider = 'sim' AND r.status = 'processed' AND r.currency = 'INR'
           AND (r.updated_at AT TIME ZONE 'Asia/Kolkata')::date = $1 ORDER BY r.id",
    )
    .bind(date)
    .fetch_all(&mut *tx)
    .await?;
    if paid.is_empty() && refunded.is_empty() {
        tx.commit().await?;
        return Ok(Json(json!({ "imported": [] })));
    }
    let mut items = Vec::new();
    for (id, amount) in &paid {
        items.push(SettlementItem {
            kind: SettlementItemKind::Payment,
            provider_ref: Some(id.clone()),
            amount_minor: *amount,
            description: "simulated payment".into(),
        });
        items.push(SettlementItem {
            kind: SettlementItemKind::Fee,
            provider_ref: Some(id.clone()),
            amount_minor: -(amount * 2 / 100),
            description: "simulated fee (2%)".into(),
        });
    }
    for (id, amount) in &refunded {
        items.push(SettlementItem {
            kind: SettlementItemKind::Refund,
            provider_ref: Some(id.clone()),
            amount_minor: -amount,
            description: "simulated refund".into(),
        });
    }
    let day = date.format("%Y%m%d");
    let rec = SettlementRecord {
        provider_ref: format!("simstl_{day}"),
        utr: Some(format!("SIMUTR{day}")),
        currency: Currency::Inr,
        settled_on: date,
        net_minor: items.iter().map(|i| i.amount_minor).sum(),
        items,
    };
    let id = ingest(&mut tx, &auth.tenant_id, "sim", &rec).await?;
    tx.commit().await?;
    Ok(Json(json!({ "imported": [id] })))
}

pub async fn list_settlements(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, Option<String>, i64, i64, i64, i64, i64, NaiveDate, String)> = sqlx::query_as(
        "SELECT id, provider, provider_ref, utr, gross_minor, refunds_minor, fee_minor, adjustment_minor, net_minor, settled_on, status FROM settlements
         WHERE ($1::text IS NULL OR status = $1) ORDER BY settled_on DESC, id DESC LIMIT $2",
    )
    .bind(&q.status)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let data: Vec<Value> = rows.into_iter().map(|(id, provider, pr, utr, gross, refunds, fee, adj, net, on, status)| json!({ "id": id, "provider": provider, "provider_ref": pr, "utr": utr, "gross_minor": gross, "refunds_minor": refunds, "fee_minor": fee, "adjustment_minor": adj, "net_minor": net, "settled_on": on, "status": status })).collect();
    Ok(Json(json!({ "data": data })))
}

pub async fn explain(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(explain_json(&mut tx, &id).await?))
}

/// "Explain every rupee": payments, refunds, fees, adjustments, the bank match, and why the net is what it is.
pub async fn explain_json(conn: &mut PgConnection, id: &str) -> Result<Value, ApiError> {
    let st: (String, String, String, Option<String>, i64, i64, i64, i64, i64, NaiveDate, String, Option<i64>) = sqlx::query_as(
        "SELECT id, provider, provider_ref, utr, gross_minor, refunds_minor, fee_minor, adjustment_minor, net_minor, settled_on, status, bank_line_id FROM settlements WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| ApiError::not_found("settlement"))?;
    let (
        _,
        provider,
        provider_ref,
        utr,
        gross,
        refunds,
        fees,
        adj,
        net,
        settled_on,
        status,
        bank_line,
    ) = st;
    let tenant_id: String = sqlx::query_scalar("SELECT tenant_id FROM settlements WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;

    let payments: Vec<Value> = sqlx::query_as::<_, (Option<String>, Option<String>, i64, Option<i64>, Option<String>, Option<String>, Option<String>)>(
        "SELECT i.attempt_id, i.provider_ref, i.amount_minor, a.amount_minor, a.method, p.external_ref, p.invoice_id
         FROM settlement_items i LEFT JOIN payment_attempts a ON a.id = i.attempt_id LEFT JOIN payment_intents p ON p.id = a.intent_id
         WHERE i.settlement_id = $1 AND i.kind = 'payment' ORDER BY i.id",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|(att, pref, amt, ours, method, ext, inv)| json!({ "attempt_id": att, "provider_ref": pref, "settled_minor": amt, "recorded_minor": ours, "method": method, "external_ref": ext, "invoice_id": inv, "matched": att.is_some() }))
    .collect();
    let refund_lines: Vec<Value> = sqlx::query_as::<_, (Option<String>, Option<String>, i64, Option<String>)>(
        "SELECT i.refund_id, i.provider_ref, i.amount_minor, r.external_ref FROM settlement_items i LEFT JOIN refunds r ON r.id = i.refund_id WHERE i.settlement_id = $1 AND i.kind = 'refund'",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|(rid, pref, amt, ext)| json!({ "refund_id": rid, "provider_ref": pref, "amount_minor": amt, "external_ref": ext, "matched": rid.is_some() }))
    .collect();
    let adjustments: Vec<Value> = sqlx::query_as::<_, (i64, String)>("SELECT amount_minor, description FROM settlement_items WHERE settlement_id = $1 AND kind = 'adjustment'")
        .bind(id)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|(a, d)| json!({ "amount_minor": a, "description": d }))
        .collect();
    let cases: Vec<Value> = sqlx::query_as::<_, (String, String, String, String, Option<String>)>("SELECT id, kind, status, explanation, assignee FROM recon_cases WHERE detail->>'settlement_id' = $1 ORDER BY created_at")
        .bind(id)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|(cid, kind, st, ex, asg)| json!({ "id": cid, "kind": kind, "status": st, "explanation": ex, "assignee": asg }))
        .collect();
    let bank = match bank_line {
        Some(b) => sqlx::query_as::<_, (NaiveDate, i64, String, Option<String>)>("SELECT posted_on, amount_minor, narration, utr FROM bank_statement_lines WHERE id = $1").bind(b).fetch_optional(&mut *conn).await?.map(|(on, a, n, u)| json!({ "posted_on": on, "amount_minor": a, "narration": n, "utr": u })),
        None => None,
    };

    // What we expected to receive: recorded payment value minus refunds minus configured fee estimate.
    let recorded: i64 = payments
        .iter()
        .filter_map(|p| p["recorded_minor"].as_i64())
        .sum();
    let routing = registry::routing(conn, &tenant_id).await?;
    let est_fee: i64 = sqlx::query_as::<_, (String, String, i64)>("SELECT a.provider, a.method, a.amount_minor FROM settlement_items i JOIN payment_attempts a ON a.id = i.attempt_id WHERE i.settlement_id = $1 AND i.kind = 'payment'")
        .bind(id)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .filter_map(|(prov, m, amt)| routing.fees.get(&prov)?.get(&m.parse::<Method>().ok()?).map(|f| f.estimate(amt)))
        .sum();
    let expected = recorded - refunds - est_fee;
    let mut breakdown = Vec::new();
    if fees != est_fee {
        breakdown.push(json!({ "reason": if est_fee == 0 { "provider fees and taxes (no fee schedule configured)" } else { "provider fees and taxes above the configured estimate" }, "amount_minor": -(fees - est_fee) }));
    }
    if adj != 0 {
        breakdown.push(json!({ "reason": "settlement adjustments", "amount_minor": adj }));
    }
    let unmatched = payments.iter().filter(|p| p["matched"] == false).count();
    if unmatched > 0 {
        breakdown.push(json!({ "reason": format!("{unmatched} payment(s) in this settlement are not in our records"), "amount_minor": 0 }));
    }
    let effective_bps = if gross > 0 { fees * 10_000 / gross } else { 0 };
    let mut narrative = format!(
        "{provider} paid {} on {settled_on}: {} payments worth {}{}. It deducted {} in fees and taxes ({}.{:02}% of payment value){}.",
        money(net),
        payments.len(),
        money(gross),
        if refunds > 0 {
            format!(", less {} of refunds", money(refunds))
        } else {
            String::new()
        },
        money(fees),
        effective_bps / 100,
        effective_bps % 100,
        if est_fee > 0 {
            format!(
                ", against about {} expected from your configured rates",
                money(est_fee)
            )
        } else {
            String::new()
        },
    );
    if adj != 0 {
        narrative += &format!(" Adjustments account for {}.", money(adj));
    }
    if expected != net {
        narrative += &format!(
            " Compared with what we expected ({}), it is {} {}.",
            money(expected),
            money((net - expected).abs()),
            if net < expected { "short" } else { "over" }
        );
    }
    narrative += &match &bank {
        Some(b) => format!(
            " The bank statement shows {} credited on {}.",
            money(b["amount_minor"].as_i64().unwrap_or(0)),
            b["posted_on"].as_str().unwrap_or("?")
        ),
        None => " No matching bank credit has been found yet.".to_string(),
    };
    if !cases.is_empty() {
        narrative += &format!(
            " {} open reconciliation case(s) need attention.",
            cases.iter().filter(|c| c["status"] != "resolved").count()
        );
    }

    Ok(json!({
        "settlement": { "id": id, "provider": provider, "provider_ref": provider_ref, "utr": utr, "settled_on": settled_on, "status": status, "net_minor": net },
        "payments": payments, "refunds": refund_lines, "adjustments": adjustments,
        "fees": { "observed_minor": fees, "configured_estimate_minor": est_fee, "effective_rate_bps": effective_bps },
        "arithmetic": { "gross_minor": gross, "refunds_minor": refunds, "fees_minor": fees, "adjustments_minor": adj, "net_minor": net, "balances": gross - refunds - fees + adj == net },
        "expected_minor": expected, "received_minor": net, "difference_minor": net - expected, "difference_breakdown": breakdown,
        "bank_match": bank, "cases": cases, "narrative": narrative,
    }))
}

// ---------- bank statements ----------

#[derive(Deserialize)]
pub struct BankLine {
    posted_on: NaiveDate,
    pub(crate) amount_minor: i64,
    currency: Option<String>,
    pub(crate) narration: Option<String>,
    pub(crate) utr: Option<String>,
}

impl BankLine {
    pub fn new(posted_on: NaiveDate, amount_minor: i64, narration: String, utr: String) -> Self {
        Self {
            posted_on,
            amount_minor,
            currency: None,
            narration: Some(narration),
            utr: Some(utr),
        }
    }
}

#[derive(Deserialize)]
pub struct ImportBank {
    pub(crate) lines: Vec<BankLine>,
}

#[derive(Deserialize, Default)]
pub struct SimulateBank {
    /// Pay this much less than the settlement's net, to see a difference become a case.
    #[serde(default)]
    short_minor: i64,
}

/// `POST /v1/settlements/{id}/simulate_bank`: test environments only. Adds the bank credit a simulated settlement
/// would produce (its net, less `short_minor`, on its settlement day, carrying its UTR), as if the bank statement had
/// been imported, then matches it the normal way.
pub async fn simulate_bank(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Option<Json<SimulateBank>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    if auth.env != "test" {
        return Err(ApiError::invalid(
            "test_only",
            "simulated bank credits are only available in test mode",
        ));
    }
    let short = body.map(|b| b.0.short_minor).unwrap_or(0);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, Option<String>, i64, NaiveDate)> = sqlx::query_as(
        "SELECT provider, utr, net_minor, settled_on FROM settlements WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    let (provider, utr, net, on) = row.ok_or_else(|| ApiError::not_found("settlement"))?;
    let utr = utr.filter(|_| provider == "sim").ok_or_else(|| {
        ApiError::invalid(
            "not_simulated",
            "only a simulated settlement can have a simulated bank credit",
        )
    })?;
    if net <= 0 {
        return Err(ApiError::invalid(
            "nothing_to_credit",
            "this settlement nets to nothing or less, so the bank would not credit anything",
        ));
    }
    if short < 0 || short >= net {
        return Err(ApiError::invalid(
            "invalid_short",
            "short_minor must be at least 0 and less than the settlement's net",
        ));
    }
    let line = BankLine::new(on, net - short, "SIMULATED settlement credit".into(), utr);
    import_bank(State(s), auth, Json(ImportBank { lines: vec![line] })).await
}

pub async fn import_bank(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<ImportBank>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mut added = 0;
    for l in &b.lines {
        added += sqlx::query("INSERT INTO bank_statement_lines (tenant_id, posted_on, amount_minor, currency, narration, utr) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (tenant_id, utr) DO NOTHING")
            .bind(&auth.tenant_id)
            .bind(l.posted_on)
            .bind(l.amount_minor)
            .bind(l.currency.as_deref().unwrap_or("INR"))
            .bind(l.narration.as_deref().unwrap_or(""))
            .bind(&l.utr)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    // Re-evaluate settlements still waiting for a bank credit, then flag credits nobody explains.
    let waiting: Vec<String> =
        sqlx::query_scalar("SELECT id FROM settlements WHERE bank_line_id IS NULL")
            .fetch_all(&mut *tx)
            .await?;
    for id in &waiting {
        evaluate(&mut tx, &auth.tenant_id, id).await?;
    }
    let orphans: Vec<(i64, i64, NaiveDate, String)> = sqlx::query_as("SELECT id, amount_minor, posted_on, narration FROM bank_statement_lines WHERE matched_settlement_id IS NULL AND amount_minor > 0").fetch_all(&mut *tx).await?;
    for (id, amount, on, narr) in orphans {
        open_case(
            &mut tx,
            &auth.tenant_id,
            "unmatched_bank_credit",
            "bank_line",
            &id.to_string(),
            None,
            Some(amount),
            &format!(
                "{} credited on {on} ({narr}) matches no provider settlement.",
                money(amount)
            ),
            json!({}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({ "added": added, "settlements_rechecked": waiting.len() }),
    ))
}

// ---------- cases ----------

#[derive(Deserialize)]
pub struct CaseQuery {
    status: Option<String>,
    kind: Option<String>,
    limit: Option<i64>,
}

pub async fn list_cases(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<CaseQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, String, String, Option<i64>, Option<i64>, Option<i64>, String, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, kind, status, subject_type, subject_id, expected_minor, actual_minor, diff_minor, explanation, assignee, created_at FROM recon_cases
         WHERE ($1::text IS NULL OR status = $1) AND ($2::text IS NULL OR kind = $2) ORDER BY created_at DESC LIMIT $3",
    )
    .bind(&q.status)
    .bind(&q.kind)
    .bind(q.limit.unwrap_or(50).clamp(1, 200))
    .fetch_all(&mut *tx)
    .await?;
    let data: Vec<Value> = rows.into_iter().map(|(id, kind, status, st, sid, e, a, d, ex, asg, at)| json!({ "id": id, "kind": kind, "status": status, "subject_type": st, "subject_id": sid, "expected_minor": e, "actual_minor": a, "diff_minor": d, "explanation": ex, "assignee": asg, "created_at": at })).collect();
    Ok(Json(json!({ "data": data })))
}

#[derive(Deserialize)]
pub struct Assign {
    assignee: String,
}

pub async fn assign_case(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<Assign>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("UPDATE recon_cases SET assignee = $2, status = 'assigned' WHERE id = $1 AND status <> 'resolved'").bind(&id).bind(&b.assignee).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("open case"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "recon_case.assigned",
        Some(&id),
        json!({ "assignee": b.assignee }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "id": id, "status": "assigned", "assignee": b.assignee }),
    ))
}

#[derive(Deserialize)]
pub struct Resolve {
    resolution: String,
}

pub async fn resolve_case(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<Resolve>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let sid: Option<Option<String>> = sqlx::query_scalar("UPDATE recon_cases SET status = 'resolved', resolution = $2, resolved_at = now() WHERE id = $1 AND status <> 'resolved' RETURNING detail->>'settlement_id'").bind(&id).bind(&b.resolution).fetch_optional(&mut *tx).await?;
    let sid = sid.ok_or_else(|| ApiError::not_found("open case"))?;
    if let Some(sid) = sid {
        // The settlement leaves `exception` once its last case is resolved.
        sqlx::query(
            "UPDATE settlements SET status = CASE WHEN bank_line_id IS NOT NULL THEN 'matched' ELSE 'imported' END
             WHERE id = $1 AND status = 'exception' AND NOT EXISTS (SELECT 1 FROM recon_cases WHERE detail->>'settlement_id' = $1 AND status <> 'resolved')",
        )
        .bind(sid)
        .execute(&mut *tx)
        .await?;
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "recon_case.resolved",
        Some(&id),
        json!({ "resolution": b.resolution }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "resolved" })))
}

pub async fn scan(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<std::collections::BTreeMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let days = q
        .get("older_than_days")
        .and_then(|d| d.parse().ok())
        .unwrap_or(3);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = scan_missing(&mut tx, &auth.tenant_id, days).await?;
    tx.commit().await?;
    Ok(Json(json!({ "new_cases": n })))
}

// ---------- measured cost ----------

/// Effective processing cost by provider and method, from configured fees and observed deductions,
/// with routing suggestions backed by the evidence.
pub async fn cost_analytics(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let routing = registry::routing(&mut tx, &auth.tenant_id).await?;
    let rows: Vec<(String, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT a.provider, a.method, a.amount_minor,
                (SELECT -SUM(f.amount_minor)::BIGINT FROM settlement_items f WHERE f.attempt_id = a.id AND f.kind = 'fee')
         FROM payment_attempts a WHERE a.status = 'captured'",
    )
    .fetch_all(&mut *tx)
    .await?;
    #[derive(Default)]
    struct Agg {
        payments: i64,
        volume: i64,
        observed_fee: i64,
        observed_volume: i64,
        configured_fee: i64,
    }
    let mut by: std::collections::BTreeMap<(String, String), Agg> = Default::default();
    for (prov, method, amount, fee) in rows {
        let a = by.entry((prov.clone(), method.clone())).or_default();
        a.payments += 1;
        a.volume += amount;
        if let Some(f) = fee {
            a.observed_fee += f;
            a.observed_volume += amount;
        }
        if let Some(fee) = method
            .parse::<Method>()
            .ok()
            .and_then(|m| routing.fees.get(&prov)?.get(&m).copied())
        {
            a.configured_fee += fee.estimate(amount);
        }
    }
    let rate = |fee: i64, vol: i64| {
        if vol > 0 {
            Some(fee * 10_000 / vol)
        } else {
            None
        }
    };
    let table: Vec<Value> = by
        .iter()
        .map(|((p, m), a)| {
            json!({
                "provider": p, "method": m, "payments": a.payments, "volume_minor": a.volume,
                "observed_fee_minor": a.observed_fee, "observed_rate_bps": rate(a.observed_fee, a.observed_volume), "observed_on_minor": a.observed_volume,
                "configured_fee_minor": a.configured_fee, "configured_rate_bps": rate(a.configured_fee, a.volume),
            })
        })
        .collect();

    // Suggest moving a method's volume to a cheaper provider that supports it.
    let candidates =
        registry::candidates(&s, &mut tx, &auth.tenant_id, &auth.env, &routing).await?;
    let eff = |p: &str, m: &str| -> Option<(i64, &'static str)> {
        let a = by.get(&(p.to_string(), m.to_string()))?;
        rate(a.observed_fee, a.observed_volume)
            .map(|r| (r, "observed"))
            .or_else(|| rate(a.configured_fee, a.volume).map(|r| (r, "configured")))
    };
    let mut suggestions = Vec::new();
    for ((from, method), a) in &by {
        let Some((from_bps, from_src)) = eff(from, method) else {
            continue;
        };
        let Ok(m) = method.parse::<Method>() else {
            continue;
        };
        for c in candidates
            .iter()
            .filter(|c| &c.provider != from && c.enabled && c.caps.methods.contains(&m))
        {
            let to_bps = eff(&c.provider, method)
                .or_else(|| c.fees.get(&m).map(|f| (f.bps as i64, "configured")));
            let Some((to_bps, to_src)) = to_bps else {
                continue;
            };
            if to_bps * 10 <= from_bps * 9 {
                suggestions.push(json!({
                    "method": method, "move_from": from, "move_to": c.provider,
                    "estimated_saving_minor": a.volume * (from_bps - to_bps) / 10_000,
                    "evidence": { "from_rate_bps": from_bps, "from_basis": from_src, "to_rate_bps": to_bps, "to_basis": to_src, "payments_considered": a.payments, "volume_minor": a.volume },
                    "action": format!("Add a routing rule preferring {} for {method}. Review before applying: this is a suggestion, not a change.", c.provider),
                }));
            }
        }
    }
    Ok(Json(
        json!({ "by_provider_method": table, "suggestions": suggestions }),
    ))
}
