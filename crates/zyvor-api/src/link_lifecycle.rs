// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! One lifecycle status for every payment link, and the rules for moving between statuses.
//!
//! The older `status` field on a link (`active`, `expired`, `canceled`, `used_up`) stays as it was. This is a
//! separate, normalised field, `lifecycle_status`, so a client can follow a link with one vocabulary and one set of
//! events.
//!
//! | status | meaning |
//! |---|---|
//! | `created` | The link exists and can be paid; nobody has started a checkout on it yet. |
//! | `active` | At least one checkout has been started and the link can still take payments. |
//! | `partially_paid` | Money has come in but the link expects more: an invoice with a balance, a part-payment link with a balance, or a link capped at several uses that has not used them all. |
//! | `paid` | The link has everything it expects: its invoice is settled, its part-payment balance is zero, or every use it was capped at has been paid. A link with no total to reach (an open, reusable link) never becomes `paid`. |
//! | `expired` | The expiry passed before the link was paid in full. |
//! | `cancelled` | The merchant cancelled it. |
//! | `failed` | The link was capped at N uses, all N checkouts were started, none was paid and none is still in progress. |
//!
//! Evaluated in this order, the first that applies wins: paid, cancelled, failed, expired, partially paid, active,
//! created. So a payment that arrives on a cancelled or expired link (a checkout opened earlier and finished
//! later) moves it to `paid` when that completes the link; the money is never lost from view.
//!
//! Allowed transitions (anything else is refused and audited, never applied):
//! `created` to `active`, `partially_paid`, `paid`, `failed`, `expired`, `cancelled`;
//! `active` to `partially_paid`, `paid`, `failed`, `expired`, `cancelled`;
//! `partially_paid` to `paid`, `expired`, `cancelled`;
//! `failed` to `partially_paid`, `paid`, `cancelled`;
//! `expired` to `paid`, `cancelled`;
//! `cancelled` to `paid`; `paid` is final.
//! Each change is written once to `payment_link_status_events` and raised as the event `payment_link.<status>`.
//!
//! The status is computed from the link and the payments made through it, so nothing a payer's browser does can set
//! it. The worker records transitions (`sweep_tenant`); creating and cancelling a link record theirs at once.
//! `GET /v1/payment_links/{id}/lifecycle` always shows the current computed status next to the recorded one, so a
//! change the worker has not yet seen is visible.

use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::{ApiError, AppState, auth::Auth, tenant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Created,
    Active,
    PartiallyPaid,
    Paid,
    Expired,
    Cancelled,
    Failed,
}

pub const ALL: [Status; 7] = [
    Status::Created,
    Status::Active,
    Status::PartiallyPaid,
    Status::Paid,
    Status::Expired,
    Status::Cancelled,
    Status::Failed,
];

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Created => "created",
            Status::Active => "active",
            Status::PartiallyPaid => "partially_paid",
            Status::Paid => "paid",
            Status::Expired => "expired",
            Status::Cancelled => "cancelled",
            Status::Failed => "failed",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        ALL.into_iter().find(|x| x.as_str() == s)
    }
    pub fn describe(self) -> &'static str {
        match self {
            Status::Created => {
                "The link exists and can be paid; no checkout has been started on it."
            }
            Status::Active => "A checkout has been started and the link can still take payments.",
            Status::PartiallyPaid => "Some money has come in but the link expects more.",
            Status::Paid => "The link has everything it expects.",
            Status::Expired => "The expiry passed before the link was paid in full.",
            Status::Cancelled => "The merchant cancelled the link.",
            Status::Failed => {
                "Every checkout the link allowed was started, none was paid and none is in progress."
            }
        }
    }
    /// Where a link in this status may go next.
    pub fn next(self) -> &'static [Status] {
        use Status::*;
        match self {
            Created => &[Active, PartiallyPaid, Paid, Failed, Expired, Cancelled],
            Active => &[PartiallyPaid, Paid, Failed, Expired, Cancelled],
            PartiallyPaid => &[Paid, Expired, Cancelled],
            Failed => &[PartiallyPaid, Paid, Cancelled],
            Expired => &[Paid, Cancelled],
            Cancelled => &[Paid],
            Paid => &[],
        }
    }
    pub fn can_become(self, to: Status) -> bool {
        self.next().contains(&to)
    }
    pub fn event(self) -> &'static str {
        match self {
            Status::Created => "payment_link.created",
            Status::Active => "payment_link.activated",
            Status::PartiallyPaid => "payment_link.partially_paid",
            Status::Paid => "payment_link.paid",
            Status::Expired => "payment_link.expired",
            Status::Cancelled => "payment_link.cancelled",
            Status::Failed => "payment_link.failed",
        }
    }
}

/// What the status is computed from.
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Facts {
    pub id: String,
    pub canceled_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub max_uses: Option<i32>,
    pub uses: i32,
    pub amount_minor: Option<i64>,
    pub has_invoice: bool,
    /// What the link's invoice still owes, when it has one.
    pub owed_minor: Option<i64>,
    /// A part-payment link (the link-upgrades feature); false where that column does not exist.
    pub partial: bool,
    pub succeeded: i64,
    pub paid_minor: i64,
    pub processing: i64,
    /// Checkouts that are not paid and have not had a failed attempt yet (still being filled in, or never tried).
    pub untried: i64,
    pub started: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Derived {
    pub status: Status,
    /// Whether a payer can start a checkout on it right now.
    pub payable: bool,
}

/// The status from the facts. Pure: the same facts and clock always give the same answer.
pub fn derive(f: &Facts, now: DateTime<Utc>) -> Derived {
    let expired = f.expires_at.is_some_and(|e| e <= now);
    let used_up = f.max_uses.is_some_and(|m| f.uses >= m);
    let complete = if f.has_invoice {
        f.owed_minor.is_some_and(|o| o <= 0)
    } else if f.partial {
        f.amount_minor.is_some_and(|a| f.paid_minor >= a)
    } else {
        f.max_uses.is_some_and(|m| f.succeeded >= i64::from(m))
    };
    let has_total = f.has_invoice || f.partial || f.max_uses.is_some_and(|m| m > 1);
    let failed =
        used_up && f.succeeded == 0 && f.processing == 0 && f.untried == 0 && f.started > 0;
    let status = if complete {
        Status::Paid
    } else if f.canceled_at.is_some() {
        Status::Cancelled
    } else if failed {
        Status::Failed
    } else if expired {
        Status::Expired
    } else if f.succeeded > 0 && has_total {
        Status::PartiallyPaid
    } else if f.uses > 0 {
        Status::Active
    } else {
        Status::Created
    };
    let payable = matches!(
        status,
        Status::Created | Status::Active | Status::PartiallyPaid
    ) && !used_up
        && !expired;
    Derived { status, payable }
}

/// The facts for these links, in one query.
pub async fn load_facts(
    conn: &mut PgConnection,
    ids: &[String],
) -> Result<Vec<Facts>, sqlx::Error> {
    // `to_jsonb(l)->>'allow_partial'` is NULL where the column does not exist, so this reads the same on a build
    // without part-payment links.
    sqlx::query_as(
        "SELECT l.id, l.canceled_at, l.expires_at, l.max_uses, l.uses, l.amount_minor,
                l.invoice_id IS NOT NULL AS has_invoice,
                (SELECT (i.amount_minor - invoice_settled(i.id))::BIGINT FROM invoices i WHERE i.id = l.invoice_id) AS owed_minor,
                COALESCE((to_jsonb(l)->>'allow_partial')::boolean, false) AS partial,
                COALESCE(a.succeeded, 0)::BIGINT AS succeeded, COALESCE(a.paid, 0)::BIGINT AS paid_minor,
                COALESCE(a.processing, 0)::BIGINT AS processing, COALESCE(a.untried, 0)::BIGINT AS untried,
                COALESCE(a.started, 0)::BIGINT AS started
           FROM payment_links l
           LEFT JOIN LATERAL (
                SELECT count(*) FILTER (WHERE i.status = 'succeeded') AS succeeded,
                       COALESCE(sum((i.metadata->>'base_minor')::bigint) FILTER (WHERE i.status = 'succeeded'), 0) AS paid,
                       count(*) FILTER (WHERE i.status = 'processing') AS processing,
                       count(*) FILTER (WHERE i.status NOT IN ('succeeded', 'processing')
                                         AND NOT EXISTS (SELECT 1 FROM payment_attempts t WHERE t.intent_id = i.id AND t.status = 'failed')) AS untried,
                       count(*) AS started
                  FROM payment_intents i WHERE i.metadata->>'payment_link' = l.id) a ON true
          WHERE l.id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await
}

/// A new link: record `created`, once. The column already starts at `created`.
pub async fn record_created(
    conn: &mut PgConnection,
    tenant_id: &str,
    actor: &str,
    link_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE payment_links SET lifecycle_at = now() WHERE id = $1")
        .bind(link_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("INSERT INTO payment_link_status_events (id, tenant_id, link_id, from_status, to_status, reason) VALUES ($1, $2, $3, NULL, 'created', $4)")
        .bind(zyvor_core::new_id("lse"))
        .bind(tenant_id)
        .bind(link_id)
        .bind(format!("created by {actor}"))
        .execute(&mut *conn)
        .await?;
    tenant::emit(
        conn,
        tenant_id,
        Status::Created.event(),
        json!({ "link_id": link_id, "from": null, "to": "created" }),
    )
    .await?;
    Ok(())
}

fn reason_for(to: Status) -> &'static str {
    match to {
        Status::Created => "created",
        Status::Active => "a checkout was started",
        Status::PartiallyPaid => "a payment came in and the link expects more",
        Status::Paid => "the link has received everything it expects",
        Status::Expired => "the expiry passed",
        Status::Cancelled => "cancelled by the merchant",
        Status::Failed => "every allowed checkout failed",
    }
}

/// Bring the recorded status up to the computed one. Returns the change made, if any. A computed status the table
/// does not allow from the recorded one is not applied; it is audited so someone can look.
pub async fn sync(
    conn: &mut PgConnection,
    tenant_id: &str,
    link_id: &str,
) -> Result<Option<(Status, Status)>, ApiError> {
    let recorded: Option<String> =
        sqlx::query_scalar("SELECT lifecycle_status FROM payment_links WHERE id = $1 FOR UPDATE")
            .bind(link_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(recorded) = recorded.as_deref().and_then(Status::parse) else {
        return Err(ApiError::not_found("payment link"));
    };
    let facts = load_facts(conn, &[link_id.to_string()]).await?;
    let Some(f) = facts.first() else {
        return Err(ApiError::not_found("payment link"));
    };
    let want = derive(f, Utc::now()).status;
    sqlx::query("UPDATE payment_links SET lifecycle_checked_at = now() WHERE id = $1")
        .bind(link_id)
        .execute(&mut *conn)
        .await?;
    if want == recorded {
        return Ok(None);
    }
    if !recorded.can_become(want) {
        tenant::audit(
            conn,
            tenant_id,
            "system:lifecycle",
            "payment_link.lifecycle_refused",
            Some(link_id),
            json!({ "from": recorded.as_str(), "to": want.as_str() }),
        )
        .await?;
        return Ok(None);
    }
    sqlx::query("UPDATE payment_links SET lifecycle_status = $2, lifecycle_at = now() WHERE id = $1 AND lifecycle_status = $3")
        .bind(link_id)
        .bind(want.as_str())
        .bind(recorded.as_str())
        .execute(&mut *conn)
        .await?;
    sqlx::query("INSERT INTO payment_link_status_events (id, tenant_id, link_id, from_status, to_status, reason) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(zyvor_core::new_id("lse"))
        .bind(tenant_id)
        .bind(link_id)
        .bind(recorded.as_str())
        .bind(want.as_str())
        .bind(reason_for(want))
        .execute(&mut *conn)
        .await?;
    tenant::emit(
        conn,
        tenant_id,
        want.event(),
        json!({ "link_id": link_id, "from": recorded.as_str(), "to": want.as_str(), "paid_minor": f.paid_minor }),
    )
    .await?;
    Ok(Some((recorded, want)))
}

/// The worker's pass for one tenant: look at links that are still moving (and recently ended ones, which a late
/// payment can still complete). Each link is handled in its own transaction. Returns how many changed.
pub async fn sweep_tenant(s: &AppState, tenant_id: &str) -> Result<usize, ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM payment_links
          WHERE lifecycle_status <> 'paid'
            AND (lifecycle_status IN ('created', 'active', 'partially_paid') OR lifecycle_at IS NULL OR lifecycle_at > now() - interval '30 days')
          ORDER BY lifecycle_checked_at NULLS FIRST, id LIMIT 200",
    )
    .fetch_all(&mut *tx)
    .await?;
    drop(tx);
    let mut changed = 0;
    for id in ids {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        match sync(&mut tx, tenant_id, &id).await {
            Ok(c) => {
                tx.commit().await?;
                changed += usize::from(c.is_some());
            }
            Err(e) => {
                tracing::warn!(link = %id, error = ?e.message, "payment link lifecycle sweep failed")
            }
        }
    }
    Ok(changed)
}

/// `POST /v1/payment_links/{id}/cancel`: the original handler, then the lifecycle record.
pub async fn cancel(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let tenant_id = auth.tenant_id.clone();
    let Json(mut out) =
        crate::payment_links::cancel_link(State(s.clone()), auth, Path(id.clone())).await?;
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    sync(&mut tx, &tenant_id, &id).await?;
    let now: String =
        sqlx::query_scalar("SELECT lifecycle_status FROM payment_links WHERE id = $1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    out["lifecycle_status"] = json!(now);
    Ok(Json(out))
}

/// `GET /v1/payment_links`: the original list, each row with its computed lifecycle status and amount mode.
pub async fn list(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    let tenant_id = auth.tenant_id.clone();
    let Json(mut out) = crate::payment_links::list_links(State(s.clone()), auth).await?;
    let ids: Vec<String> = out["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["id"].as_str().map(str::to_string))
        .collect();
    if ids.is_empty() {
        return Ok(Json(out));
    }
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let facts = load_facts(&mut tx, &ids).await?;
    let modes: Vec<(String, Option<String>, bool)> =
        sqlx::query_as("SELECT id, amount_mode, open_amount FROM payment_links WHERE id = ANY($1)")
            .bind(&ids)
            .fetch_all(&mut *tx)
            .await?;
    let now = Utc::now();
    if let Some(rows) = out["data"].as_array_mut() {
        for r in rows {
            let id = r["id"].as_str().unwrap_or_default().to_string();
            if let Some(f) = facts.iter().find(|f| f.id == id) {
                let d = derive(f, now);
                r["lifecycle_status"] = json!(d.status.as_str());
                r["payable"] = json!(d.payable);
                r["paid_minor"] = json!(f.paid_minor);
            }
            if let Some((_, mode, open)) = modes.iter().find(|m| m.0 == id) {
                r["amount_mode"] = json!(mode.clone().unwrap_or_else(|| if *open {
                    "pay_what_you_want".into()
                } else {
                    "exact".into()
                }));
            }
        }
    }
    Ok(Json(out))
}

/// `GET /v1/payment_links/lifecycle`: the model itself, for clients that want to render or validate against it.
pub async fn model(State(_): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require_branch("payments:read")?;
    let statuses: Vec<Value> = ALL
        .iter()
        .map(|s| {
            json!({
                "status": s.as_str(),
                "description": s.describe(),
                "event": s.event(),
                "terminal": s.next().is_empty(),
                "next": s.next().iter().map(|n| n.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(Json(json!({
        "data": statuses,
        "precedence": ["paid", "cancelled", "failed", "expired", "partially_paid", "active", "created"],
        "note": "The status is computed from the link and the payments made through it. A payment finishing after a cancel or expiry can still move the link to paid.",
    })))
}

/// `GET /v1/payment_links/{id}/lifecycle`: the computed status now, the recorded one, and every change so far.
pub async fn get(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require_branch("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let recorded: Option<String> =
        sqlx::query_scalar("SELECT lifecycle_status FROM payment_links WHERE id = $1")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?;
    let recorded = recorded.ok_or_else(|| ApiError::not_found("payment link"))?;
    let facts = load_facts(&mut tx, std::slice::from_ref(&id)).await?;
    let f = facts
        .first()
        .ok_or_else(|| ApiError::not_found("payment link"))?;
    let d = derive(f, Utc::now());
    let history: Vec<(Option<String>, String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT from_status, to_status, reason, at FROM payment_link_status_events WHERE link_id = $1 ORDER BY at, id",
    )
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({
        "id": id,
        "status": d.status.as_str(),
        "recorded_status": recorded,
        "payable": d.payable,
        "paid_minor": f.paid_minor,
        "payments": f.succeeded,
        "checkouts_started": f.started,
        "allowed_next": d.status.next().iter().map(|n| n.as_str()).collect::<Vec<_>>(),
        "history": history.into_iter().map(|(from, to, reason, at)| json!({ "from": from, "to": to, "reason": reason, "at": at })).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    fn f() -> Facts {
        Facts {
            id: "plk_1".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_new_link_is_created_and_payable() {
        let d = derive(&f(), now());
        assert_eq!((d.status, d.payable), (Status::Created, true));
    }

    #[test]
    fn a_started_checkout_makes_it_active() {
        let d = derive(
            &Facts {
                uses: 1,
                started: 1,
                untried: 1,
                ..f()
            },
            now(),
        );
        assert_eq!(d.status, Status::Active);
        // a reusable link that was paid once keeps collecting: it has no total to reach
        let d = derive(
            &Facts {
                uses: 3,
                succeeded: 3,
                paid_minor: 150,
                started: 3,
                ..f()
            },
            now(),
        );
        assert_eq!((d.status, d.payable), (Status::Active, true));
    }

    #[test]
    fn single_use_links_end_paid_or_failed() {
        let paid = Facts {
            max_uses: Some(1),
            uses: 1,
            succeeded: 1,
            paid_minor: 500,
            started: 1,
            ..f()
        };
        let d = derive(&paid, now());
        assert_eq!((d.status, d.payable), (Status::Paid, false));
        let failed = Facts {
            max_uses: Some(1),
            uses: 1,
            started: 1,
            ..f()
        };
        assert_eq!(derive(&failed, now()).status, Status::Failed);
        // not failed while the payer is still filling the checkout in, or a payment is in progress
        assert_eq!(
            derive(
                &Facts {
                    untried: 1,
                    ..failed.clone()
                },
                now()
            )
            .status,
            Status::Active
        );
        assert_eq!(
            derive(
                &Facts {
                    processing: 1,
                    ..failed
                },
                now()
            )
            .status,
            Status::Active
        );
    }

    #[test]
    fn partial_payments_and_invoices_have_a_balance() {
        let inv = Facts {
            has_invoice: true,
            owed_minor: Some(600),
            succeeded: 1,
            paid_minor: 400,
            uses: 1,
            started: 1,
            ..f()
        };
        assert_eq!(derive(&inv, now()).status, Status::PartiallyPaid);
        let settled = Facts {
            owed_minor: Some(0),
            ..inv
        };
        assert_eq!(derive(&settled, now()).status, Status::Paid);
        let part = Facts {
            partial: true,
            amount_minor: Some(1000),
            succeeded: 2,
            paid_minor: 600,
            uses: 2,
            started: 2,
            ..f()
        };
        assert_eq!(derive(&part, now()).status, Status::PartiallyPaid);
        assert_eq!(
            derive(
                &Facts {
                    paid_minor: 1000,
                    ..part
                },
                now()
            )
            .status,
            Status::Paid
        );
        // capped at three uses, one paid
        let capped = Facts {
            max_uses: Some(3),
            uses: 2,
            succeeded: 1,
            paid_minor: 100,
            started: 2,
            ..f()
        };
        assert_eq!(derive(&capped, now()).status, Status::PartiallyPaid);
    }

    #[test]
    fn expiry_and_cancel_beat_progress_but_not_completion() {
        let past = Some(now() - Duration::seconds(5));
        let e = Facts {
            expires_at: past,
            uses: 1,
            started: 1,
            ..f()
        };
        let d = derive(&e, now());
        assert_eq!((d.status, d.payable), (Status::Expired, false));
        let part = Facts {
            expires_at: past,
            succeeded: 1,
            has_invoice: true,
            owed_minor: Some(10),
            ..f()
        };
        assert_eq!(derive(&part, now()).status, Status::Expired);
        let c = Facts {
            canceled_at: Some(now()),
            ..f()
        };
        assert_eq!(derive(&c, now()).status, Status::Cancelled);
        // a payment that finishes the link after a cancel or expiry still shows as paid
        let late = Facts {
            canceled_at: Some(now()),
            max_uses: Some(1),
            succeeded: 1,
            uses: 1,
            ..f()
        };
        assert_eq!(derive(&late, now()).status, Status::Paid);
        let future = Facts {
            expires_at: Some(now() + Duration::hours(1)),
            ..f()
        };
        assert_eq!(derive(&future, now()).status, Status::Created);
    }

    #[test]
    fn the_transition_table_is_closed_and_final_states_stay_final() {
        assert!(Status::Paid.next().is_empty());
        assert!(Status::Created.can_become(Status::Active));
        assert!(!Status::Active.can_become(Status::Created));
        assert!(!Status::Paid.can_become(Status::Cancelled));
        assert!(!Status::Expired.can_become(Status::Active));
        for s in ALL {
            assert!(!s.next().contains(&s), "{} lists itself", s.as_str());
            assert_eq!(Status::parse(s.as_str()), Some(s));
            assert!(
                crate::events::known(s.event()),
                "{} not in the catalogue",
                s.event()
            );
        }
        assert_eq!(
            Status::parse("canceled"),
            None,
            "the normalised spelling is `cancelled`"
        );
    }

    #[test]
    fn every_status_the_derivation_can_produce_follows_the_table_from_where_it_was() {
        // Walk a link through its life; each step the derivation takes must be one the table allows.
        let past = Some(now() - Duration::seconds(5));
        let steps = [
            f(),
            Facts {
                uses: 1,
                started: 1,
                untried: 1,
                ..f()
            },
            Facts {
                has_invoice: true,
                owed_minor: Some(10),
                succeeded: 1,
                uses: 1,
                started: 1,
                ..f()
            },
            Facts {
                has_invoice: true,
                owed_minor: Some(10),
                succeeded: 1,
                uses: 1,
                started: 1,
                expires_at: past,
                ..f()
            },
            Facts {
                has_invoice: true,
                owed_minor: Some(0),
                succeeded: 2,
                uses: 2,
                started: 2,
                expires_at: past,
                ..f()
            },
        ];
        let mut at = Status::Created;
        for s in steps {
            let to = derive(&s, now()).status;
            assert!(
                at == to || at.can_become(to),
                "{} -> {}",
                at.as_str(),
                to.as_str()
            );
            at = to;
        }
        assert_eq!(at, Status::Paid);
    }
}
