// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! PayU (India) adapter: hosted checkout, Verify Payment, refunds, webhooks and settlement reports.
//!
//! BUILT ONLY FROM PayU'S PUBLIC DOCUMENTATION (docs.payu.in, read November 2026) and tested against
//! `zyvor-provider-mock` (a simulator we wrote from the same pages). It has NOT been exercised against PayU's
//! sandbox or a live account. Every place where the documentation is ambiguous or silent is listed in
//! `docs/zyvor/PAYU.md` under "Needs live-sandbox verification", and the conservative reading is what is coded.
//!
//! What it does, and where the documentation says so:
//! * **Pay** is PayU's *hosted checkout*: the payer's browser POSTs a form (with a SHA-512 hash computed here on
//!   the server) to `/_payment`. `initiate` therefore makes **no network call**: it returns the form for the
//!   client to submit, the payment stays `Pending`, and card data never touches Zyvor. Because nothing is sent,
//!   `initiate` can never leave an unknown outcome behind.
//! * **Look up** a payment with `verify_payment` (`sha512(key|command|var1|salt)`, `var1` = our transaction id).
//! * **Refund** with `cancel_refund_transaction` (`var1` PayU id, `var2` our unique token, `var3` amount).
//!   Refunds are asynchronous at PayU, so they start `Pending` and are followed with `check_action_status`.
//! * **Webhooks**: the payment success / failure callback is form-encoded and carries a *reverse hash*
//!   (`sha512(SALT|status||||||udf5|...|udf1|email|firstname|productinfo|amount|txnid|key)`). PayU's refund
//!   callback is JSON and its page documents **no signature**, so this adapter refuses to act on it (see
//!   `verify_webhook`): refund state is read back with `check_action_status`.
//! * **Settlements**: the *Settlement Detail Range* API (HMAC `Authorization` header built from the merchant
//!   secret), reduced to the same item list every other adapter returns.
//!
//! Ids: PayU limits our transaction id to 25 characters and the refund token to 23, and our attempt and refund
//! ids are longer, so both are derived deterministically ([`txnid_for`], [`refund_token_for`]). The derivation is
//! what lets a lost response be looked up again.

use std::{collections::BTreeMap, time::Duration};

use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha512};
use zyvor_core::{Currency, decimal_to_minor, minor_to_decimal};

use crate::*;

/// Where each family of PayU calls goes. The documentation uses a different host for each, per environment.
#[derive(Debug, Clone)]
struct Hosts {
    /// Browser form POST (`/_payment`).
    payment: String,
    /// Server calls (`/merchant/postservice.php?form=2`).
    postservice: String,
    /// Settlement reports (`/settlement/range`).
    settlement: String,
}

pub struct PayU {
    hosts: Hosts,
    key: String,
    salt: String,
    /// Only the settlement report API wants it (a different secret from the salt). Without it settlements are
    /// reported as unsupported rather than guessed.
    merchant_secret: Option<String>,
    http: reqwest::Client,
}

impl PayU {
    /// One base URL for every call: a mock or a proxy.
    pub fn new(
        base_url: impl Into<String>,
        key: impl Into<String>,
        salt: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        let base = base_url.into().trim_end_matches('/').to_string();
        Self::build(
            Hosts {
                payment: base.clone(),
                postservice: base.clone(),
                settlement: base,
            },
            key.into(),
            salt.into(),
            timeout,
        )
    }

    /// PayU's own hosts as the documentation lists them: `live` selects production, otherwise the test hosts.
    pub fn hosted(
        live: bool,
        key: impl Into<String>,
        salt: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        let hosts = if live {
            Hosts {
                payment: "https://secure.payu.in".into(),
                postservice: "https://info.payu.in".into(),
                settlement: "https://info.payu.in".into(),
            }
        } else {
            Hosts {
                payment: "https://test.payu.in".into(),
                postservice: "https://test.payu.in".into(),
                settlement: "https://apitest.payu.in".into(),
            }
        };
        Self::build(hosts, key.into(), salt.into(), timeout)
    }

    fn build(hosts: Hosts, key: String, salt: String, timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client");
        Self {
            hosts,
            key,
            salt,
            merchant_secret: None,
            http,
        }
    }

    /// The secret the settlement report API signs with (from the dashboard; not the salt).
    pub fn with_merchant_secret(mut self, secret: impl Into<String>) -> Self {
        let s: String = secret.into();
        self.merchant_secret = (!s.is_empty()).then_some(s);
        self
    }

    /// A server call: `command` with `var1`, hashed as `sha512(key|command|var1|salt)`.
    async fn command(
        &self,
        command: &str,
        var1: &str,
        extra: &[(&str, &str)],
    ) -> Result<Value, ProviderError> {
        let hash = sha512_hex(&[&self.key, command, var1, &self.salt]);
        let body = {
            let mut form = form_urlencoded::Serializer::new(String::new());
            form.append_pair("key", &self.key)
                .append_pair("command", command)
                .append_pair("var1", var1)
                .append_pair("hash", &hash);
            for (k, v) in extra {
                form.append_pair(k, v);
            }
            form.finish()
        };
        let res = self
            .http
            .post(format!(
                "{}/merchant/postservice.php?form=2",
                self.hosts.postservice
            ))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(classify)?;
        read_json(res).await
    }
}

/// `sha512` of the parts joined with `|`, as lower-case hex (PayU's own samples lower-case it).
fn sha512_hex(parts: &[&str]) -> String {
    hex::encode(Sha512::digest(parts.join("|").as_bytes()))
}

fn short_hex(prefix: &str, id: &str, len: usize) -> String {
    let h = hex::encode(Sha512::digest(
        format!("zyvor-payu:{prefix}:{id}").as_bytes(),
    ));
    format!("{prefix}{}", &h[..len - prefix.len()])
}

/// PayU's transaction id: at most 25 characters, unique, never reused. Derived from our attempt id, so the same
/// attempt always maps to the same PayU transaction and a lookup needs nothing stored.
pub fn txnid_for(attempt_id: &str) -> String {
    short_hex("zy", attempt_id, 25)
}

/// PayU's refund token (`var2`): at most 23 characters, unique per refund request. Derived from our refund id.
pub fn refund_token_for(refund_id: &str) -> String {
    short_hex("zr", refund_id, 23)
}

/// The hash for the hosted-checkout form: `key|txnid|amount|productinfo|firstname|email|udf1..udf5||||||SALT`.
/// `core` is `[key, txnid, amount, productinfo, firstname, email]`; the udf fields we send are all empty.
fn request_hash(core: [&str; 6], udf: [&str; 5], salt: &str) -> String {
    let mut parts: Vec<&str> = core.to_vec();
    parts.extend(udf);
    parts.extend(["", "", "", "", "", salt]);
    sha512_hex(&parts)
}

/// The hash PayU sends back: `[additional_charges|]SALT|status||||||udf5..udf1|email|firstname|productinfo|amount|txnid|key`.
fn response_hash(salt: &str, f: &BTreeMap<String, String>, key: &str) -> String {
    let g = |k: &str| f.get(k).map(String::as_str).unwrap_or("");
    let mut parts: Vec<&str> = Vec::new();
    // The documentation writes the field `additional_charges` in the formula and `additionalCharges` in the
    // payload list; the payload spelling is the one we can see, so that is what is read. Listed for verification.
    let extra = g("additionalCharges");
    if !extra.is_empty() {
        parts.push(extra);
    }
    parts.extend([
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
        key,
    ]);
    sha512_hex(&parts)
}

/// PayU's payment states, mapped as its "Payment State Explanations" table says (`unmappedstatus` column).
fn status_of(unmapped: &str) -> Option<PaymentStatus> {
    match unmapped.to_ascii_lowercase().as_str() {
        "captured" => Some(PaymentStatus::Captured),
        "auth" => Some(PaymentStatus::Authorized),
        "usercancelled" | "bounced" | "dropped" | "failed" | "autorefund" => {
            Some(PaymentStatus::Failed)
        }
        "initiated" | "in progress" | "pending" => Some(PaymentStatus::Pending),
        _ => None,
    }
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Decimal rupees as PayU writes them (a string, or sometimes a number) into paise, without floats.
fn minor_of(v: &Value) -> Option<i64> {
    let t = text(v)?;
    let (neg, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.as_str()),
    };
    // PayU writes some amounts with more decimals than paise ("0.00000"); extra zeros are harmless, extra
    // non-zero digits would be a fraction of a paisa and are refused.
    let digits = match digits.split_once('.') {
        Some((w, f)) if f.len() > 2 && f[2..].bytes().all(|b| b == b'0') => {
            format!("{w}.{}", &f[..2])
        }
        _ => digits.to_string(),
    };
    let m = decimal_to_minor(&digits)?;
    Some(if neg { -m } else { m })
}

/// Refund states as `check_action_status` words them. The documentation lists both an upper-case set and a
/// lower-case one, so this compares case-insensitively and treats anything unrecognised as unknown.
fn refund_status_of(s: &str) -> Option<RefundStatus> {
    match s.to_ascii_lowercase().as_str() {
        "success" => Some(RefundStatus::Processed),
        "failure" | "failed" => Some(RefundStatus::Failed),
        "queued" | "in progress" | "requested" | "od_hit" | "pending" | "processing" => {
            Some(RefundStatus::Pending)
        }
        _ => None,
    }
}

#[async_trait]
impl Provider for PayU {
    fn name(&self) -> &'static str {
        "payu"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            // The hosted page offers whatever the merchant has enabled; nothing is sent to pick a method.
            methods: vec![
                Method::Upi,
                Method::Card,
                Method::Netbanking,
                Method::Wallet,
            ],
            currencies: vec![Currency::Inr],
            // Documented: a captured payment may be refunded in part (the amount must be below the original).
            partial_refund: true,
            // Pre-authorisation is a separate S2S product in PayU's docs; this adapter does not use it.
            manual_capture: false,
            // UPI collect from the server is an S2S flow that needs enabling by PayU; not built here.
            server_initiated_upi: false,
            upi_intent: false,
            mandates: false,
            // The documentation gives no refund window.
            refund_window_days: None,
        }
    }

    async fn initiate(&self, req: &CreatePayment) -> Result<Initiated, ProviderError> {
        check_request(&self.capabilities(), req)?;
        // PayU makes `surl` and `furl` mandatory: it sends the payer back there. Without one we would be
        // inventing a place to send a payer, so the request is refused (definitely not started).
        let back = req
            .return_url
            .as_deref()
            .filter(|u| !u.is_empty())
            .ok_or_else(|| {
                ProviderError::Rejected("PayU needs a return_url (its surl and furl)".into())
            })?;
        let txnid = txnid_for(&req.attempt_id);
        let amount = minor_to_decimal(req.amount_minor, req.currency);
        // PayU wants a name, an email and a phone number. When we do not have them, neutral placeholders go
        // in, the same way the other hosted adapters do; the hash covers exactly what is sent.
        let (productinfo, firstname, email) = ("Payment", "Customer", "payer@zyvor.invalid");
        let phone = req
            .customer_phone
            .clone()
            .unwrap_or_else(|| "9999999999".into());
        let hash = request_hash(
            [&self.key, &txnid, &amount, productinfo, firstname, email],
            [""; 5],
            &self.salt,
        );
        // The salt is used above and goes no further: only the hash is sent to the payer's browser.
        let fields = json!({
            "key": self.key, "txnid": txnid, "amount": amount, "productinfo": productinfo,
            "firstname": firstname, "email": email, "phone": phone,
            "surl": back, "furl": back, "hash": hash,
        });
        Ok(Initiated {
            state: PaymentState {
                status: PaymentStatus::Pending,
                provider_order_ref: Some(txnid),
                provider_payment_ref: None,
                failure_reason: None,
                raw: fields.clone(),
            },
            action: ClientAction::HostedCheckout {
                params: json!({
                    "provider": "payu", "method": "POST",
                    "action_url": format!("{}/_payment", self.hosts.payment),
                    "fields": fields,
                }),
            },
        })
    }

    async fn capture(
        &self,
        _attempt_id: &str,
        _provider_payment_ref: &str,
        _amount_minor: i64,
        _currency: Currency,
    ) -> Result<PaymentState, ProviderError> {
        Err(ProviderError::Unsupported(
            "PayU manual capture (pre-authorisation) is not built; payments capture automatically"
                .into(),
        ))
    }

    async fn fetch_payment(&self, attempt_id: &str) -> Result<PaymentState, ProviderError> {
        let txnid = txnid_for(attempt_id);
        let v = self.command("verify_payment", &txnid, &[]).await?;
        let d = &v["transaction_details"][&txnid];
        // PayU answers "Not Found" until the payer has actually reached its page. For a hosted checkout that
        // is the normal state of a payment nobody has started yet, so it is NOT a failure: reading it as one
        // could fail a payment the payer is about to make. It stays pending, and the platform's own expiry
        // closes abandoned attempts.
        if d["mihpayid"] == "Not Found" || d["status"] == "Not Found" {
            return Ok(PaymentState {
                status: PaymentStatus::Pending,
                provider_order_ref: Some(txnid),
                provider_payment_ref: None,
                failure_reason: None,
                raw: d.clone(),
            });
        }
        if !d.is_object() || v["status"] != 1 {
            return Err(ProviderError::Uncertain(format!(
                "PayU verify_payment gave no usable answer: {}",
                v["msg"].as_str().unwrap_or("no message")
            )));
        }
        // Only an explicit `captured` counts as paid. A bare `status: success` without the detailed state is
        // not enough to say money was captured (it also covers `auth`), so it is an unknown outcome.
        let status = d["unmappedstatus"]
            .as_str()
            .and_then(status_of)
            .or_else(|| match d["status"].as_str() {
                Some("failure") => Some(PaymentStatus::Failed),
                Some("pending") => Some(PaymentStatus::Pending),
                _ => None,
            })
            .ok_or_else(|| {
                ProviderError::Uncertain(format!(
                    "PayU returned an unrecognised state {:?}/{:?}",
                    d["status"], d["unmappedstatus"]
                ))
            })?;
        Ok(PaymentState {
            failure_reason: (status == PaymentStatus::Failed)
                .then(|| text(&d["error_Message"]).or_else(|| text(&d["error_code"])))
                .flatten(),
            status,
            provider_order_ref: Some(txnid),
            provider_payment_ref: text(&d["mihpayid"]),
            raw: d.clone(),
        })
    }

    async fn refund(&self, req: &RefundRequest) -> Result<RefundState, ProviderError> {
        if req.amount_minor <= 0 {
            return Err(ProviderError::Rejected(
                "refund amount must be positive".into(),
            ));
        }
        if req.currency != Currency::Inr {
            return Err(ProviderError::Unsupported(format!(
                "payu refunds are INR only, not {}",
                req.currency
            )));
        }
        let payu_id = req.provider_payment_ref.trim();
        if payu_id.is_empty() {
            return Err(ProviderError::Rejected(
                "a PayU refund needs the PayU payment id (mihpayid)".into(),
            ));
        }
        let token = refund_token_for(&req.refund_id);
        // The amount is sent as decimal rupees. The documentation's own example does so ("0.10") while its API
        // schema types the field as an integer; the decimal reading is the one coded. Listed for verification.
        let amount = minor_to_decimal(req.amount_minor, req.currency);
        let v = self
            .command(
                "cancel_refund_transaction",
                payu_id,
                &[("var2", &token), ("var3", &amount)],
            )
            .await?;
        if v["status"] == 1 {
            return Ok(RefundState {
                status: RefundStatus::Pending,
                provider_refund_ref: text(&v["request_id"]),
                failure_reason: None,
                raw: v,
            });
        }
        let msg = v["msg"].as_str().unwrap_or("refund request refused");
        // The same refund id sent twice reuses its token. PayU then says the token is used or pending: that
        // means the first request exists, so report what it did rather than failing the retry.
        if msg.to_ascii_lowercase().contains("token already used") {
            return self
                .fetch_refund(&req.attempt_id, &req.refund_id, payu_id)
                .await;
        }
        Err(ProviderError::Rejected(format!(
            "PayU refused the refund: {msg}"
        )))
    }

    async fn fetch_refund(
        &self,
        _attempt_id: &str,
        refund_id: &str,
        provider_payment_ref: &str,
    ) -> Result<RefundState, ProviderError> {
        let payu_id = provider_payment_ref.trim();
        if payu_id.is_empty() {
            return Err(ProviderError::Rejected(
                "looking up a PayU refund needs the PayU payment id".into(),
            ));
        }
        let token = refund_token_for(refund_id);
        let v = self
            .command("check_action_status", payu_id, &[("var2", "payuid")])
            .await?;
        // `transaction_details[<payu id>]` maps each request id to one action; the schema in the same page
        // also shows a flat object, so both are read.
        let by_payment = &v["transaction_details"][payu_id];
        let actions: Vec<&Value> = match by_payment {
            Value::Object(m) if m.contains_key("action") => vec![by_payment],
            Value::Object(m) => m.values().collect(),
            _ => vec![],
        };
        let found = actions
            .into_iter()
            .find(|a| a["action"] == "refund" && a["token"].as_str() == Some(token.as_str()));
        let Some(a) = found else {
            // Not listed. Unlike a payment, a refund is not safe to call "never happened": a queued refund
            // may simply not be visible yet, and failing it would invite a second refund of the same money.
            return Err(ProviderError::Uncertain(
                "PayU does not list this refund (yet); not treated as failed".into(),
            ));
        };
        let status = a["status"]
            .as_str()
            .and_then(refund_status_of)
            .ok_or_else(|| {
                ProviderError::Uncertain(format!(
                    "PayU refund state {:?} is not recognised",
                    a["status"]
                ))
            })?;
        Ok(RefundState {
            failure_reason: (status == RefundStatus::Failed).then(|| {
                text(&a["error_Message"]).unwrap_or_else(|| "refund failed at PayU".into())
            }),
            status,
            provider_refund_ref: text(&a["request_id"]),
            raw: a.clone(),
        })
    }

    async fn fetch_settlements(
        &self,
        on: chrono::NaiveDate,
    ) -> Result<Vec<SettlementRecord>, ProviderError> {
        let Some(secret) = &self.merchant_secret else {
            return Err(ProviderError::Unsupported(
                "PayU settlement reports need the merchant secret (set `merchant_secret` in the credentials)".into(),
            ));
        };
        let mut out = Vec::new();
        for page in 1..=50u32 {
            // The report API is signed over the request body (empty for a GET), the Date header and the secret.
            let date = chrono::Utc::now()
                .format("%a, %d %b %Y %H:%M:%S GMT")
                .to_string();
            let sig = sha512_hex(&["", &date, secret]);
            let res = self
                .http
                .get(format!(
                    "{}/settlement/range?dateFrom={on}&dateTo={on}&page={page}&pageSize=100",
                    self.hosts.settlement
                ))
                .header("Date", &date)
                .header(
                    "Authorization",
                    format!(
                        "hmac username=\"{}\", algorithm=\"sha512\", headers=\"date\", signature=\"{sig}\"",
                        self.key
                    ),
                )
                .send()
                .await
                .map_err(classify)?;
            let status = res.status();
            let v: Value = res.json().await.unwrap_or(Value::Null);
            if v["message"] == "No data found" {
                break;
            }
            if status.is_server_error() {
                return Err(ProviderError::Uncertain(format!("HTTP {status}")));
            }
            if !status.is_success() || v["status"] != 0 {
                return Err(ProviderError::Rejected(format!(
                    "PayU settlement report refused: {}",
                    v["message"].as_str().unwrap_or("no message")
                )));
            }
            let rows = v["result"]["data"].as_array().cloned().unwrap_or_default();
            let total = v["result"]["totalCount"].as_u64().unwrap_or(0) as usize;
            let empty = rows.is_empty();
            for row in &rows {
                out.push(settlement_record(row, on)?);
            }
            if empty || out.len() >= total {
                break;
            }
        }
        Ok(out)
    }

    /// Payment callbacks are form-encoded and carry PayU's reverse hash. The hash is keyed by the salt, so the
    /// stored webhook secret is not used. PayU sends no timestamp and no event id: replays are caught by the
    /// event id built here from the *signed* fields, `payu:{txnid}:{status}`.
    ///
    /// PayU's refund and dispute callbacks are JSON and the documentation describes no signature for them, so
    /// they are refused as unsigned. Refund state is read with `check_action_status` instead.
    fn verify_webhook(
        &self,
        _secret: &[u8],
        _headers: &Headers,
        body: &[u8],
        _now_unix: i64,
    ) -> Result<WebhookEvent, WebhookError> {
        if body.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
            return Err(WebhookError::MissingSignature);
        }
        let f: BTreeMap<String, String> = form_urlencoded::parse(body)
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let given = f.get("hash").ok_or(WebhookError::MissingSignature)?;
        if f.get("key").map(String::as_str) != Some(self.key.as_str()) {
            return Err(WebhookError::BadSignature);
        }
        let want = response_hash(&self.salt, &f, &self.key);
        if !ct_eq(want.as_bytes(), given.to_ascii_lowercase().as_bytes()) {
            return Err(WebhookError::BadSignature);
        }
        let g = |k: &str| f.get(k).map(|s| s.trim()).filter(|s| !s.is_empty());
        let txnid = g("txnid").ok_or_else(|| WebhookError::Malformed("missing txnid".into()))?;
        let status = g("status").unwrap_or_default();
        // Only the signed `status` decides the outcome. `unmappedstatus` is not covered by the hash, so it can
        // only make us do less: an `auth` (authorised, not captured) is left for a lookup to settle.
        let kind = match (status, g("unmappedstatus")) {
            ("success", Some(u)) if u.eq_ignore_ascii_case("auth") => EventKind::Other,
            ("success", _) => EventKind::PaymentCaptured,
            ("failure", _) => EventKind::PaymentFailed,
            _ => EventKind::Other,
        };
        let amount_minor = Some(
            decimal_to_minor(g("amount").unwrap_or_default())
                .ok_or_else(|| WebhookError::Malformed("amount is not a decimal".into()))?,
        );
        let raw = Value::Object(f.iter().map(|(k, v)| (k.clone(), json!(v))).collect());
        Ok(WebhookEvent {
            event_id: format!("payu:{txnid}:{status}"),
            kind,
            order_ref: Some(txnid.to_string()),
            provider_payment_ref: g("mihpayid").map(str::to_string),
            provider_refund_ref: None,
            refund_ref: None,
            amount_minor,
            failure_reason: (kind == EventKind::PaymentFailed)
                .then(|| {
                    g("error_Message")
                        .or_else(|| g("error"))
                        .map(str::to_string)
                })
                .flatten(),
            raw,
        })
    }
}

/// One settlement of PayU's range report as a record. `settlementAmount` is PayU's own net, kept as it is; the
/// items are rebuilt from the transactions, and anything PayU's net includes that the rows do not explain
/// becomes one labelled adjustment, so the items always add up to the net (the contract every adapter keeps).
fn settlement_record(
    row: &Value,
    on: chrono::NaiveDate,
) -> Result<SettlementRecord, ProviderError> {
    let bad = |what: &str| ProviderError::Uncertain(format!("PayU settlement report: {what}"));
    let net = minor_of(&row["settlementAmount"])
        .ok_or_else(|| bad("settlementAmount is not a decimal"))?;
    let mut items = Vec::new();
    for t in row["transaction"].as_array().cloned().unwrap_or_default() {
        let action = t["action"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let id = text(&t["payuId"]);
        let amt = minor_of(&t["transactionAmount"])
            .ok_or_else(|| bad("transactionAmount is not a decimal"))?;
        match action.as_str() {
            "capture" => {
                items.push(SettlementItem {
                    kind: SettlementItemKind::Payment,
                    provider_ref: id.clone(),
                    amount_minor: amt,
                    description: "payment".into(),
                });
                // PayU's own net for the row minus the gross is the fee and tax it took.
                if let Some(row_net) = minor_of(&t["merchantNetAmount"])
                    && row_net != amt
                {
                    items.push(SettlementItem {
                        kind: SettlementItemKind::Fee,
                        provider_ref: id,
                        amount_minor: row_net - amt,
                        description: "service fee and tax".into(),
                    });
                }
            }
            "refund" => items.push(SettlementItem {
                kind: SettlementItemKind::Refund,
                provider_ref: id,
                amount_minor: -amt.abs(),
                description: "refund".into(),
            }),
            other => {
                // Chargebacks and debits take money; their reversals and credits give it back.
                let takes = other.contains("debit")
                    || (other.contains("chargeback") && !other.contains("reversal"));
                items.push(SettlementItem {
                    kind: SettlementItemKind::Adjustment,
                    provider_ref: id,
                    amount_minor: if takes { -amt.abs() } else { amt.abs() },
                    description: other.to_string(),
                });
            }
        }
    }
    let explained: i64 = items.iter().map(|i| i.amount_minor).sum();
    if explained != net {
        items.push(SettlementItem {
            kind: SettlementItemKind::Adjustment,
            provider_ref: None,
            amount_minor: net - explained,
            description: "difference between PayU's settlement amount and its listed transactions"
                .into(),
        });
    }
    let settled_on = row["settlementCompletedDate"]
        .as_str()
        .and_then(|s| s.get(..10))
        .and_then(|d| d.parse().ok())
        .unwrap_or(on);
    Ok(SettlementRecord {
        provider_ref: text(&row["settlementId"]).ok_or_else(|| bad("settlementId is missing"))?,
        utr: text(&row["utrNumber"]),
        currency: Currency::Inr,
        settled_on,
        net_minor: net,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payu() -> PayU {
        PayU::new(
            "http://127.0.0.1:9",
            "TESTKEY",
            "TESTSALT",
            Duration::from_secs(1),
        )
    }

    #[test]
    fn the_request_hash_follows_the_documented_sequence() {
        // key|txnid|amount|productinfo|firstname|email|udf1|udf2|udf3|udf4|udf5||||||salt, hashed with SHA-512
        let want = hex::encode(Sha512::digest(b"K|T1|10.00|Pay|Ann|a@b.c|||||||||||S"));
        let got = request_hash(["K", "T1", "10.00", "Pay", "Ann", "a@b.c"], [""; 5], "S");
        assert_eq!(got, want);
        assert_eq!(got.len(), 128);
    }

    #[test]
    fn the_response_hash_is_the_documented_reverse_sequence() {
        let f: BTreeMap<String, String> = [
            ("status", "success"),
            ("email", "a@b.c"),
            ("firstname", "Ann"),
            ("productinfo", "Pay"),
            ("amount", "10.00"),
            ("txnid", "T1"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .into();
        // SALT|status||||||udf5|udf4|udf3|udf2|udf1|email|firstname|productinfo|amount|txnid|key
        let want = hex::encode(Sha512::digest(
            b"S|success|||||||||||a@b.c|Ann|Pay|10.00|T1|K",
        ));
        assert_eq!(response_hash("S", &f, "K"), want);
        // with additionalCharges, that value leads
        let mut g = f.clone();
        g.insert("additionalCharges".into(), "2.00".into());
        let want = hex::encode(Sha512::digest(
            b"2.00|S|success|||||||||||a@b.c|Ann|Pay|10.00|T1|K",
        ));
        assert_eq!(response_hash("S", &g, "K"), want);
    }

    #[test]
    fn derived_ids_fit_payus_limits_and_are_stable() {
        let long = "att_01928f0a1b2c7d3e8f4a5b6c7d8e9f01";
        assert_eq!(txnid_for(long).len(), 25);
        assert_eq!(txnid_for(long), txnid_for(long));
        assert_ne!(txnid_for(long), txnid_for("att_other"));
        assert_eq!(
            refund_token_for("rfd_01928f0a1b2c7d3e8f4a5b6c7d8e9f01").len(),
            23
        );
        assert!(txnid_for(long).bytes().all(|b| b.is_ascii_alphanumeric()));
    }

    #[test]
    fn state_mapping_follows_the_published_table() {
        assert_eq!(status_of("captured"), Some(PaymentStatus::Captured));
        assert_eq!(status_of("auth"), Some(PaymentStatus::Authorized));
        for f in [
            "userCancelled",
            "bounced",
            "dropped",
            "failed",
            "autoRefund",
        ] {
            assert_eq!(status_of(f), Some(PaymentStatus::Failed), "{f}");
        }
        for p in ["initiated", "in progress", "pending"] {
            assert_eq!(status_of(p), Some(PaymentStatus::Pending), "{p}");
        }
        assert_eq!(
            status_of("something new"),
            None,
            "unknown is unknown, not a failure"
        );
    }

    #[test]
    fn an_unsigned_refund_callback_is_refused_not_trusted() {
        let body = br#"{"action":"refund","status":"success","token":"zr1","amt":"10.00","key":"TESTKEY"}"#;
        assert_eq!(
            payu()
                .verify_webhook(b"", &Headers::new(), body, 0)
                .unwrap_err(),
            WebhookError::MissingSignature
        );
    }

    #[test]
    fn settlement_rows_always_add_up_to_payus_net() {
        let row = json!({
            "settlementId": "S1", "utrNumber": "UTR1", "settlementCompletedDate": "2026-08-26 02:51:22.000000",
            "settlementAmount": "1479.82",
            "transaction": [
                { "action": "capture", "payuId": "P1", "transactionAmount": "2480.0", "merchantNetAmount": "2467.13" },
                { "action": "ADJ_debit", "payuId": "ADJ_1", "transactionAmount": "987.31" },
            ],
        });
        let r = settlement_record(&row, "2026-08-26".parse().unwrap()).unwrap();
        assert_eq!(r.net_minor, 147_982);
        assert_eq!(
            r.items.iter().map(|i| i.amount_minor).sum::<i64>(),
            r.net_minor
        );
        assert!(
            r.items
                .iter()
                .any(|i| i.kind == SettlementItemKind::Fee && i.amount_minor == -1_287)
        );
        // a row PayU's net includes but the list does not explain becomes one labelled adjustment
        let odd = json!({ "settlementId": "S2", "settlementAmount": "10.00", "transaction": [] });
        let r = settlement_record(&odd, "2026-08-26".parse().unwrap()).unwrap();
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].amount_minor, 1_000);
    }
}
