//! Two small fake gateways and a fault-injecting proxy for the conformance suite (SIMULATED, never a live service).
//!
//! * `custom_router` is the reference implementation of `docs/zyvor/PROVIDER_CONTRACT.md`: what the `custom` and
//!   `sponsor_upi` adapters talk to.
//! * `phonepe_router` is a fake of the thin PhonePe shape the `phonepe` adapter speaks (there is no PhonePe entry
//!   in `zyvor-provider-mock`).
//! * [`proxy`] sits in front of any HTTP fake and can make the next calls misbehave in the ways the contract
//!   cares about: a definite refusal, a 5xx, no answer at all, an answer lost after the provider acted, and an
//!   unreadable answer.

use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

// ------------------------------------------------------------------ the fault-injecting proxy

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Pass everything through.
    None,
    /// Answer with this status and do not forward (a 400 is a definite no, a 5xx may have happened).
    Status(u16),
    /// Never answer (the caller's timeout fires). Nothing was forwarded.
    Hang,
    /// Forward the call, so the provider acts, then never answer: the response was lost.
    LoseResponse,
    /// Answer 200 with a body that is not JSON.
    Garbled,
}

#[derive(Clone)]
pub struct Faults(Arc<Mutex<Fault>>);

impl Faults {
    pub fn set(&self, f: Fault) {
        *self.0.lock().unwrap() = f;
    }
    pub fn clear(&self) {
        self.set(Fault::None);
    }
}

#[derive(Clone)]
struct ProxyState {
    upstream: String,
    fault: Faults,
    http: reqwest::Client,
}

/// Start a proxy in front of `upstream`. Returns its base URL and the handle that switches faults on and off.
pub async fn proxy(upstream: String) -> (String, Faults) {
    let faults = Faults(Arc::new(Mutex::new(Fault::None)));
    let st = ProxyState {
        upstream,
        fault: faults.clone(),
        http: reqwest::Client::new(),
    };
    let app = Router::new().fallback(forward).with_state(st);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.ok() });
    (url, faults)
}

async fn forward(State(st): State<ProxyState>, req: Request<Body>) -> Response {
    let fault = *st.fault.0.lock().unwrap();
    match fault {
        Fault::Status(code) => {
            return (
                StatusCode::from_u16(code).unwrap(),
                Json(json!({ "message": "injected fault" })),
            )
                .into_response();
        }
        Fault::Hang => {
            tokio::time::sleep(Duration::from_secs(30)).await;
            return StatusCode::GATEWAY_TIMEOUT.into_response();
        }
        Fault::Garbled => return (StatusCode::OK, "<html>not json</html>").into_response(),
        Fault::None | Fault::LoseResponse => {}
    }
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20)
        .await
        .unwrap_or_default();
    let path = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let mut up = st
        .http
        .request(parts.method, format!("{}{path}", st.upstream))
        .body(bytes);
    for (k, v) in &parts.headers {
        if k != "host" && k != "content-length" {
            up = up.header(k, v);
        }
    }
    let res = up.send().await;
    if fault == Fault::LoseResponse {
        // the provider has acted; the answer never arrives
        tokio::time::sleep(Duration::from_secs(30)).await;
        return StatusCode::GATEWAY_TIMEOUT.into_response();
    }
    match res {
        Ok(r) => {
            let status = StatusCode::from_u16(r.status().as_u16()).unwrap();
            let ct = r.headers().get("content-type").cloned();
            let body = r.bytes().await.unwrap_or_default();
            let mut out = (status, body).into_response();
            if let Some(ct) = ct {
                out.headers_mut().insert("content-type", ct);
            }
            out
        }
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

// ------------------------------------------------------------------ the gateway's own state

pub const GW_KEY: &str = "gw_test_api_key";
pub const GW_WEBHOOK_SECRET: &[u8] = b"gw_test_webhook_secret";
pub const PP_WEBHOOK_SECRET: &[u8] = b"pp_test_webhook_secret";

#[derive(Clone)]
struct Pay {
    attempt_id: String,
    pay_ref: String,
    amount: i64,
    status: &'static str,
    refunded: i64,
}

#[derive(Clone)]
struct Rf {
    refund_id: String,
    rf_ref: String,
    pay_ref: String,
    amount: i64,
}

#[derive(Default)]
pub struct Core {
    seq: u64,
    pays: BTreeMap<String, Pay>,
    refunds: BTreeMap<String, Rf>,
    settled: HashSet<String>,
    settlements: Vec<Value>,
}

type Shared = Arc<Mutex<Core>>;

impl Core {
    fn next(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn create(&mut self, attempt: &str, amount: i64) -> Result<Pay, (StatusCode, &'static str)> {
        if amount <= 0 {
            return Err((StatusCode::UNPROCESSABLE_ENTITY, "amount must be positive"));
        }
        // creating again with the same reference returns the payment that already exists
        if let Some(p) = self.pays.get(attempt) {
            return Ok(p.clone());
        }
        let n = self.next();
        let p = Pay {
            attempt_id: attempt.into(),
            pay_ref: format!("gw_{n:06}"),
            amount,
            status: "pending",
            refunded: 0,
        };
        self.pays.insert(attempt.into(), p.clone());
        Ok(p)
    }

    fn refund(
        &mut self,
        refund_id: &str,
        pay_ref: &str,
        amount: i64,
    ) -> Result<Rf, (StatusCode, &'static str)> {
        if let Some(r) = self.refunds.get(refund_id) {
            return Ok(r.clone());
        }
        let n = self.next();
        let p = self
            .pays
            .values_mut()
            .find(|p| p.pay_ref == pay_ref)
            .ok_or((StatusCode::NOT_FOUND, "no such payment"))?;
        if p.status != "captured" {
            return Err((StatusCode::CONFLICT, "payment is not captured"));
        }
        if amount <= 0 || p.refunded + amount > p.amount {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                "refund exceeds the payment",
            ));
        }
        p.refunded += amount;
        let r = Rf {
            refund_id: refund_id.into(),
            rf_ref: format!("rf_{n:06}"),
            pay_ref: pay_ref.into(),
            amount,
        };
        self.refunds.insert(refund_id.into(), r.clone());
        Ok(r)
    }

    fn settle(&mut self, fee_bps: i64, tax_bps: i64, adjustment: i64) -> Value {
        let n = self.next();
        let mut items = Vec::new();
        let fee_of = |amt: i64| {
            let fee = (amt * fee_bps + 5_000) / 10_000;
            fee + (fee * tax_bps + 5_000) / 10_000
        };
        for p in self.pays.values().filter(|p| p.status == "captured") {
            if self.settled.insert(format!("pay:{}", p.pay_ref)) {
                items.push(json!({ "kind": "payment", "provider_ref": p.pay_ref, "amount_minor": p.amount, "description": "payment" }));
                items.push(json!({ "kind": "fee", "provider_ref": p.pay_ref, "amount_minor": -fee_of(p.amount), "description": "gateway fee" }));
            }
        }
        for r in self.refunds.values() {
            if self.settled.insert(format!("rf:{}", r.rf_ref)) {
                items.push(json!({ "kind": "refund", "provider_ref": r.rf_ref, "amount_minor": -r.amount, "description": "refund" }));
            }
        }
        if adjustment != 0 {
            items.push(json!({ "kind": "adjustment", "provider_ref": null, "amount_minor": adjustment, "description": "adjustment" }));
        }
        let net: i64 = items
            .iter()
            .map(|i| i["amount_minor"].as_i64().unwrap())
            .sum();
        let s = json!({
            "provider_ref": format!("st_{n:04}"), "utr": format!("UTR{n:010}"), "currency": "INR",
            "settled_on": chrono::Utc::now().date_naive().to_string(), "net_minor": net, "items": items,
        });
        self.settlements.push(s.clone());
        s
    }
}

fn pay_json(p: &Pay, action: Value) -> Value {
    json!({ "status": p.status, "provider_payment_ref": p.pay_ref, "action": action })
}

fn action_for(method: &str, vpa: &Value, intent: bool, attempt: &str) -> Value {
    match (method, vpa.as_str()) {
        ("upi", Some(v)) => json!({ "type": "upi_collect", "vpa": v }),
        ("upi", None) if intent => {
            json!({ "type": "upi_intent", "uri": format!("upi://pay?pa=gw@sandbox&tn={attempt}"), "apps": {}, "expires_at": null })
        }
        ("card", _) | ("upi", None) => {
            json!({ "type": "hosted_checkout", "params": { "provider": "gateway" } })
        }
        _ => json!({ "type": "none" }),
    }
}

fn webhook(core: &mut Core, secret: &[u8], sig_header: &str, body: Value) -> Value {
    let n = core.next();
    let mut body = body;
    body["event_id"] = json!(format!("evt_{n:06}"));
    let body = body.to_string();
    let ts = chrono::Utc::now().timestamp().to_string();
    let mut m = Hmac::<Sha256>::new_from_slice(secret).unwrap();
    m.update(ts.as_bytes());
    m.update(b".");
    m.update(body.as_bytes());
    json!({ "headers": { sig_header: hex::encode(m.finalize().into_bytes()), "x-zyvor-timestamp": ts }, "body": body })
}

type Resp = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn no(code: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "message": msg })))
}

// ------------------------------------------------------------------ control endpoints (both gateways)

async fn sim_pay(
    State(c): State<Shared>,
    Json(b): Json<Value>,
    secret: &'static [u8],
    sig: &'static str,
) -> Resp {
    let attempt = b["attempt_id"].as_str().unwrap_or_default();
    let ok = b["outcome"] == "captured";
    let mut g = c.lock().unwrap();
    let p = g
        .pays
        .get_mut(attempt)
        .ok_or_else(|| no(StatusCode::NOT_FOUND, "unknown payment"))?;
    p.status = if ok { "captured" } else { "failed" };
    let p = p.clone();
    let event = json!({
        "type": if ok { "payment.captured" } else { "payment.failed" }, "attempt_id": attempt,
        "provider_payment_ref": p.pay_ref, "amount_minor": p.amount,
        "failure_reason": if ok { Value::Null } else { json!("declined_by_bank") },
    });
    let hook = webhook(&mut g, secret, sig, event);
    Ok(Json(json!({ "webhook": hook })))
}

async fn sim_refund_event(
    State(c): State<Shared>,
    Json(b): Json<Value>,
    secret: &'static [u8],
    sig: &'static str,
) -> Resp {
    let rid = b["refund_id"].as_str().unwrap_or_default();
    let mut g = c.lock().unwrap();
    let r = g
        .refunds
        .get(rid)
        .cloned()
        .ok_or_else(|| no(StatusCode::NOT_FOUND, "unknown refund"))?;
    let attempt = g
        .pays
        .values()
        .find(|p| p.pay_ref == r.pay_ref)
        .map(|p| p.attempt_id.clone());
    let event = json!({
        "type": "refund.processed", "attempt_id": attempt, "refund_id": r.refund_id,
        "provider_refund_ref": r.rf_ref, "amount_minor": r.amount,
    });
    let hook = webhook(&mut g, secret, sig, event);
    Ok(Json(json!({ "webhook": hook })))
}

async fn sim_settle(State(c): State<Shared>, Json(b): Json<Value>) -> Resp {
    let s = c.lock().unwrap().settle(
        b["fee_bps"].as_i64().unwrap_or(200),
        b["tax_bps"].as_i64().unwrap_or(1800),
        b["adjustment_minor"].as_i64().unwrap_or(0),
    );
    Ok(Json(s))
}

// ------------------------------------------------------------------ the `custom` contract gateway

fn authed(h: &HeaderMap) -> Result<(), (StatusCode, Json<Value>)> {
    if h.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {GW_KEY}")) {
        Ok(())
    } else {
        Err(no(StatusCode::UNAUTHORIZED, "bad key"))
    }
}

pub fn custom_router() -> Router {
    let core: Shared = Default::default();
    Router::new()
        .route(
            "/payments",
            post(
                |State(c): State<Shared>, h: HeaderMap, Json(b): Json<Value>| async move {
                    authed(&h)?;
                    let method = b["method"].as_str().unwrap_or_default();
                    if !["upi", "card"].contains(&method) {
                        return Err(no(StatusCode::UNPROCESSABLE_ENTITY, "unsupported method"));
                    }
                    if b["currency"] != "INR" {
                        return Err(no(StatusCode::UNPROCESSABLE_ENTITY, "unsupported currency"));
                    }
                    let attempt = b["attempt_id"].as_str().unwrap_or_default();
                    let p = c
                        .lock()
                        .unwrap()
                        .create(attempt, b["amount_minor"].as_i64().unwrap_or(0))
                        .map_err(|(s, m)| no(s, m))?;
                    let action = action_for(method, &b["vpa"], b["upi_intent"] == true, attempt);
                    Ok::<_, (StatusCode, Json<Value>)>(Json(pay_json(&p, action)))
                },
            ),
        )
        .route(
            "/payments/{id}",
            get(
                |State(c): State<Shared>, Path(id): Path<String>| async move {
                    c.lock()
                        .unwrap()
                        .pays
                        .get(&id)
                        .map(|p| Json(pay_json(p, json!({ "type": "none" }))))
                        .ok_or(StatusCode::NOT_FOUND)
                },
            ),
        )
        .route(
            "/payments/{id}/capture",
            post(
                |State(c): State<Shared>, Path(r): Path<String>| async move {
                    let g = c.lock().unwrap();
                    let p = g
                        .pays
                        .values()
                        .find(|p| p.pay_ref == r)
                        .ok_or(StatusCode::NOT_FOUND)?;
                    Ok::<_, StatusCode>(Json(pay_json(p, json!({ "type": "none" }))))
                },
            ),
        )
        .route(
            "/refunds",
            post(|State(c): State<Shared>, Json(b): Json<Value>| async move {
                let r = c
                    .lock()
                    .unwrap()
                    .refund(
                        b["refund_id"].as_str().unwrap_or_default(),
                        b["provider_payment_ref"].as_str().unwrap_or_default(),
                        b["amount_minor"].as_i64().unwrap_or(0),
                    )
                    .map_err(|(s, m)| no(s, m))?;
                Ok::<_, (StatusCode, Json<Value>)>(Json(
                    json!({ "status": "processed", "provider_refund_ref": r.rf_ref }),
                ))
            }),
        )
        .route(
            "/refunds/{id}",
            get(
                |State(c): State<Shared>, Path(id): Path<String>| async move {
                    c.lock()
                        .unwrap()
                        .refunds
                        .get(&id)
                        .map(|r| {
                            Json(json!({ "status": "processed", "provider_refund_ref": r.rf_ref }))
                        })
                        .ok_or(StatusCode::NOT_FOUND)
                },
            ),
        )
        .route(
            "/settlements",
            get(
                |State(c): State<Shared>, Query(_q): Query<BTreeMap<String, String>>| async move {
                    Json(json!({ "settlements": c.lock().unwrap().settlements.clone() }))
                },
            ),
        )
        .route(
            "/_sim/pay",
            post(|s: State<Shared>, b: Json<Value>| {
                sim_pay(s, b, GW_WEBHOOK_SECRET, "x-zyvor-signature")
            }),
        )
        .route(
            "/_sim/refund_event",
            post(|s: State<Shared>, b: Json<Value>| {
                sim_refund_event(s, b, GW_WEBHOOK_SECRET, "x-zyvor-signature")
            }),
        )
        .route("/_sim/settle", post(sim_settle))
        .with_state(core)
}

// ------------------------------------------------------------------ the PhonePe-shaped fake

pub fn phonepe_router() -> Router {
    let core: Shared = Default::default();
    Router::new()
        .route(
            "/pg/v1/pay",
            post(|State(c): State<Shared>, Json(b): Json<Value>| async move {
                let method = b["method"].as_str().unwrap_or_default();
                let attempt = b["merchantTransactionId"].as_str().unwrap_or_default();
                let p = c
                    .lock()
                    .unwrap()
                    .create(attempt, b["amount"].as_i64().unwrap_or(0))
                    .map_err(|(s, m)| no(s, m))?;
                let mut v = pay_json(&p, json!({ "type": "none" }));
                if method == "upi" && b["vpa"].is_null() && b["upiIntent"] == true {
                    v["data"] = json!({ "instrumentResponse": { "intentUrl": format!("upi://pay?pa=pp@sandbox&tn={attempt}") } });
                } else if method == "upi" && b["vpa"].is_string() {
                    v["action"] = json!({ "type": "upi_collect", "vpa": b["vpa"] });
                }
                Ok::<_, (StatusCode, Json<Value>)>(Json(v))
            }),
        )
        .route(
            "/pg/v1/status/{mid}/{txn}",
            get(|State(c): State<Shared>, Path((_m, txn)): Path<(String, String)>| async move {
                c.lock()
                    .unwrap()
                    .pays
                    .get(&txn)
                    .map(|p| Json(pay_json(p, json!({ "type": "none" }))))
                    .ok_or(StatusCode::NOT_FOUND)
            }),
        )
        .route(
            "/pg/v1/refund",
            post(|State(c): State<Shared>, Json(b): Json<Value>| async move {
                let r = c
                    .lock()
                    .unwrap()
                    .refund(
                        b["merchantRefundId"].as_str().unwrap_or_default(),
                        b["originalTransactionId"].as_str().unwrap_or_default(),
                        b["amount"].as_i64().unwrap_or(0),
                    )
                    .map_err(|(s, m)| no(s, m))?;
                Ok::<_, (StatusCode, Json<Value>)>(Json(json!({ "status": "processed", "provider_refund_ref": r.rf_ref })))
            }),
        )
        .route(
            "/pg/v1/refund/{mid}/{rid}",
            get(|State(c): State<Shared>, Path((_m, rid)): Path<(String, String)>| async move {
                c.lock()
                    .unwrap()
                    .refunds
                    .get(&rid)
                    .map(|r| Json(json!({ "status": "processed", "provider_refund_ref": r.rf_ref })))
                    .ok_or(StatusCode::NOT_FOUND)
            }),
        )
        .route(
            "/_sim/pay",
            post(|s: State<Shared>, b: Json<Value>| sim_pay(s, b, PP_WEBHOOK_SECRET, "x-verify")),
        )
        .route(
            "/_sim/refund_event",
            post(|s: State<Shared>, b: Json<Value>| sim_refund_event(s, b, PP_WEBHOOK_SECRET, "x-verify")),
        )
        .with_state(core)
}

/// Serve a router on an ephemeral port.
pub async fn serve(app: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.ok() });
    url
}
