//! A local stand-in for the Razorpay, Cashfree and PayU HTTP APIs plus control endpoints under `/_sim/*`.
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
    /// Sticky outage per provider ("razorpay" / "cashfree"): `reject` (HTTP 400, nothing created: a definitive
    /// answer), `unavailable` (HTTP 503: outcome unknown to the caller) or `hang` (no answer: a timeout).
    outages: BTreeMap<String, String>,
    charges: BTreeMap<String, i64>,
    /// calls received by the Secure ID (verification) endpoints, by path
    kyc_calls: BTreeMap<String, u64>,
    payu: PayuState,
}

/// What the PayU simulation keeps. The credentials are obviously fake and can be changed with
/// `POST /_sim/payu/credentials`; nothing here is, or looks like, a live secret.
struct PayuState {
    key: String,
    salt: String,
    merchant_secret: String,
    /// by our transaction id
    txns: BTreeMap<String, Value>,
    /// by the merchant's refund token
    refunds: BTreeMap<String, Value>,
}

impl Default for PayuState {
    fn default() -> Self {
        Self {
            key: "payu_test_key".into(),
            salt: "payu_test_salt".into(),
            merchant_secret: "payu_test_merchant_secret".into(),
            txns: Default::default(),
            refunds: Default::default(),
        }
    }
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
    /// The sticky outage mode for a provider, if one is set.
    fn outage(&self, provider: &str) -> Option<String> {
        self.inner.lock().unwrap().outages.get(provider).cloned()
    }
    fn take_behaviour(&self, op: &str) -> Option<String> {
        self.inner.lock().unwrap().behaviours.remove(op)
    }
}

/// A twelve-digit made-up bank reference, stable per payment id.
fn mock_rrn(id: &str) -> String {
    let h = id.bytes().fold(1_469_598_103_934_665_603u64, |h, b| {
        (h ^ b as u64).wrapping_mul(1_099_511_628_211)
    });
    format!("{:012}", h % 1_000_000_000_000)
}

type Resp = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn err(code: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (
        code,
        Json(json!({ "error": { "description": msg }, "message": msg })),
    )
}
/// Apply a switched-on outage to a create-order call. `Ok` means carry on normally.
async fn check_outage(m: &Mock, provider: &str) -> Result<(), (StatusCode, Json<Value>)> {
    match m.outage(provider).as_deref() {
        Some("reject") => Err(err(
            StatusCode::BAD_REQUEST,
            "simulated outage: request refused",
        )),
        Some("unavailable") => Err(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "simulated outage: service unavailable",
        )),
        Some("hang") => {
            hang().await;
            Err(err(
                StatusCode::GATEWAY_TIMEOUT,
                "simulated outage: timeout",
            ))
        }
        _ => Ok(()),
    }
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
        .route("/v1/payments/create/upi", post(rz_create_upi))
        .route("/v1/payments/{id}/capture", post(rz_capture))
        .route("/v1/payments/{id}/refund", post(rz_refund))
        .route("/v1/payments/{id}/refunds", get(rz_payment_refunds))
        .route("/v1/settlements/recon/combined", get(settlement_recon_rz))
        // ---- cashfree ----
        .route("/pg/orders/{id}", get(cf_get_order))
        // Cashfree Secure ID (verification), following its API reference and sandbox test data
        .route("/verification/pan", post(kyc_pan))
        .route("/verification/bank-account/sync", post(kyc_bank))
        .route("/verification/gstin", post(kyc_gstin))
        .route("/pg/orders", post(cf_create_order))
        .route("/pg/orders/sessions", post(cf_pay))
        .route("/pg/orders/{id}/payments", get(cf_order_payments))
        .route("/pg/orders/{id}/authorization", post(cf_capture))
        .route("/pg/orders/{id}/refunds", post(cf_refund))
        .route("/pg/orders/{id}/refunds/{rid}", get(cf_get_refund))
        .route("/pg/settlement/recon", post(settlement_recon_cf))
        // ---- payu ----
        .route("/_payment", post(payu_payment))
        .route("/merchant/postservice.php", post(payu_postservice))
        .route("/settlement/range", get(payu_settlement_range))
        .route("/_sim/payu/credentials", post(sim_payu_credentials))
        .route("/_sim/payu/pay", post(sim_payu_pay))
        .route("/_sim/payu/refund_complete", post(sim_payu_refund_complete))
        .route("/_sim/payu/settle", post(sim_payu_settle))
        // ---- control ----
        .route("/_sim/behaviour", post(sim_behaviour))
        .route("/_sim/outage", post(sim_outage))
        .route("/_sim/razorpay/pay", post(sim_rz_pay))
        .route("/_sim/razorpay/refund_event", post(sim_rz_refund_event))
        .route("/_sim/cashfree/pay", post(sim_cf_pay))
        .route("/_sim/cashfree/refund_event", post(sim_cf_refund_event))
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
    check_outage(&m, "razorpay").await?;
    let amount = b["amount"]
        .as_i64()
        .filter(|a| *a >= 100)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "amount must be at least 100 paise"))?;
    let id = m.next("order_");
    let o = json!({ "id": id, "entity": "order", "amount": amount, "currency": b["currency"], "receipt": b["receipt"], "status": "created", "notes": b["notes"] });
    m.inner.lock().unwrap().rz_orders.insert(id, o.clone());
    Ok(Json(o))
}

/// S2S UPI intent / QR create (shape used by the Razorpay adapter's opt-in path).
async fn rz_create_upi(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let order_id = b["order_id"]
        .as_str()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "order_id required"))?
        .to_string();
    let amount = b["amount"]
        .as_i64()
        .filter(|a| *a >= 100)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "amount must be at least 100 paise"))?;
    let flow = b["upi"]["flow"].as_str().unwrap_or("intent");
    let id = m.next("pay_");
    let mut p = json!({
        "id": id, "razorpay_payment_id": id, "entity": "payment", "amount": amount,
        "currency": b["currency"], "order_id": order_id, "status": "created", "method": "upi",
        "notes": b["notes"],
    });
    if flow == "qr" {
        p["qr_image"] = json!("data:image/png;base64,iVBORw0KGgo=");
    } else {
        p["link"] = json!(format!(
            "upi://pay?pa=zyvor@rzp&pn=Zyvor&am={:.2}&cu=INR&tn={}",
            amount as f64 / 100.0,
            id
        ));
    }
    m.inner.lock().unwrap().rz_payments.insert(id, p.clone());
    Ok(Json(p))
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
    if p["acquirer_data"]["rrn"].is_null() {
        p["acquirer_data"] = json!({ "rrn": mock_rrn(p["id"].as_str().unwrap_or("")) });
    }
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
            // a UPI/card payment that went through carries the bank's reference, as Razorpay's `acquirer_data.rrn`
            "acquirer_data": if outcome == "failed" { json!({}) } else { json!({ "rrn": mock_rrn(&id) }) },
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
    check_outage(&m, "cashfree").await?;
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
    if behaviour.as_deref() == Some("reject") {
        return Err(err(StatusCode::BAD_REQUEST, "refund rejected by bank"));
    }
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
        if outcome == "SUCCESS" {
            p["bank_reference"] = json!(mock_rrn(&key));
        }
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

/// Deliver Cashfree's refund-status webhook for a refund we created. `refund_id` is ours.
async fn sim_cf_refund_event(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let rid = b["refund_id"].as_str().unwrap_or_default().to_string();
    let r = m
        .inner
        .lock()
        .unwrap()
        .cf_refunds
        .get(&rid)
        .cloned()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "refund not found"))?;
    let hook = cf_delivery(
        &m,
        "REFUND_STATUS_WEBHOOK",
        json!({ "refund": r, "order": { "order_id": r["order_id"] } }),
    );
    Ok(Json(json!({ "refund": r, "webhook": hook })))
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

/// Switch a provider's outage on or off: `{"provider": "razorpay", "mode": "reject" | "unavailable" | "hang" | "off"}`.
/// Sticky until switched off, unlike `behaviour`, which is one-shot.
async fn sim_outage(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let provider = b["provider"].as_str().unwrap_or_default().to_string();
    let mode = b["mode"].as_str().unwrap_or("off");
    if !matches!(provider.as_str(), "razorpay" | "cashfree") {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "provider must be razorpay or cashfree",
        ));
    }
    if !matches!(mode, "reject" | "unavailable" | "hang" | "off") {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "mode must be reject, unavailable, hang or off",
        ));
    }
    let mut g = m.inner.lock().unwrap();
    if mode == "off" {
        g.outages.remove(&provider);
    } else {
        g.outages.insert(provider, mode.to_string());
    }
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
        "kyc_calls": g.kyc_calls,
        "cashfree_successful_payments_by_order": cf_orders,
        "cashfree_payments": g.cf_payments.len(),
        "razorpay_payments": g.rz_payments.len(),
        "razorpay_orders": g.rz_orders.len(),
        "cashfree_orders": g.cf_orders.len(),
    })))
}

// ---------- Cashfree Secure ID (verification) ----------
//
// Credentials act as switches: a client id `BADID` is refused (401), `NOBAL` has no balance (422), `DOWN` is an outage
// (502). Test data is Cashfree's documented sandbox set (PANs, accounts, IFSCs, GSTINs); anything not listed as invalid is
// treated as valid and echoes the name it was given, except where noted.

type KycResp = (StatusCode, Json<Value>);

fn kyc_gate(m: &Mock, h: &axum::http::HeaderMap, path: &str) -> Option<KycResp> {
    *m.inner
        .lock()
        .unwrap()
        .kyc_calls
        .entry(path.to_string())
        .or_default() += 1;
    let id = h
        .get("x-client-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let (st, ty, code, msg) = match id {
        "" => (
            400,
            "validation_error",
            "x-client-id_missing",
            "x-client-id is missing in the request.",
        ),
        "BADID" => (
            401,
            "authentication_error",
            "authentication_failed",
            "Invalid clientId and clientSecret combination",
        ),
        "NOBAL" => (
            422,
            "validation_error",
            "insufficient_balance",
            "Insufficient balance to process this request",
        ),
        "DOWN" => (
            502,
            "internal_error",
            "verification_failed",
            "Authorised source is temporarily unavailable, please try again shortly",
        ),
        _ => return None,
    };
    Some((
        StatusCode::from_u16(st).unwrap(),
        Json(json!({ "type": ty, "code": code, "message": msg })),
    ))
}

/// Cashfree's name-match bands: exact 100, all words of one name in the other 85, some words in common 61, none 14.
fn kyc_name_match(a: &str, b: &str) -> (&'static str, &'static str) {
    let words = |s: &str| -> std::collections::BTreeSet<String> {
        s.to_uppercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let (x, y) = (words(a), words(b));
    if x == y {
        ("100.00", "DIRECT_MATCH")
    } else if x.is_subset(&y) || y.is_subset(&x) {
        ("85.00", "GOOD_PARTIAL_MATCH")
    } else if x.intersection(&y).next().is_some() {
        ("61.00", "MODERATE_PARTIAL_MATCH")
    } else {
        ("14.00", "NO_MATCH")
    }
}

async fn kyc_pan(State(m): State<Mock>, h: axum::http::HeaderMap, Json(b): Json<Value>) -> KycResp {
    if let Some(r) = kyc_gate(&m, &h, "pan") {
        return r;
    }
    let pan = b["pan"].as_str().unwrap_or_default();
    if pan.len() != 10 {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({ "type": "validation_error", "code": "pan_length_short", "message": "Enter valid PAN." }),
            ),
        );
    }
    if ["DEFPV0126D", "TUVPP5678W", "LMNCD8010T", "EFGHH2020L"].contains(&pan) {
        return (
            StatusCode::OK,
            Json(
                json!({ "pan": pan, "reference_id": 17974435, "name_provided": b["name"], "valid": false, "message": "Invalid PAN", "name_match_score": 0, "name_match_result": "-", "pan_status_code": "N", "pan_status_desc": "Not found in ITD database or Invalid PAN" }),
            ),
        );
    }
    let given = b["name"].as_str().unwrap_or_default();
    // one test PAN is registered to someone else; another has the name-match add-on switched off
    let registered = if pan == "XYZPP4321W" {
        "SOMEONE ELSE ENTIRELY"
    } else {
        given
    };
    let mut out = json!({ "pan": pan, "type": "Individual", "reference_id": 161, "name_provided": given, "registered_name": registered.to_uppercase(), "valid": true, "message": "PAN verified successfully", "pan_status": "VALID", "pan_status_code": "E", "pan_status_desc": "Existing and Valid" });
    if pan != "AAAHU4383C" && !given.is_empty() {
        let (score, result) = kyc_name_match(given, registered);
        out["name_match_score"] = json!(score);
        out["name_match_result"] = json!(result);
    }
    (StatusCode::OK, Json(out))
}

async fn kyc_bank(
    State(m): State<Mock>,
    h: axum::http::HeaderMap,
    Json(b): Json<Value>,
) -> KycResp {
    if let Some(r) = kyc_gate(&m, &h, "bank") {
        return r;
    }
    let (acct, ifsc, given) = (
        b["bank_account"].as_str().unwrap_or_default(),
        b["ifsc"].as_str().unwrap_or_default(),
        b["name"].as_str().unwrap_or_default(),
    );
    let ifsc_ok = ifsc.len() == 11
        && ifsc[..4].chars().all(|c| c.is_ascii_alphabetic())
        && ifsc.as_bytes()[4] == b'0';
    if !ifsc_ok {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({ "type": "validation_error", "code": "ifsc_value_invalid", "message": "ifsc should be of 11 characters and 5th character should be 0." }),
            ),
        );
    }
    let invalid = |code: &str| {
        (
            StatusCode::OK,
            Json(
                json!({ "reference_id": 35, "name_at_bank": null, "bank_name": null, "name_match_score": null, "name_match_result": null, "account_status": "INVALID", "account_status_code": code, "utr": null }),
            ),
        )
    };
    if ifsc == "CNRR0002640" {
        return invalid("INVALID_IFSC_FAIL");
    }
    if acct == "026291800001190" {
        return invalid("INVALID_ACCOUNT_FAIL");
    }
    let at_bank = match acct {
        "1233943142" => "TOM WELLS".to_string(),
        "388108022658" => "BOB HOPE".to_string(),
        _ if given.is_empty() => "ACCOUNT HOLDER".to_string(),
        _ => given.to_uppercase(),
    };
    let mut out = json!({ "reference_id": 34, "name_at_bank": at_bank, "bank_name": "YES BANK", "city": "MUMBAI", "branch": "SANTACRUZ, MUMBAI", "account_status": "VALID", "account_status_code": "ACCOUNT_IS_VALID", "utr": "404223241811" });
    if !given.is_empty() {
        let (score, result) = kyc_name_match(given, &at_bank);
        out["name_match_score"] = json!(score);
        out["name_match_result"] = json!(result);
    }
    (StatusCode::OK, Json(out))
}

async fn kyc_gstin(
    State(m): State<Mock>,
    h: axum::http::HeaderMap,
    Json(b): Json<Value>,
) -> KycResp {
    if let Some(r) = kyc_gate(&m, &h, "gstin") {
        return r;
    }
    let g = b["GSTIN"].as_str().unwrap_or_default();
    if g.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({ "type": "validation_error", "code": "GSTIN_missing", "message": "GSTIN is missing in the request." }),
            ),
        );
    }
    // the documented non-existent test numbers fail the checksum here, so "does not exist" goes by the PAN inside
    if ["29AAIZP2912R1ZR", "05BBICA2912R1ZR"].contains(&g)
        || matches!(g.get(2..12), Some("AAIZP2912R" | "BBICA2912R"))
    {
        return (
            StatusCode::OK,
            Json(json!({ "reference_id": 20, "GSTIN": g, "message": "GSTIN Doesn't Exist" })),
        );
    }
    // a GSTIN whose entity character (13th) is 9 is a registration that has been cancelled
    let status = if g.as_bytes().get(12) == Some(&b'9') {
        "Cancelled"
    } else {
        "Active"
    };
    (
        StatusCode::OK,
        Json(
            json!({ "reference_id": 19, "GSTIN": g, "legal_name_of_business": "KIRAN TRADERS PRIVATE LIMITED", "trade_name_of_business": "KIRAN TRADERS", "gst_in_status": status, "taxpayer_type": "Regular", "valid": true, "message": "GSTIN Exists" }),
        ),
    )
}

// =============== PayU ===============
//
// Built from PayU's public documentation (docs.payu.in), the same pages the adapter was built from, so the two can
// agree with each other and still both be wrong about PayU. It is a SIMULATION, not PayU's sandbox. Hosted checkout
// (`/_payment`), Verify Payment and the refund commands (`/merchant/postservice.php`), the Settlement Detail Range
// report (`/settlement/range`) and the payment webhook (returned by `/_sim/payu/pay` for the test to deliver).

fn sha512_hex(parts: &[&str]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha512::digest(parts.join("|").as_bytes()))
}

/// A decimal-rupee string ("499.00") from paise, and back, both without floats.
fn payu_dec(minor: i64) -> String {
    format!("{}.{:02}", minor / 100, minor % 100)
}
fn payu_minor(s: &str) -> Option<i64> {
    let (w, f) = s.split_once('.').unwrap_or((s, ""));
    let f = format!("{f:0<2}");
    Some(w.parse::<i64>().ok()? * 100 + f.get(..2)?.parse::<i64>().ok()?)
}

type Form = axum::Form<BTreeMap<String, String>>;

/// The payer's browser posts the checkout form. The hash is checked as PayU does; a good one starts the transaction.
async fn payu_payment(State(m): State<Mock>, axum::Form(f): Form) -> Resp {
    let mut g = m.inner.lock().unwrap();
    let p = &mut g.payu;
    let v = |k: &str| f.get(k).map(String::as_str).unwrap_or("");
    for required in [
        "key",
        "txnid",
        "amount",
        "productinfo",
        "firstname",
        "email",
        "phone",
        "surl",
        "furl",
        "hash",
    ] {
        if v(required).is_empty() {
            return Err(err(
                StatusCode::BAD_REQUEST,
                &format!("{required} is mandatory"),
            ));
        }
    }
    let want = sha512_hex(&[
        v("key"),
        v("txnid"),
        v("amount"),
        v("productinfo"),
        v("firstname"),
        v("email"),
        v("udf1"),
        v("udf2"),
        v("udf3"),
        v("udf4"),
        v("udf5"),
        "",
        "",
        "",
        "",
        "",
        &p.salt,
    ]);
    if v("key") != p.key || want != v("hash") {
        return Err(err(StatusCode::BAD_REQUEST, "invalid hash"));
    }
    if v("txnid").len() > 25 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "txnid is longer than 25 characters",
        ));
    }
    if payu_minor(v("amount")).is_none_or(|a| a < 100) {
        return Err(err(StatusCode::BAD_REQUEST, "amount must be at least 1.00"));
    }
    let txnid = v("txnid").to_string();
    if p.txns
        .get(&txnid)
        .is_some_and(|t| t["unmappedstatus"] == "captured")
    {
        return Err(err(
            StatusCode::CONFLICT,
            "this transaction was already processed successfully",
        ));
    }
    let t = json!({
        "txnid": txnid, "amount": v("amount"), "key": v("key"), "productinfo": v("productinfo"),
        "firstname": v("firstname"), "email": v("email"), "phone": v("phone"),
        "udf1": v("udf1"), "udf2": v("udf2"), "udf3": v("udf3"), "udf4": v("udf4"), "udf5": v("udf5"),
        "status": "pending", "unmappedstatus": "initiated", "mihpayid": Value::Null,
    });
    p.txns.insert(txnid, t.clone());
    Ok(Json(json!({ "txnid": t["txnid"], "status": "initiated" })))
}

fn payu_auth(
    f: &BTreeMap<String, String>,
    salt: &str,
    key: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    let v = |k: &str| f.get(k).map(String::as_str).unwrap_or("");
    let want = sha512_hex(&[key, v("command"), v("var1"), salt]);
    if v("key") != key || v("hash") != want {
        return Err((
            StatusCode::OK,
            Json(json!({ "status": 0, "msg": "Invalid hash", "transaction_details": {} })),
        ));
    }
    Ok(())
}

/// Verify Payment, the refund command and Check Action Status, as the documentation describes them.
async fn payu_postservice(State(m): State<Mock>, axum::Form(f): Form) -> Resp {
    let mut g = m.inner.lock().unwrap();
    let (key, salt) = (g.payu.key.clone(), g.payu.salt.clone());
    payu_auth(&f, &salt, &key)?;
    let v = |k: &str| f.get(k).map(String::as_str).unwrap_or("").to_string();
    match v("command").as_str() {
        "verify_payment" => {
            let mut details = serde_json::Map::new();
            let mut found = 0;
            let ids: Vec<String> = v("var1").split('|').map(str::to_string).collect();
            for id in &ids {
                match g.payu.txns.get(id) {
                    Some(t) if !t["mihpayid"].is_null() || t["unmappedstatus"] != "initiated" => {
                        found += 1;
                        details.insert(id.clone(), t.clone());
                    }
                    // a transaction the payer has not reached is "Not Found", as the documentation shows
                    _ => {
                        details.insert(
                            id.clone(),
                            json!({ "mihpayid": "Not Found", "status": "Not Found" }),
                        );
                    }
                }
            }
            Ok(Json(json!({
                "status": if found == ids.len() { 1 } else { 0 },
                "msg": format!("{found} out of {} Transactions Fetched Successfully", ids.len()),
                "transaction_details": details,
            })))
        }
        "cancel_refund_transaction" => {
            let (payu_id, token, amount) = (v("var1"), v("var2"), v("var3"));
            let fail = |msg: &str| Ok(Json(json!({ "status": 0, "msg": msg })));
            if token.is_empty() {
                return fail("token is empty");
            }
            if amount.is_empty() {
                return fail("amount is empty");
            }
            if token.len() > 23 {
                return fail("token is longer than 23 characters");
            }
            if g.payu.refunds.contains_key(&token) {
                return fail("token already used or request pending");
            }
            let Some(txn) = g
                .payu
                .txns
                .values()
                .find(|t| t["mihpayid"] == payu_id.as_str())
                .cloned()
            else {
                return fail("transaction not exists");
            };
            let paid = payu_minor(txn["amount"].as_str().unwrap_or("0")).unwrap_or(0);
            let want = payu_minor(&amount).unwrap_or(0);
            let done: i64 = g
                .payu
                .refunds
                .values()
                .filter(|r| r["mihpayid"] == payu_id.as_str())
                .filter_map(|r| payu_minor(r["amt"].as_str()?))
                .sum();
            if txn["unmappedstatus"] != "captured" || want <= 0 || done + want > paid {
                return fail("Refund request failed");
            }
            let request_id = format!("{}", 100_000_000 + g.payu.refunds.len() as u64 + 1);
            let r = json!({
                "action": "refund", "status": "QUEUED", "token": token, "amt": amount,
                "mihpayid": payu_id, "request_id": request_id, "mode": "UPI",
                "bank_ref_num": Value::Null, "refund_mode": "Back to Source",
            });
            g.payu.refunds.insert(token, r);
            Ok(Json(json!({
                "status": 1, "msg": "Refund Request Queued", "request_id": request_id,
                "bank_ref_num": Value::Null, "mihpayid": payu_id,
            })))
        }
        "check_action_status" => {
            let payu_id = v("var1");
            let mut actions = serde_json::Map::new();
            if let Some(t) = g
                .payu
                .txns
                .values()
                .find(|t| t["mihpayid"] == payu_id.as_str())
            {
                actions.insert("1".into(), json!({ "action": "capture", "status": "SUCCESS", "amt": t["amount"], "mihpayid": payu_id, "request_id": "1" }));
            }
            for r in g
                .payu
                .refunds
                .values()
                .filter(|r| r["mihpayid"] == payu_id.as_str())
            {
                actions.insert(
                    r["request_id"].as_str().unwrap_or("").to_string(),
                    r.clone(),
                );
            }
            if actions.is_empty() {
                return Ok(Json(json!({
                    "status": 0, "msg": "0 out of 1 Transactions Fetched Successfully",
                    "transaction_details": { payu_id: "No action status found" },
                })));
            }
            Ok(Json(json!({
                "status": 1, "msg": "1 out of 1 Transactions Fetched Successfully",
                "transaction_details": { payu_id: actions },
            })))
        }
        other => Ok(Json(
            json!({ "status": 0, "msg": format!("unknown command {other}") }),
        )),
    }
}

fn payu_webhook_body(salt: &str, t: &Value) -> String {
    let g = |k: &str| t[k].as_str().unwrap_or("");
    let hash = sha512_hex(&[
        salt,
        g("status"),
        "",
        "",
        "",
        "",
        "",
        g("udf5"),
        g("udf4"),
        g("udf3"),
        g("udf2"),
        g("udf1"),
        g("email"),
        g("firstname"),
        g("productinfo"),
        g("amount"),
        g("txnid"),
        g("key"),
    ]);
    let mut s = form_urlencoded::Serializer::new(String::new());
    for (k, v) in [
        ("mihpayid", g("mihpayid")),
        ("mode", "UPI"),
        ("status", g("status")),
        ("key", g("key")),
        ("txnid", g("txnid")),
        ("amount", g("amount")),
        ("productinfo", g("productinfo")),
        ("firstname", g("firstname")),
        ("email", g("email")),
        ("phone", g("phone")),
        ("udf1", g("udf1")),
        ("udf2", g("udf2")),
        ("udf3", g("udf3")),
        ("udf4", g("udf4")),
        ("udf5", g("udf5")),
        ("unmappedstatus", g("unmappedstatus")),
        ("bank_ref_num", g("bank_ref_num")),
        ("error", g("error")),
        ("error_Message", g("error_Message")),
        ("PG_TYPE", "UPI-PG"),
        ("hash", &hash),
    ] {
        s.append_pair(k, v);
    }
    s.finish()
}

/// The payer finishes (or abandons) the hosted page. `outcome`: captured | failed | auth. Returns the form-encoded
/// webhook PayU would POST, with its reverse hash, for the test to deliver (or tamper with, or repeat).
async fn sim_payu_pay(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let txnid = b["txnid"].as_str().unwrap_or_default().to_string();
    let outcome = b["outcome"].as_str().unwrap_or("captured");
    let mut g = m.inner.lock().unwrap();
    let n = g.payu.txns.len();
    let salt = g.payu.salt.clone();
    let t = g
        .payu
        .txns
        .get_mut(&txnid)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "transaction not found"))?;
    if t["mihpayid"].is_null() {
        t["mihpayid"] = json!(format!("{}", 403_993_715_000_000_000u64 + n as u64 + 1));
    }
    let mihpayid = t["mihpayid"].as_str().unwrap_or("").to_string();
    match outcome {
        "captured" | "auth" => {
            t["status"] = json!("success");
            t["unmappedstatus"] = json!(outcome);
            t["bank_ref_num"] = json!(mock_rrn(&mihpayid));
            t["error"] = json!("E000");
            t["error_Message"] = json!("NO ERROR");
        }
        _ => {
            t["status"] = json!("failure");
            t["unmappedstatus"] = json!("failed");
            t["error"] = json!("E308");
            t["error_Message"] = json!("Bank denied the transaction");
        }
    }
    let t = t.clone();
    if outcome == "captured" {
        *g.charges.entry("payu".into()).or_default() += 1;
    }
    let body = payu_webhook_body(&salt, &t);
    Ok(Json(json!({
        "payment": t,
        "webhook": { "headers": { "content-type": "application/x-www-form-urlencoded" }, "body": body },
    })))
}

/// PayU finishes a queued refund. `outcome`: success | failure.
async fn sim_payu_refund_complete(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let id = b["request_id"].as_str().unwrap_or_default();
    let mut g = m.inner.lock().unwrap();
    let r = g
        .payu
        .refunds
        .values_mut()
        .find(|r| r["request_id"] == id)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "refund not found"))?;
    let ok = b["outcome"].as_str().unwrap_or("success") == "success";
    r["status"] = json!(if ok { "success" } else { "failure" });
    if ok {
        r["bank_ref_num"] = json!(mock_rrn(id));
    }
    Ok(Json(r.clone()))
}

async fn sim_payu_credentials(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let mut g = m.inner.lock().unwrap();
    let p = &mut g.payu;
    if let Some(v) = b["key"].as_str() {
        p.key = v.to_string();
    }
    if let Some(v) = b["salt"].as_str() {
        p.salt = v.to_string();
    }
    if let Some(v) = b["merchant_secret"].as_str() {
        p.merchant_secret = v.to_string();
    }
    Ok(Json(json!({ "ok": true })))
}

/// Settle everything captured or refunded and not yet settled, in the shape of PayU's Settlement Detail Range report.
/// Body as for the other providers: `fee_bps`, `tax_bps` (on the fee), `adjustment_minor` (signed).
async fn sim_payu_settle(State(m): State<Mock>, Json(b): Json<Value>) -> Resp {
    let (fee_bps, tax_bps) = (
        b["fee_bps"].as_i64().unwrap_or(200),
        b["tax_bps"].as_i64().unwrap_or(1800),
    );
    let mut g = m.inner.lock().unwrap();
    let sid = format!(
        "1212729820250826{:04}",
        g.settlements.get("payu").map_or(0, Vec::len) + 1
    );
    let (mut gross, mut net_total, mut txns, mut refunds_total) = (0i64, 0i64, Vec::new(), 0i64);
    let pays: Vec<Value> = g
        .payu
        .txns
        .values()
        .filter(|t| t["unmappedstatus"] == "captured")
        .cloned()
        .collect();
    for t in pays {
        let id = format!("payu:{}", t["txnid"].as_str().unwrap_or(""));
        if g.settled.contains(&id) {
            continue;
        }
        let amt = payu_minor(t["amount"].as_str().unwrap_or("0")).unwrap_or(0);
        let fee = (amt * fee_bps + 5_000) / 10_000;
        let tax = (fee * tax_bps + 5_000) / 10_000;
        gross += amt;
        net_total += amt - fee - tax;
        txns.push(json!({
            "action": "capture", "payuId": t["mihpayid"], "requestId": "1", "transactionAmount": format!("{}0", payu_dec(amt)),
            "merchantServiceFee": format!("{}00", payu_dec(fee)), "merchantServiceTax": format!("{}00", payu_dec(tax)),
            "merchantNetAmount": payu_dec(amt - fee - tax), "merchantTransactionId": t["txnid"], "mode": "UPI",
            "paymentStatus": "captured", "transactionCurrency": "INR", "settlementCurrency": "INR",
        }));
        g.settled.insert(id);
    }
    let rfs: Vec<Value> = g
        .payu
        .refunds
        .values()
        .filter(|r| r["status"] == "success")
        .cloned()
        .collect();
    for r in rfs {
        let id = format!("payu:refund:{}", r["token"].as_str().unwrap_or(""));
        if g.settled.contains(&id) {
            continue;
        }
        let amt = payu_minor(r["amt"].as_str().unwrap_or("0")).unwrap_or(0);
        refunds_total += amt;
        net_total -= amt;
        txns.push(json!({
            "action": "refund", "payuId": r["mihpayid"], "requestId": r["request_id"],
            "transactionAmount": payu_dec(amt), "merchantNetAmount": "", "mode": "UPI",
            "paymentStatus": "refund",
        }));
        g.settled.insert(id);
    }
    let adjustment = b["adjustment_minor"].as_i64().unwrap_or(0);
    if adjustment != 0 {
        net_total += adjustment;
        txns.push(json!({
            "action": if adjustment < 0 { "ADJ_debit" } else { "ADJ_credit" }, "payuId": format!("ADJ_{sid}"),
            "requestId": "", "transactionAmount": payu_dec(adjustment.abs()), "merchantNetAmount": "", "mode": "debit",
        }));
    }
    let row = json!({
        "settlementId": sid, "settlementCompletedDate": format!("{} 02:51:22.000000", chrono::Utc::now().date_naive()),
        "settlementAmount": payu_dec(net_total), "merchantId": 12127298, "utrNumber": format!("UTRPAYU{sid}"),
        "transactionAmount": payu_dec(gross), "adjustmentAmount": payu_dec(adjustment),
        "refundAmount": payu_dec(refunds_total), "transaction": txns,
    });
    let utr = row["utrNumber"].clone();
    g.settlements.entry("payu".into()).or_default().push(row);
    Ok(Json(json!({ "settlement_id": sid, "utr": utr })))
}

/// The settlement report. The `Authorization` header is checked against the merchant secret the way the
/// documentation describes: `sha512("" + "|" + Date + "|" + secret)`.
async fn payu_settlement_range(
    State(m): State<Mock>,
    headers: axum::http::HeaderMap,
    Query(q): Query<BTreeMap<String, String>>,
) -> Resp {
    let g = m.inner.lock().unwrap();
    let h = |n: &str| {
        headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    };
    let want = sha512_hex(&["", &h("date"), &g.payu.merchant_secret]);
    let auth_ok = h("authorization").contains(&format!("signature=\"{want}\""))
        && h("authorization").contains(&format!("username=\"{}\"", g.payu.key));
    let unauthorized = || {
        Ok(Json(
            json!({ "status": 1, "message": "Unauthorized: Invalid signature", "result": null }),
        ))
    };
    if !auth_ok {
        return unauthorized();
    }
    let rows = g.settlements.get("payu").cloned().unwrap_or_default();
    if rows.is_empty() {
        return Ok(Json(
            json!({ "status": 1, "message": "No data found", "result": "No data found" }),
        ));
    }
    let size: usize = q
        .get("pageSize")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let page: usize = q.get("page").and_then(|s| s.parse().ok()).unwrap_or(1);
    let data: Vec<Value> = rows
        .iter()
        .skip((page - 1) * size)
        .take(size)
        .cloned()
        .collect();
    Ok(Json(json!({
        "status": 0,
        "result": { "page": page, "size": data.len(), "totalCount": rows.len(), "data": data },
    })))
}
