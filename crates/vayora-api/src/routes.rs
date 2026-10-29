use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use vayora_core::{Money, new_id};
use vayora_ledger::{Entry, Side, Transaction, accounts};
use vayora_upi::{CollectOutcome, CollectRequest, RefundRequest, is_valid_vpa};

use crate::{
    AppState,
    error::ApiError,
    idempotency::{self, Begin, fingerprint},
};

#[derive(Debug, Serialize, FromRow)]
pub struct Order {
    pub id: String,
    pub amount_paise: i64,
    pub currency: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct Payment {
    pub id: String,
    pub order_id: String,
    pub vpa: String,
    pub amount_paise: i64,
    pub refunded_paise: i64,
    pub status: String,
    pub upi_ref: Option<String>,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct Refund {
    pub id: String,
    pub payment_id: String,
    pub amount_paise: i64,
    pub status: String,
    pub upi_ref: String,
    pub created_at: DateTime<Utc>,
}

fn money(paise: i64) -> Money {
    Money::from_paise(paise).expect("DB constraints keep amounts non-negative")
}

// ---------- orders ----------

#[derive(Deserialize)]
pub struct CreateOrder {
    amount_paise: i64,
}

pub async fn create_order(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<CreateOrder>,
) -> Result<Response, ApiError> {
    if b.amount_paise <= 0 {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_paise must be a positive integer",
        ));
    }
    let mut tx = s.pool.begin().await?;
    let key = match idempotency::begin(
        &mut tx,
        "orders",
        &headers,
        &fingerprint(&[&b.amount_paise.to_string()]),
    )
    .await?
    {
        Begin::Replay(r) => return Ok(r),
        Begin::Proceed(k) => k,
    };
    let order: Order = sqlx::query_as(
        "INSERT INTO orders (id, amount_paise, status) VALUES ($1, $2, 'created') RETURNING *",
    )
    .bind(new_id("order"))
    .bind(b.amount_paise)
    .fetch_one(&mut *tx)
    .await?;
    let res = idempotency::finish(&mut tx, key, StatusCode::CREATED, &order).await?;
    tx.commit().await?;
    Ok(res)
}

pub async fn get_order(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Order>, ApiError> {
    sqlx::query_as("SELECT * FROM orders WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.pool)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("order"))
}

// ---------- payments ----------

#[derive(Deserialize)]
pub struct CreatePayment {
    order_id: String,
    vpa: String,
}

pub async fn create_payment(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<CreatePayment>,
) -> Result<Response, ApiError> {
    if !is_valid_vpa(&b.vpa) {
        return Err(ApiError::invalid(
            "invalid_vpa",
            "vpa must look like name@psp",
        ));
    }
    let mut tx = s.pool.begin().await?;
    let key = match idempotency::begin(
        &mut tx,
        "payments",
        &headers,
        &fingerprint(&[&b.order_id, &b.vpa]),
    )
    .await?
    {
        Begin::Replay(r) => return Ok(r),
        Begin::Proceed(k) => k,
    };

    // Serialise payment attempts per order.
    let order: Order = sqlx::query_as("SELECT * FROM orders WHERE id = $1 FOR UPDATE")
        .bind(&b.order_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found("order"))?;
    if order.status == "paid" {
        return Err(ApiError::conflict(
            "order_already_paid",
            "this order has already been paid",
        ));
    }

    let payment_id = new_id("pay");
    // NOTE: the simulator answers instantly, so we call it inside the DB
    // transaction. A real async PSP needs a pending state + callback.
    let outcome = s
        .upi
        .collect(CollectRequest {
            payment_id: payment_id.clone(),
            vpa: b.vpa.clone(),
            amount: money(order.amount_paise),
        })
        .await?;

    let (status, upi_ref, failure) = match outcome {
        CollectOutcome::Success { upi_ref } => ("captured", Some(upi_ref), None),
        CollectOutcome::Failed { reason } => ("failed", None, Some(reason)),
    };
    let payment: Payment = sqlx::query_as(
        "INSERT INTO payments (id, order_id, vpa, amount_paise, status, upi_ref, failure_reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *",
    )
    .bind(&payment_id)
    .bind(&order.id)
    .bind(&b.vpa)
    .bind(order.amount_paise)
    .bind(status)
    .bind(&upi_ref)
    .bind(&failure)
    .fetch_one(&mut *tx)
    .await?;

    if status == "captured" {
        sqlx::query("UPDATE orders SET status = 'paid' WHERE id = $1")
            .bind(&order.id)
            .execute(&mut *tx)
            .await?;
        // Money received from UPI is owed to the merchant.
        Transaction::new(vec![
            Entry {
                account: accounts::UPI_CLEARING.into(),
                side: Side::Debit,
                amount: money(order.amount_paise),
            },
            Entry {
                account: accounts::MERCHANT_PAYABLE.into(),
                side: Side::Credit,
                amount: money(order.amount_paise),
            },
        ])
        .map_err(vayora_ledger::StoreError::from)?
        .post(&mut tx, &format!("payment:{payment_id}:capture"))
        .await?;
    }
    let event = if status == "captured" {
        "payment.captured"
    } else {
        "payment.failed"
    };
    vayora_webhooks::enqueue(&mut tx, event, json!({ "payment": &payment })).await?;

    let res = idempotency::finish(&mut tx, key, StatusCode::CREATED, &payment).await?;
    tx.commit().await?;
    Ok(res)
}

pub async fn get_payment(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Payment>, ApiError> {
    sqlx::query_as("SELECT * FROM payments WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.pool)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("payment"))
}

// ---------- refunds ----------

#[derive(Deserialize, Default)]
pub struct CreateRefund {
    /// Omit to refund whatever remains of the payment.
    amount_paise: Option<i64>,
}

pub async fn create_refund(
    State(s): State<AppState>,
    Path(payment_id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<CreateRefund>>,
) -> Result<Response, ApiError> {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    if matches!(req.amount_paise, Some(a) if a <= 0) {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_paise must be a positive integer",
        ));
    }
    let amount_text = req
        .amount_paise
        .map(|a| a.to_string())
        .unwrap_or_else(|| "full".into());

    let mut tx = s.pool.begin().await?;
    let key = match idempotency::begin(
        &mut tx,
        "refunds",
        &headers,
        &fingerprint(&[&payment_id, &amount_text]),
    )
    .await?
    {
        Begin::Replay(r) => return Ok(r),
        Begin::Proceed(k) => k,
    };

    let payment: Payment = sqlx::query_as("SELECT * FROM payments WHERE id = $1 FOR UPDATE")
        .bind(&payment_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found("payment"))?;
    let upi_ref = match (&payment.status[..], &payment.upi_ref) {
        ("captured", Some(r)) => r.clone(),
        _ => {
            return Err(ApiError::conflict(
                "payment_not_captured",
                "only captured payments can be refunded",
            ));
        }
    };
    let remaining = payment.amount_paise - payment.refunded_paise;
    let amount = req.amount_paise.unwrap_or(remaining);
    if amount <= 0 || amount > remaining {
        return Err(ApiError::invalid(
            "refund_exceeds_payment",
            format!("refundable amount remaining is {remaining} paise"),
        ));
    }

    let refund_upi_ref = s
        .upi
        .refund(RefundRequest {
            payment_id: payment.id.clone(),
            upi_ref,
            amount: money(amount),
        })
        .await?;
    let refund: Refund = sqlx::query_as(
        "INSERT INTO refunds (id, payment_id, amount_paise, status, upi_ref)
         VALUES ($1, $2, $3, 'processed', $4) RETURNING *",
    )
    .bind(new_id("rfnd"))
    .bind(&payment.id)
    .bind(amount)
    .bind(&refund_upi_ref)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("UPDATE payments SET refunded_paise = refunded_paise + $2 WHERE id = $1")
        .bind(&payment.id)
        .bind(amount)
        .execute(&mut *tx)
        .await?;

    // Reverse the capture, for the refunded amount.
    Transaction::new(vec![
        Entry {
            account: accounts::MERCHANT_PAYABLE.into(),
            side: Side::Debit,
            amount: money(amount),
        },
        Entry {
            account: accounts::UPI_CLEARING.into(),
            side: Side::Credit,
            amount: money(amount),
        },
    ])
    .map_err(vayora_ledger::StoreError::from)?
    .post(&mut tx, &format!("refund:{}", refund.id))
    .await?;
    vayora_webhooks::enqueue(&mut tx, "refund.processed", json!({ "refund": &refund })).await?;

    let res = idempotency::finish(&mut tx, key, StatusCode::CREATED, &refund).await?;
    tx.commit().await?;
    Ok(res)
}

// ---------- balance ----------

pub async fn balance(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut conn = s.pool.acquire().await?;
    let (pd, pc) = vayora_ledger::totals(&mut conn, accounts::MERCHANT_PAYABLE).await?;
    let (cd, cc) = vayora_ledger::totals(&mut conn, accounts::UPI_CLEARING).await?;
    Ok(Json(json!({
        "merchant_payable_paise": pc - pd,
        "upi_clearing_paise": cd - cc,
    })))
}
