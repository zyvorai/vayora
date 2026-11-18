//! Soft-decline retry: after a DEFINITIVE provider-side failure, start the next attempt on another provider, so a
//! payer is not sent back to the start because one provider had a bad moment. Opt-in per business
//! (`routing_automation.soft_retry_enabled`), off while the kill switch is on.
//!
//! It never weakens the rule that protects the payer's money: there is no path in this module that creates an
//! attempt while an earlier attempt on the same payment is unresolved. The attempt is created by
//! `payments::start_attempt_with`, the same function every attempt goes through, which under a row lock on the
//! intent refuses a new attempt while any attempt is `created`, `pending`, `authorized`, `unknown` or `captured`.
//! This module only ADDS conditions (`guard`), evaluated under that same lock:
//!
//! - the newest attempt on the payment is the failed one being retried, and it failed with class
//!   `provider_fault` or `provider_rejected` (the provider said "failed", or refused the request before creating
//!   anything). NOT `payer_declined` (the payer's doing), NOT `declined` (cause not recognised: treated as unknown),
//!   NOT `cancelled`, and never an `unknown` outcome (that is not a failure at all until the provider says so);
//! - the payment has fewer than `soft_retry_max_attempts` attempts and its first attempt is no older than
//!   `soft_retry_window_seconds`;
//! - another provider is eligible (the hard filters, the payment's allowed providers and the rules all still apply),
//!   has not been used for this payment, and is not degraded;
//! - the failed attempt has not been decided before. `routing_soft_retries.from_attempt_id` is UNIQUE and the retry
//!   uses the idempotency key `softretry:<failed attempt id>`, so any number of workers, webhooks and requests racing
//!   to retry one failure produce exactly one new attempt.

use axum::http::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::new_id;
use zyvor_providers::{
    Method,
    router::{Breaker, Candidate, Decision},
};

use crate::{
    AppState,
    error::ApiError,
    payments::{CreateAttempt, start_attempt_with},
    routing_auto::Automation,
    tenant,
};

/// Failure classes a soft retry acts on (see the module notes for why only these two).
pub const RETRYABLE: [&str; 2] = ["provider_fault", "provider_rejected"];

/// A skip that is final: nothing in the facts will change, so the decision is recorded and not revisited.
const FINAL: &str = "soft_retry_skipped";
/// Not applicable (switched off, already handled, superseded): nothing is recorded.
const NOT_APPLICABLE: &str = "soft_retry_not_applicable";

pub struct Ctx {
    pub from_attempt_id: String,
}

/// What the guard established under the intent's lock.
pub struct Guard {
    pub from_attempt_id: String,
    pub from_provider: String,
    pub failure_class: String,
    /// Every provider already used for this payment.
    pub tried: Vec<String>,
    /// The number the new attempt will have (the first attempt is 1).
    pub attempt_number: i32,
    pub method: String,
}

impl Guard {
    /// For `route.soft_retry` on the new attempt.
    pub fn describe(&self, to_provider: &str) -> Value {
        json!({
            "from_attempt_id": self.from_attempt_id,
            "from_provider": self.from_provider,
            "failure_class": self.failure_class,
            "to_provider": to_provider,
            "attempt_number": self.attempt_number,
            "text": format!(
                "{} could not take this payment ({}), so it was tried on {} instead. The first attempt had already failed; no second payment was started while it was unresolved.",
                self.from_provider, self.failure_class, to_provider
            ),
        })
    }
}

fn not_applicable(why: &str) -> ApiError {
    ApiError::conflict(NOT_APPLICABLE, why.to_string())
}
fn final_skip(why: impl Into<String>) -> ApiError {
    ApiError::conflict(FINAL, why)
}

/// Every condition that must hold, checked after `start_attempt_with` took the intent's row lock and found no live
/// attempt on it. Errors are never surfaced to a payer: the caller treats them as "no retry".
pub async fn guard(
    conn: &mut PgConnection,
    tenant_id: &str,
    intent_id: &str,
    ctx: &Ctx,
    auto: &Automation,
    method: Method,
) -> Result<Guard, ApiError> {
    if !auto.soft_retry_active() {
        return Err(not_applicable(
            "soft retry is off, or the kill switch is on",
        ));
    }
    let from: Option<(String, Option<String>, String, String, Option<String>, bool)> =
        sqlx::query_as(
            "SELECT status, failure_class, method, provider, mandate_id, retry_input IS NOT NULL
             FROM payment_attempts WHERE id = $1 AND intent_id = $2",
        )
        .bind(&ctx.from_attempt_id)
        .bind(intent_id)
        .fetch_optional(&mut *conn)
        .await?;
    let Some((status, class, from_method, from_provider, mandate, has_input)) = from else {
        return Err(not_applicable("the failed attempt is not on this payment"));
    };
    let class = class.unwrap_or_default();
    if status != "failed" || !RETRYABLE.contains(&class.as_str()) {
        return Err(not_applicable(
            "only a definitive provider-side failure is retried",
        ));
    }
    if from_method != method.as_str() || mandate.is_some() || !has_input {
        return Err(not_applicable("this attempt cannot be repeated elsewhere"));
    }
    let decided: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM routing_soft_retries WHERE from_attempt_id = $1)",
    )
    .bind(&ctx.from_attempt_id)
    .fetch_one(&mut *conn)
    .await?;
    if decided {
        return Err(not_applicable("this failure was already handled"));
    }
    let (count, elapsed, newest): (i64, f64, Option<String>) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(EXTRACT(EPOCH FROM (now() - MIN(created_at))), 0)::float8,
                (SELECT id FROM payment_attempts WHERE intent_id = $1 ORDER BY created_at DESC, id DESC LIMIT 1)
         FROM payment_attempts WHERE intent_id = $1",
    )
    .bind(intent_id)
    .fetch_one(&mut *conn)
    .await?;
    if newest.as_deref() != Some(ctx.from_attempt_id.as_str()) {
        return Err(not_applicable("a newer attempt exists on this payment"));
    }
    if count >= i64::from(auto.soft_retry_max_attempts) {
        return Err(final_skip(format!(
            "the payment already has {count} attempts (limit {})",
            auto.soft_retry_max_attempts
        )));
    }
    if elapsed > f64::from(auto.soft_retry_window_seconds) {
        return Err(final_skip(format!(
            "the payment's first attempt is older than the {} second retry window",
            auto.soft_retry_window_seconds
        )));
    }
    let tried: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT provider FROM payment_attempts WHERE intent_id = $1 ORDER BY provider",
    )
    .bind(intent_id)
    .fetch_all(&mut *conn)
    .await?;
    let _ = tenant_id;
    Ok(Guard {
        from_attempt_id: ctx.from_attempt_id.clone(),
        from_provider,
        failure_class: class,
        tried,
        attempt_number: count as i32 + 1,
        method: from_method,
    })
}

/// After routing: the chosen provider must be a different one that is not degraded. Otherwise there is nothing to
/// retry on (never "try the same provider again": that is a plain retry by the payer, not ours to start).
pub fn check_alternate(
    g: &Guard,
    decision: &Decision,
    candidates: &[Candidate],
) -> Result<(), ApiError> {
    if g.tried.contains(&decision.provider) {
        return Err(final_skip(
            "no other eligible provider can take this payment",
        ));
    }
    let degraded = candidates
        .iter()
        .find(|c| c.provider == decision.provider)
        .and_then(|c| c.signal.as_ref())
        .is_some_and(|s| s.breaker == Breaker::Degraded);
    if degraded {
        return Err(final_skip(
            "the only other providers are degraded by the circuit breaker",
        ));
    }
    Ok(())
}

/// In the new attempt's transaction: record that the failure was retried, drop the stored request of the failed
/// attempt, and leave an audit entry.
pub async fn record_started(
    conn: &mut PgConnection,
    tenant_id: &str,
    intent_id: &str,
    to_attempt_id: &str,
    g: &Guard,
    to_provider: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO routing_soft_retries (id, tenant_id, intent_id, from_attempt_id, to_attempt_id, status, from_provider, to_provider, method, failure_class, attempt_number)
         VALUES ($1, $2, $3, $4, $5, 'started', $6, $7, $8, $9, $10)",
    )
    .bind(new_id("srt"))
    .bind(tenant_id)
    .bind(intent_id)
    .bind(&g.from_attempt_id)
    .bind(to_attempt_id)
    .bind(&g.from_provider)
    .bind(to_provider)
    .bind(&g.method)
    .bind(&g.failure_class)
    .bind(g.attempt_number)
    .execute(&mut *conn)
    .await?;
    sqlx::query("UPDATE payment_attempts SET retry_input = NULL WHERE id = $1")
        .bind(&g.from_attempt_id)
        .execute(&mut *conn)
        .await?;
    tenant::audit(
        conn,
        tenant_id,
        "routing",
        "routing.soft_retry_started",
        Some(to_attempt_id),
        json!({
            "intent_id": intent_id, "from_attempt_id": g.from_attempt_id, "from_provider": g.from_provider,
            "to_provider": to_provider, "failure_class": g.failure_class, "attempt_number": g.attempt_number,
        }),
    )
    .await?;
    Ok(())
}

/// A final "no": recorded once, so the checkout stops saying "trying another way" and nobody asks again.
async fn record_skip(
    s: &AppState,
    tenant_id: &str,
    intent_id: &str,
    from_attempt_id: &str,
    reason: &str,
) -> Result<(), ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let from: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT provider, method, failure_class FROM payment_attempts WHERE id = $1 AND intent_id = $2",
    )
    .bind(from_attempt_id)
    .bind(intent_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((provider, method, class)) = from else {
        return Ok(());
    };
    let n = sqlx::query(
        "INSERT INTO routing_soft_retries (id, tenant_id, intent_id, from_attempt_id, status, reason, from_provider, method, failure_class)
         VALUES ($1, $2, $3, $4, 'skipped', $5, $6, $7, $8) ON CONFLICT (from_attempt_id) DO NOTHING",
    )
    .bind(new_id("srt"))
    .bind(tenant_id)
    .bind(intent_id)
    .bind(from_attempt_id)
    .bind(reason)
    .bind(&provider)
    .bind(&method)
    .bind(class.clone().unwrap_or_default())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 1 {
        sqlx::query("UPDATE payment_attempts SET retry_input = NULL WHERE id = $1")
            .bind(from_attempt_id)
            .execute(&mut *tx)
            .await?;
        tenant::audit(
            &mut tx,
            tenant_id,
            "routing",
            "routing.soft_retry_skipped",
            Some(from_attempt_id),
            json!({ "intent_id": intent_id, "from_provider": provider, "failure_class": class, "reason": reason }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Try to retry the newest failed attempt on a payment, then (if that retry failed definitively too) the next one,
/// until the guards say stop. Idempotent and safe from any number of callers at once. Never returns an error: a
/// retry that cannot be made leaves the payment exactly as it was.
pub async fn run(s: &AppState, tenant_id: &str, intent_id: &str) {
    if let Err(e) = run_inner(s, tenant_id, intent_id).await {
        tracing::warn!(intent = intent_id, error = ?e.message, "soft retry did not run");
    }
}

async fn run_inner(s: &AppState, tenant_id: &str, intent_id: &str) -> Result<(), ApiError> {
    let env: Option<String> = sqlx::query_scalar("SELECT env FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_optional(&s.pool)
        .await?;
    let Some(env) = env else {
        return Ok(());
    };
    // each round retries the newest failed attempt; a retry that fails definitively is itself retried, within the limits
    for _ in 0..5 {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        if !crate::routing_auto::load(&mut tx, tenant_id)
            .await?
            .soft_retry_active()
        {
            return Ok(());
        }
        #[allow(clippy::type_complexity)]
        let newest: Option<(
            String,
            String,
            Option<String>,
            Option<Value>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, status, failure_class, retry_input, mandate_id FROM payment_attempts
                 WHERE intent_id = $1 ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(intent_id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.rollback().await?;
        let Some((from_id, status, class, input, mandate)) = newest else {
            return Ok(());
        };
        let (Some(input), None) = (input, mandate) else {
            return Ok(());
        };
        if status != "failed" || !RETRYABLE.contains(&class.as_deref().unwrap_or_default()) {
            return Ok(());
        }
        let text = |k: &str| input.get(k).and_then(Value::as_str).map(str::to_string);
        let body = CreateAttempt {
            method: String::new(), // filled below from the failed attempt
            vpa: text("vpa"),
            return_url: text("return_url"),
            upi_flow: text("upi_flow"),
            card_token_id: None,
        };
        let method: String = {
            let mut tx = tenant::begin(&s.pool, tenant_id).await?;
            let m = sqlx::query_scalar("SELECT method FROM payment_attempts WHERE id = $1")
                .bind(&from_id)
                .fetch_one(&mut *tx)
                .await?;
            tx.rollback().await?;
            m
        };
        let body = CreateAttempt { method, ..body };
        let mut headers = HeaderMap::new();
        headers.insert(
            "idempotency-key",
            HeaderValue::from_str(&format!("softretry:{from_id}"))
                .map_err(|_| ApiError::internal())?,
        );
        let ctx = Ctx {
            from_attempt_id: from_id.clone(),
        };
        match start_attempt_with(
            s,
            tenant_id,
            &env,
            "soft_retry",
            intent_id,
            &headers,
            body,
            Some(&ctx),
        )
        .await
        {
            // a new attempt exists; if it already failed definitively, give the next round a chance
            Ok((_, Some(_))) => continue,
            Ok(_) => return Ok(()),
            Err(e)
                if e.code == NOT_APPLICABLE
                    || matches!(
                        e.code,
                        "attempt_in_progress" | "payment_outcome_unresolved" | "intent_not_payable"
                    ) =>
            {
                return Ok(());
            }
            Err(e) if e.code == FINAL || e.status.is_client_error() => {
                record_skip(s, tenant_id, intent_id, &from_id, &e.message).await?;
                return Ok(());
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// The worker's backstop: retry failures nobody has handled (a webhook or resolver that landed while the process
/// restarted), and forget stored payer requests that can no longer be used. Returns how many payments it looked at.
pub async fn sweep(s: &AppState, tenant_id: &str) -> Result<usize, ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    // the stored request (a UPI ID, a return URL) is kept only as long as a retry could still use it
    sqlx::query(
        "UPDATE payment_attempts SET retry_input = NULL
         WHERE retry_input IS NOT NULL AND (status IN ('captured', 'authorized') OR created_at < now() - interval '1 day')",
    )
    .execute(&mut *tx)
    .await?;
    let auto = crate::routing_auto::load(&mut tx, tenant_id).await?;
    let intents: Vec<String> = if auto.soft_retry_active() {
        sqlx::query_scalar(
            "SELECT DISTINCT a.intent_id FROM payment_attempts a
             WHERE a.status = 'failed' AND a.failure_class IN ('provider_fault', 'provider_rejected')
               AND a.retry_input IS NOT NULL AND a.mandate_id IS NULL
               AND a.created_at > now() - make_interval(secs => $1::float8)
               AND NOT EXISTS (SELECT 1 FROM routing_soft_retries r WHERE r.from_attempt_id = a.id)
             LIMIT 50",
        )
        .bind(f64::from(auto.soft_retry_window_seconds))
        .fetch_all(&mut *tx)
        .await?
    } else {
        Vec::new()
    };
    tx.commit().await?;
    for id in &intents {
        run(s, tenant_id, id).await;
    }
    Ok(intents.len())
}

/// True while a soft retry may still start for this payment: the checkout says "trying another way" meanwhile.
/// Becomes false as soon as the retry exists, or was decided against.
pub async fn pending(conn: &mut PgConnection, intent_id: &str) -> Result<bool, ApiError> {
    let tenant_id: Option<String> =
        sqlx::query_scalar("SELECT tenant_id FROM payment_intents WHERE id = $1")
            .bind(intent_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(tenant_id) = tenant_id else {
        return Ok(false);
    };
    let auto = crate::routing_auto::load(conn, &tenant_id).await?;
    if !auto.soft_retry_active() {
        return Ok(false);
    }
    let waiting: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM payment_attempts a
            JOIN payment_intents i ON i.id = a.intent_id
            WHERE a.id = (SELECT id FROM payment_attempts WHERE intent_id = $1 ORDER BY created_at DESC, id DESC LIMIT 1)
              AND i.status = 'requires_payment_method'
              AND a.status = 'failed' AND a.failure_class IN ('provider_fault', 'provider_rejected')
              AND a.retry_input IS NOT NULL AND a.mandate_id IS NULL
              AND NOT EXISTS (SELECT 1 FROM routing_soft_retries r WHERE r.from_attempt_id = a.id)
              AND (SELECT COUNT(*) FROM payment_attempts WHERE intent_id = $1) < $2
              AND now() - (SELECT MIN(created_at) FROM payment_attempts WHERE intent_id = $1) <= make_interval(secs => $3::float8)
         )",
    )
    .bind(intent_id)
    .bind(i64::from(auto.soft_retry_max_attempts))
    .bind(f64::from(auto.soft_retry_window_seconds))
    .fetch_one(conn)
    .await?;
    Ok(waiting)
}
