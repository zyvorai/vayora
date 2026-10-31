//! A local stand-in for the Razorpay and Cashfree HTTP APIs plus control endpoints under `/_sim/*`.
//! Everything here is SIMULATED: shapes follow the providers' public docs, but behaviour is ours.
//! Control endpoints return signed webhook deliveries instead of pushing them, so tests decide
//! when (and how many times, and in what order) the platform receives them.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

#[derive(Clone)]
pub struct MockConfig {
    pub razorpay_webhook_secret: String,
    pub cashfree_webhook_secret: String,
}

#[derive(Default)]
struct Inner {
    seq: u64,
    // razorpay
    rz_orders: BTreeMap<String, Value>,
    rz_payments: BTreeMap<String, Value>,
    rz_refunds: BTreeMap<String, Value>,
    // cashfree
    cf_orders: BTreeMap<String, Value>,
    cf_payments: BTreeMap<String, Value>, // by cf_payment_id
    cf_refunds: BTreeMap<String, Value>,  // by our refund_id
    // settlements: provider -> list of recon rows
    settlements: BTreeMap<String, Vec<Value>>,
    settled: std::collections::BTreeSet<String>,
    /// One-shot misbehaviour per operation, e.g. "cashfree.pay" -> "hang_after_accept".
    behaviours: BTreeMap<String, String>,
    charges: BTreeMap<String, i64>,
}

#[derive(Clone)]
struct Mock {
    cfg: MockConfig,
    inner: Arc<Mutex<Inner>>,
}

impl Mock {
    fn next(&self, prefix: &str) -> String {
        let mut g = self.inner.lock().unwrap();
        g.seq += 1;
        format!("{prefix}{:014}", g.seq)
    }
    fn take_behaviour(&self, op: &str) -> Option<String> {
        self.inner.lock().unwrap().behaviours.remove(op)
    }
}

type Resp = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn err(code: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (
        code,
        Json(json!({ "error": { "description": msg }, "message": msg })),
    )
}
async fn hang() {
    tokio::time::sleep(Duration::from_secs(30)).await;
}

pub fn router(cfg: MockConfig) -> Router {
    let m = Mock {
        cfg,
        inner: Default::default(),
    };
    Router::new()
        // ---- razorpay ----
        .route("/v1/payments", get(rz_list_payments))
        .route("/v1/orders", post(rz_create_order).get(rz_find_orders))
        .route("/v1/orders/{id}/payments", get(rz_order_payments))
        .route("/v1/payments/{id}/capture", post(rz_capture))
        .route("/v1/payments/{id}/refund", post(rz_refund))
        .route("/v1/payments/{id}/refunds", get(rz_payment_refunds))
        .route("/v1/settlements/recon/combined", get(settlement_recon_rz))
        // ---- cashfree ----
        .route("/pg/orders/{id}", get(cf_get_order))
        .route("/pg/orders", post(cf_create_order))
        .route("/pg/orders/sessions", post(cf_pay))
        .route("/pg/orders/{id}/payments", get(cf_order_payments))
        .route("/pg/orders/{id}/authorization", post(cf_capture))
        .route("/pg/orders/{id}/refunds", post(cf_refund))
        .route("/pg/orders/{id}/refunds/{rid}", get(cf_get_refund))
        .route("/pg/settlement/recon", post(settlement_recon_cf))
        // ---- control ----
        .route("/_sim/behaviour", post(sim_behaviour))
        .route("/_sim/razorpay/pay", post(sim_rz_pay))
        .route("/_sim/razorpay/refund_event", post(sim_rz_refund_event))
        .route("/_sim/cashfree/pay", post(sim_cf_pay))
        .route("/_sim/{provider}/settle", post(sim_settle))
        .route("/_sim/stats", get(sim_stats))
        .with_state(m)
}

/// Bind on an ephemeral port and serve in the background. Returns the base URL.
pub async fn spawn(cfg: MockConfig) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router(cfg)).await.ok() });
    url
}

// =============== Razorpay ===============

async fn rz_create_order(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let amount = b["amount"]
        .as_i64()
        .filter(|a| *a >= 100)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "amount must be at least 100 paise"))?;
    let id = m.next("order_");
    let o = json!({ "id": id, "entity": "order", "amount": amount, "currency": b["currency"], "receipt": b["receipt"], "status": "created", "notes": b["notes"] });
    m.inner.lock().unwrap().rz_orders.insert(id, o.clone());
    Ok(Json(o))
}

/// A key id ending in `bad` is refused, so tests can see a rejected connection check.
async fn rz_list_payments(headers: axum::http::HeaderMap) -> Resp {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let decoded = auth.strip_prefix("Basic ").and_then(|b| {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(b).ok()
    });
    let user = decoded
        .map(|d| {
            String::from_utf8_lossy(&d)
                .split(':')
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .unwrap_or_default();
    if user.ends_with("bad") {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": { "description": "Authentication failed" } })),
        ));
    }
    Ok(Json(
        json!({ "entity": "collection", "count": 0, "items": [] }),
    ))
}

/// Cashfree order lookup: no order exists (404), unless the client id is `BADID` (401).
async fn cf_get_order(headers: axum::http::HeaderMap, Path(_id): Path<String>) -> Resp {
    if headers.get("x-client-id").and_then(|v| v.to_str().ok()) == Some("BADID") {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "authentication Failed", "type": "authentication_error" })),
        ));
    }
    Err((
        StatusCode::NOT_FOUND,
        Json(json!({ "message": "order not found", "type": "order_not_found" })),
    ))
}

async fn rz_find_orders(State(m): State<Mock>, Query(q): Query<BTreeMap<String, String>>) -> Resp {
    let g = m.inner.lock().unwrap();
    let items: Vec<Value> = g
        .rz_orders
        .values()
        .filter(|o| q.get("receipt").is_none_or(|r| o["receipt"] == r.as_str()))
        .cloned()
        .collect();
    Ok(Json(
        json!({ "entity": "collection", "count": items.len(), "items": items }),
    ))
}

async fn rz_order_payments(State(m): State<Mock>, Path(id): Path<String>) -> Resp {
    let g = m.inner.lock().unwrap();
    let items: Vec<Value> = g
        .rz_payments
        .values()
        .filter(|p| p["order_id"] == id.as_str())
        .cloned()
        .collect();
    Ok(Json(
        json!({ "entity": "collection", "count": items.len(), "items": items }),
    ))
}

async fn rz_capture(State(m): State<Mock>, Path(id): Path<String>, Json(b): Json<Value>) -> Resp {
    let mut g = m.inner.lock().unwrap();
    let p = g
        .rz_payments
        .get_mut(&id)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "payment not found"))?;
    if p["status"] != "authorized" {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "payment is not in authorized state",
        ));
    }
    if b["amount"] != p["amount"] {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "capture amount must equal authorized amount",
        ));
    }
    p["status"] = json!("captured");
    p["captured"] = json!(true);
    let out = p.clone();
    *g.charges.entry("razorpay".into()).or_default() += 1;
    Ok(Json(out))
}

async fn rz_refund(State(m): State<Mock>, Path(id): Path<String>, Json(b): Json<Value>) -> Resp {
    let amount = b["amount"]
        .as_i64()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "amount required"))?;
    let behaviour = m.take_behaviour("razorpay.refund");
    if behaviour.as_deref() == Some("reject") {
        return Err(err(StatusCode::BAD_REQUEST, "refund rejected by bank"));
    }
    let rid = m.next("rfnd_");
    let r = {
        let mut g = m.inner.lock().unwrap();
        let p = g
            .rz_payments
            .get(&id)
            .cloned()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "payment not found"))?;
        if p["status"] != "captured" {
            return Err(err(StatusCode::BAD_REQUEST, "payment not captured"));
        }
        let done: i64 = g
            .rz_refunds
            .values()
            .filter(|r| r["payment_id"] == id.as_str())
            .filter_map(|r| r["amount"].as_i64())
            .sum();
        if done + amount > p["amount"].as_i64().unwrap_or(0) {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "refund amount exceeds the captured amount",
            ));
        }
        let r = json!({ "id": rid, "entity": "refund", "payment_id": id, "amount": amount, "status": "processed", "receipt": b["receipt"], "notes": b["notes"] });
        g.rz_refunds.insert(rid, r.clone());
        r
    };
    match behaviour.as_deref() {
        Some("hang_after_accept") => {
            hang().await;
            Ok(Json(r))
        }
        Some("server_error") => Err(err(StatusCode::INTERNAL_SERVER_ERROR, "boom")),
        _ => Ok(Json(r)),
    }
}

async fn rz_payment_refunds(State(m): State<Mock>, Path(id): Path<String>) -> Resp {
    let g = m.inner.lock().unwrap();
    let items: Vec<Value> = g
        .rz_refunds
        .values()
        .filter(|r| r["payment_id"] == id.as_str())
        .cloned()
        .collect();
    Ok(Json(
        json!({ "entity": "collection", "count": items.len(), "items": items }),
    ))
}

fn rz_sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

fn rz_delivery(m: &Mock, event: &str, entity_key: &str, entity: &Value) -> Value {
    let body = json!({ "entity": "event", "event": event, "payload": { entity_key: { "entity": entity } } }).to_string();
    let event_id = m.next("evt_");
    json!({ "headers": { "x-razorpay-signature": rz_sign(&m.cfg.razorpay_webhook_secret, body.as_bytes()), "x-razorpay-event-id": event_id, "content-type": "application/json" }, "body": body })
}

/// Customer pays at Razorpay Checkout. `outcome`: captured | authorized | failed.
async fn sim_rz_pay(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let order_id = b["order_id"]
        .as_str()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "order_id"))?
        .to_string();
    let outcome = b["outcome"].as_str().unwrap_or("captured");
    let id = b["payment_id"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| m.next("pay_"));
    let (payment, event) = {
        let mut g = m.inner.lock().unwrap();
        let order = g
            .rz_orders
            .get(&order_id)
            .cloned()
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "order not found"))?;
        let p = json!({ "id": id, "entity": "payment", "order_id": order_id, "amount": order["amount"], "currency": order["currency"], "status": outcome,
            "method": b["method"].as_str().unwrap_or("upi"), "captured": outcome == "captured",
            "error_description": if outcome == "failed" { json!("Payment failed at bank") } else { Value::Null } });
        g.rz_payments.insert(id.clone(), p.clone());
        if outcome == "captured" {
            *g.charges.entry("razorpay".into()).or_default() += 1;
        }
        let ev = match outcome {
            "captured" => "payment.captured",
            "authorized" => "payment.authorized",
            _ => "payment.failed",
        };
        (p, ev)
    };
    Ok(Json(
        json!({ "payment": payment, "webhook": rz_delivery(&m, event, "payment", &payment) }),
    ))
}

async fn sim_rz_refund_event(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let rid = b["refund_id"].as_str().unwrap_or_default().to_string();
    let r = m
        .inner
        .lock()
        .unwrap()
        .rz_refunds
        .get(&rid)
        .cloned()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "refund not found"))?;
    Ok(Json(
        json!({ "refund": r, "webhook": rz_delivery(&m, "refund.processed", "refund", &r) }),
    ))
}

// =============== Cashfree ===============

fn cf_amount(v: &Value) -> i64 {
    v.as_f64().map(|f| (f * 100.0).round() as i64).unwrap_or(0)
}

async fn cf_create_order(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let order_id = b["order_id"]
        .as_str()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "order_id is required"))?
        .to_string();
    if cf_amount(&b["order_amount"]) < 100 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "order_amount must be at least 1.00",
        ));
    }
    let mut g = m.inner.lock().unwrap();
    if g.cf_orders.contains_key(&order_id) {
        return Err(err(StatusCode::CONFLICT, "order_id already exists"));
    }
    let o = json!({ "cf_order_id": g.cf_orders.len() + 1, "order_id": order_id, "order_amount": b["order_amount"], "order_currency": b["order_currency"], "order_status": "ACTIVE", "payment_session_id": format!("session_{order_id}") });
    g.cf_orders.insert(order_id, o.clone());
    Ok(Json(o))
}

/// Server-initiated UPI collect (Order Pay).
async fn cf_pay(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let session = b["payment_session_id"].as_str().unwrap_or_default();
    let order_id = session
        .strip_prefix("session_")
        .unwrap_or_default()
        .to_string();
    let behaviour = m.take_behaviour("cashfree.pay");
    let pay = {
        let mut g = m.inner.lock().unwrap();
        let o = g
            .cf_orders
            .get(&order_id)
            .cloned()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "invalid payment_session_id"))?;
        let id = format!("cfpay{:08}", g.cf_payments.len() + 1);
        let p = json!({ "cf_payment_id": id, "order_id": order_id, "payment_amount": o["order_amount"], "payment_status": "PENDING", "payment_method": b["payment_method"], "channel": "collect" });
        g.cf_payments.insert(id, p.clone());
        p
    };
    match behaviour.as_deref() {
        // The collect request IS created at the provider, but the caller never hears back.
        Some("hang_after_accept") => {
            hang().await;
            Ok(Json(pay))
        }
        _ => Ok(Json(pay)),
    }
}

async fn cf_order_payments(State(m): State<Mock>, Path(id): Path<String>) -> Resp {
    let g = m.inner.lock().unwrap();
    if !g.cf_orders.contains_key(&id) {
        return Err(err(StatusCode::NOT_FOUND, "order not found"));
    }
    let items: Vec<Value> = g
        .cf_payments
        .values()
        .filter(|p| p["order_id"] == id.as_str())
        .cloned()
        .collect();
    Ok(Json(json!(items)))
}

async fn cf_capture(State(m): State<Mock>, Path(id): Path<String>, Json(b): Json<Value>) -> Resp {
    let mut g = m.inner.lock().unwrap();
    let _ = b;
    let p = g
        .cf_payments
        .values_mut()
        .find(|p| p["order_id"] == id.as_str() && p["payment_status"] == "AUTHORIZED")
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "no authorized payment"))?;
    p["payment_status"] = json!("SUCCESS");
    let out = p.clone();
    *g.charges.entry("cashfree".into()).or_default() += 1;
    Ok(Json(out))
}

async fn cf_refund(State(m): State<Mock>, Path(id): Path<String>, Json(b): Json<Value>) -> Resp {
    let rid = b["refund_id"]
        .as_str()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "refund_id required"))?
        .to_string();
    let amount = cf_amount(&b["refund_amount"]);
    let behaviour = m.take_behaviour("cashfree.refund");
    let r = {
        let mut g = m.inner.lock().unwrap();
        if let Some(existing) = g.cf_refunds.get(&rid) {
            return Ok(Json(existing.clone()));
        }
        let paid = g
            .cf_payments
            .values()
            .find(|p| p["order_id"] == id.as_str() && p["payment_status"] == "SUCCESS")
            .cloned()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "order has no successful payment"))?;
        let done: i64 = g
            .cf_refunds
            .values()
            .filter(|r| r["order_id"] == id.as_str())
            .map(|r| cf_amount(&r["refund_amount"]))
            .sum();
        if done + amount > cf_amount(&paid["payment_amount"]) {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "refund_amount exceeds the paid amount",
            ));
        }
        let r = json!({ "cf_refund_id": format!("cfrf{:06}", g.cf_refunds.len() + 1), "refund_id": rid, "order_id": id, "refund_amount": b["refund_amount"], "refund_status": "SUCCESS" });
        g.cf_refunds.insert(rid, r.clone());
        r
    };
    if behaviour.as_deref() == Some("hang_after_accept") {
        hang().await;
    }
    Ok(Json(r))
}

async fn cf_get_refund(State(m): State<Mock>, Path((_id, rid)): Path<(String, String)>) -> Resp {
    m.inner
        .lock()
        .unwrap()
        .cf_refunds
        .get(&rid)
        .cloned()
        .map(Json)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "refund not found"))
}

fn cf_delivery(m: &Mock, ty: &str, data: Value) -> Value {
    let ts = chrono::Utc::now().timestamp_millis().to_string();
    let body = json!({ "type": ty, "event_time": chrono::Utc::now().to_rfc3339(), "data": data })
        .to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(m.cfg.cashfree_webhook_secret.as_bytes()).unwrap();
    mac.update(ts.as_bytes());
    mac.update(body.as_bytes());
    let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    json!({ "headers": { "x-webhook-signature": sig, "x-webhook-timestamp": ts, "content-type": "application/json" }, "body": body })
}

/// The payer approves (or declines) the Cashfree payment. `outcome`: SUCCESS | FAILED | AUTHORIZED.
async fn sim_cf_pay(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let order_id = b["order_id"].as_str().unwrap_or_default().to_string();
    let outcome = b["outcome"].as_str().unwrap_or("SUCCESS").to_string();
    let pay = {
        let mut g = m.inner.lock().unwrap();
        let o = g
            .cf_orders
            .get(&order_id)
            .cloned()
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "order not found"))?;
        let existing = g
            .cf_payments
            .iter()
            .find(|(_, p)| p["order_id"] == order_id.as_str() && p["payment_status"] == "PENDING")
            .map(|(k, _)| k.clone());
        let key = existing.unwrap_or_else(|| format!("cfpay{:08}", g.cf_payments.len() + 1));
        let p = g.cf_payments.entry(key.clone()).or_insert_with(|| json!({ "cf_payment_id": key, "order_id": order_id, "payment_amount": o["order_amount"], "payment_method": { "upi": {} } }));
        p["payment_status"] = json!(outcome);
        if outcome == "FAILED" {
            p["error_details"] = json!({ "error_description": "Payment declined by payer bank" });
        }
        let out = p.clone();
        if outcome == "SUCCESS" {
            *g.charges.entry("cashfree".into()).or_default() += 1;
        }
        out
    };
    let ty = if outcome == "FAILED" {
        "PAYMENT_FAILED_WEBHOOK"
    } else {
        "PAYMENT_SUCCESS_WEBHOOK"
    };
    let hook = cf_delivery(
        &m,
        ty,
        json!({ "order": { "order_id": order_id }, "payment": pay }),
    );
    Ok(Json(json!({ "payment": pay, "webhook": hook })))
}

// =============== settlements (shared) ===============

/// Settle everything captured/refunded and not yet settled for `provider`.
/// Body: { fee_bps, tax_bps (on the fee), adjustment_minor, adjustment_note, skip: [payment ids] }
async fn sim_settle(
    State(m): State<Mock>,
    Path(provider): Path<String>,
    Json(b): Json<Value>,
) -> Resp {
    let (fee_bps, tax_bps) = (
        b["fee_bps"].as_i64().unwrap_or(200),
        b["tax_bps"].as_i64().unwrap_or(1800),
    );
    let skip: Vec<String> = b["skip"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut g = m.inner.lock().unwrap();
    let sid = format!(
        "{}{:06}",
        if provider == "razorpay" {
            "setl_"
        } else {
            "cfsetl"
        },
        g.settlements.values().map(Vec::len).sum::<usize>() + 1
    );
    let utr = format!("UTR{}", sid.to_uppercase());
    let mut rows = Vec::new();
    let fee_of = |amt: i64| -> (i64, i64) {
        let fee = (amt * fee_bps + 5_000) / 10_000;
        (fee, (fee * tax_bps + 5_000) / 10_000)
    };
    if provider == "razorpay" {
        let pays: Vec<Value> = g
            .rz_payments
            .values()
            .filter(|p| p["status"] == "captured")
            .cloned()
            .collect();
        for p in pays {
            let id = p["id"].as_str().unwrap().to_string();
            if g.settled.contains(&id) || skip.contains(&id) {
                continue;
            }
            let amt = p["amount"].as_i64().unwrap();
            let (fee, tax) = fee_of(amt);
            rows.push(json!({ "entity_id": id, "type": "payment", "credit": amt, "debit": 0, "fee": fee, "tax": tax, "settlement_id": sid, "settlement_utr": utr }));
            g.settled.insert(id);
        }
        let rfs: Vec<Value> = g.rz_refunds.values().cloned().collect();
        for r in rfs {
            let id = r["id"].as_str().unwrap().to_string();
            if g.settled.contains(&id) {
                continue;
            }
            rows.push(json!({ "entity_id": id, "type": "refund", "credit": 0, "debit": r["amount"], "fee": 0, "tax": 0, "settlement_id": sid, "settlement_utr": utr }));
            g.settled.insert(id);
        }
        if let Some(adj) = b["adjustment_minor"].as_i64() {
            rows.push(json!({ "entity_id": format!("adj_{sid}"), "type": "adjustment", "credit": adj.max(0), "debit": (-adj).max(0), "fee": 0, "tax": 0, "description": b["adjustment_note"].as_str().unwrap_or("adjustment"), "settlement_id": sid, "settlement_utr": utr }));
        }
    } else {
        let pays: Vec<Value> = g
            .cf_payments
            .values()
            .filter(|p| p["payment_status"] == "SUCCESS")
            .cloned()
            .collect();
        for p in pays {
            let id = p["order_id"].as_str().unwrap().to_string();
            if g.settled.contains(&id) || skip.contains(&id) {
                continue;
            }
            let amt = cf_amount(&p["payment_amount"]);
            let (fee, tax) = fee_of(amt);
            rows.push(json!({ "order_id": id, "event_type": "PAYMENT", "event_amount": amt as f64 / 100.0, "service_charge": fee as f64 / 100.0, "service_tax": tax as f64 / 100.0, "cf_settlement_id": sid, "settlement_utr": utr }));
            g.settled.insert(id);
        }
        let rfs: Vec<Value> = g.cf_refunds.values().cloned().collect();
        for r in rfs {
            let rid = format!("rf:{}", r["refund_id"].as_str().unwrap());
            if g.settled.contains(&rid) {
                continue;
            }
            rows.push(json!({ "order_id": r["order_id"], "event_type": "REFUND", "event_amount": r["refund_amount"], "service_charge": 0.0, "service_tax": 0.0, "cf_settlement_id": sid, "settlement_utr": utr }));
            g.settled.insert(rid);
        }
        if let Some(adj) = b["adjustment_minor"].as_i64() {
            rows.push(json!({ "order_id": Value::Null, "event_type": b["adjustment_note"].as_str().unwrap_or("ADJUSTMENT"), "event_amount": adj as f64 / 100.0, "service_charge": 0.0, "service_tax": 0.0, "cf_settlement_id": sid, "settlement_utr": utr }));
        }
    }
    let count = rows.len();
    g.settlements.entry(provider).or_default().extend(rows);
    Ok(Json(
        json!({ "settlement_id": sid, "utr": utr, "rows": count }),
    ))
}

async fn settlement_recon_rz(State(m): State<Mock>) -> Resp {
    let g = m.inner.lock().unwrap();
    Ok(Json(
        json!({ "items": g.settlements.get("razorpay").cloned().unwrap_or_default() }),
    ))
}
async fn settlement_recon_cf(State(m): State<Mock>) -> Resp {
    let g = m.inner.lock().unwrap();
    Ok(Json(
        json!({ "data": g.settlements.get("cashfree").cloned().unwrap_or_default() }),
    ))
}

// =============== control ===============

/// { "op": "cashfree.pay", "behaviour": "hang_after_accept" } affects only the next call.
async fn sim_behaviour(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    m.inner.lock().unwrap().behaviours.insert(
        b["op"].as_str().unwrap_or_default().into(),
        b["behaviour"].as_str().unwrap_or_default().into(),
    );
    Ok(Json(json!({ "ok": true })))
}

/// How many times money was actually taken, per provider and per order.
async fn sim_stats(State(m): State<Mock>) -> Resp {
    let g = m.inner.lock().unwrap();
    let cf_orders: BTreeMap<String, usize> = g
        .cf_payments
        .values()
        .filter(|p| p["payment_status"] == "SUCCESS")
        .fold(BTreeMap::new(), |mut a, p| {
            *a.entry(p["order_id"].as_str().unwrap_or_default().to_string())
                .or_default() += 1;
            a
        });
    Ok(Json(json!({
        "charges": g.charges,
        "cashfree_successful_payments_by_order": cf_orders,
        "cashfree_payments": g.cf_payments.len(),
        "razorpay_payments": g.rz_payments.len(),
    })))
}
