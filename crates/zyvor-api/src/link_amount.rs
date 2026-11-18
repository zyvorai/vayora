//! Payment-link amount modes, on top of the open-amount links that already exist.
//!
//! | mode | what the payer may pay | what the merchant gives |
//! |---|---|---|
//! | `exact` | the amount on the link (or what an invoice still owes), nothing else | `amount_minor` or `invoice_id` |
//! | `minimum` | that amount or more, with no upper limit but the platform ceiling | `min_amount_minor` |
//! | `up_to` | any amount from 1 minor unit up to a cap | `max_amount_minor` |
//! | `pay_what_you_want` | any amount, optionally bounded below and/or above | `min_amount_minor`, `max_amount_minor` (both optional) and a `suggested_amount_minor` shown as a default |
//!
//! The rule is enforced on the server when the payer starts a checkout (`payment_links::link_start`, which reads
//! the stored minimum and maximum), so a payer who edits the page or calls the API directly cannot pay outside it.
//! The payment intent that is created carries the amount the payer chose, so the checkout, the receipt and the
//! ledger all agree. Coupons and the convenience fee apply to `exact` links only (unchanged).
//! Part payments (`allow_partial`, from the link-upgrades work) also apply only to `exact` links: the balance is what is
//! left of `amount_minor`, which an open amount does not have. Asking for both is refused with `invalid_amount_mode`.
//!
//! These are thin wrappers around `payment_links::create_link` and `payment_links::link_info`: they translate the
//! mode into the open-amount fields, call the original handler, and add what is new. Nothing in the original
//! handlers changes, and a link made without `amount_mode` behaves exactly as before.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{ApiError, AppState, auth::Auth, tenant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmountMode {
    Exact,
    Minimum,
    UpTo,
    PayWhatYouWant,
}

impl AmountMode {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "exact" => Self::Exact,
            "minimum" => Self::Minimum,
            "up_to" => Self::UpTo,
            "pay_what_you_want" => Self::PayWhatYouWant,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Minimum => "minimum",
            Self::UpTo => "up_to",
            Self::PayWhatYouWant => "pay_what_you_want",
        }
    }
}

/// What to store for a new link.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub mode: AmountMode,
    /// The payer types the amount (everything except `exact`).
    pub open: bool,
}

#[derive(Default, Debug)]
pub struct Given {
    pub open_amount: Option<bool>,
    pub has_amount: bool,
    pub has_invoice: bool,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub suggested: Option<i64>,
    /// A part-payment link (`allow_partial`): the payer pays the amount in parts. Only with `exact`.
    pub allow_partial: bool,
}

fn in_range(v: i64) -> bool {
    (1..=zyvor_core::MAX_AMOUNT_MINOR).contains(&v)
}

fn bad(m: &str) -> ApiError {
    ApiError::invalid("invalid_amount_mode", m.to_string())
}

/// Check what the merchant asked for against the chosen mode. `mode` of `None` is the older way of asking:
/// `open_amount` true is `pay_what_you_want`, otherwise `exact`. The original handler still checks the rest
/// (invoice, splits, fee, expiry) after this.
pub fn plan(mode: Option<&str>, g: &Given) -> Result<Plan, ApiError> {
    let open_requested = mode.map_or(g.open_amount.unwrap_or(false), |m| m != "exact");
    if g.allow_partial && open_requested {
        return Err(bad(
            "allow_partial (part payments) only works with an exact amount: the balance is what is left of amount_minor",
        ));
    }
    let mode = match mode {
        None => {
            if g.suggested.is_some() && !g.open_amount.unwrap_or(false) {
                return Err(bad("suggested_amount_minor needs an open-amount mode"));
            }
            return Ok(if g.open_amount.unwrap_or(false) {
                Plan {
                    mode: AmountMode::PayWhatYouWant,
                    open: true,
                }
            } else {
                Plan {
                    mode: AmountMode::Exact,
                    open: false,
                }
            });
        }
        Some(m) => AmountMode::parse(m)
            .ok_or_else(|| bad("amount_mode must be exact, minimum, up_to or pay_what_you_want"))?,
    };
    let open = mode != AmountMode::Exact;
    if g.open_amount.is_some_and(|o| o != open) {
        return Err(bad("open_amount contradicts amount_mode; leave it out"));
    }
    for v in [g.min, g.max, g.suggested].into_iter().flatten() {
        if !in_range(v) {
            return Err(bad(
                "amounts must be positive whole minor units within the platform ceiling",
            ));
        }
    }
    if g.min.zip(g.max).is_some_and(|(lo, hi)| lo > hi) {
        return Err(bad("min_amount_minor is above max_amount_minor"));
    }
    match mode {
        AmountMode::Exact => {
            if g.min.is_some() || g.max.is_some() || g.suggested.is_some() {
                return Err(bad(
                    "an exact link takes no minimum, maximum or suggested amount",
                ));
            }
        }
        AmountMode::Minimum => {
            if g.min.is_none() {
                return Err(bad("a minimum link needs min_amount_minor"));
            }
            if g.max.is_some() || g.suggested.is_some() || g.has_amount || g.has_invoice {
                return Err(bad(
                    "a minimum link takes only min_amount_minor (no maximum, suggested amount, amount or invoice)",
                ));
            }
        }
        AmountMode::UpTo => {
            if g.max.is_none() {
                return Err(bad("an up_to link needs max_amount_minor (the cap)"));
            }
            if g.min.is_some() || g.has_amount || g.has_invoice {
                return Err(bad(
                    "an up_to link takes only max_amount_minor (no minimum, amount or invoice); use pay_what_you_want for both bounds",
                ));
            }
            if g.suggested.zip(g.max).is_some_and(|(s, m)| s > m) {
                return Err(bad("suggested_amount_minor is above the cap"));
            }
        }
        AmountMode::PayWhatYouWant => {
            if g.has_amount || g.has_invoice {
                return Err(bad(
                    "a pay-what-you-want link takes no amount or invoice; give optional min/max bounds",
                ));
            }
            let (lo, hi) = (g.min.unwrap_or(1), g.max.unwrap_or(i64::MAX));
            if g.suggested.is_some_and(|s| s < lo || s > hi) {
                return Err(bad("suggested_amount_minor is outside the min/max bounds"));
            }
        }
    }
    Ok(Plan { mode, open })
}

/// What the server accepts from a payer on an open link: the same rule `link_start` applies, in one place for the
/// tests and the docs. Bounds are inclusive; a missing minimum is 1 and a missing maximum is the platform ceiling.
pub fn check_payer_amount(min: Option<i64>, max: Option<i64>, chosen: i64) -> Result<i64, String> {
    let (lo, hi) = (
        min.unwrap_or(1),
        max.unwrap_or(zyvor_core::MAX_AMOUNT_MINOR),
    );
    if chosen < lo || chosen > hi {
        return Err(format!(
            "the amount must be between {lo} and {hi} minor units"
        ));
    }
    Ok(chosen)
}

/// One plain sentence for the checkout page.
pub fn describe(
    mode: AmountMode,
    min: Option<i64>,
    max: Option<i64>,
    amount: i64,
    currency: &str,
) -> String {
    let m = |v: i64| crate::ai::money(v, currency);
    match mode {
        AmountMode::Exact => format!("Pay exactly {}.", m(amount)),
        AmountMode::Minimum => format!(
            "Pay {} or more. You choose the amount.",
            m(min.unwrap_or(1))
        ),
        AmountMode::UpTo => format!("Pay any amount up to {}.", m(max.unwrap_or(0))),
        AmountMode::PayWhatYouWant => match (min, max) {
            (Some(lo), Some(hi)) => format!("Pay what you want, between {} and {}.", m(lo), m(hi)),
            (Some(lo), None) => format!("Pay what you want, at least {}.", m(lo)),
            (None, Some(hi)) => format!("Pay what you want, up to {}.", m(hi)),
            (None, None) => "Pay what you want.".to_string(),
        },
    }
}

/// `POST /v1/payment_links`. The body is the original link body plus `amount_mode`, `suggested_amount_minor` and
/// `instrument_rules`; everything else is passed through untouched.
#[derive(Deserialize)]
pub struct LinkCreate {
    /// `exact` (the default), `minimum`, `up_to` or `pay_what_you_want`. See the table in the module docs.
    amount_mode: Option<String>,
    /// For open-amount modes: the amount the page offers first, within the bounds. Display only.
    suggested_amount_minor: Option<i64>,
    /// Only these payer banks / UPI handles may pay; a payment from another is refunded when the provider reports who paid.
    instrument_rules: Option<crate::instrument::Rules>,
    #[serde(flatten)]
    base: Map<String, Value>,
}

pub async fn create_link(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<LinkCreate>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require_branch("payments:write")?;
    let mut base = b.base;
    let int = |m: &Map<String, Value>, k: &str| m.get(k).and_then(Value::as_i64);
    let given = Given {
        open_amount: base.get("open_amount").and_then(Value::as_bool),
        has_amount: base.get("amount_minor").is_some_and(|v| !v.is_null()),
        has_invoice: base.get("invoice_id").is_some_and(|v| !v.is_null()),
        min: int(&base, "min_amount_minor"),
        max: int(&base, "max_amount_minor"),
        suggested: b.suggested_amount_minor,
        allow_partial: base
            .get("allow_partial")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    let plan = plan(b.amount_mode.as_deref(), &given)?;
    if let Some(r) = &b.instrument_rules {
        crate::instrument::validate(r)?;
    }
    if plan.open {
        base.insert("open_amount".into(), json!(true));
    }
    let link: crate::payment_links::NewLink =
        serde_json::from_value(Value::Object(base)).map_err(|_| {
            ApiError::invalid(
                "invalid_link",
                "the body is not a valid payment link (check the field types)",
            )
        })?;
    let (tenant_id, actor) = (auth.tenant_id.clone(), auth.actor.clone());
    let (code, Json(mut out)) =
        crate::payment_links::create_link(State(s.clone()), auth, Json(link)).await?;
    // A link returned from an earlier identical request (`existing`) is left as it was made.
    if code != StatusCode::CREATED {
        return Ok((code, Json(out)));
    }
    let id = out["id"].as_str().unwrap_or_default().to_string();
    let rules = b
        .instrument_rules
        .as_ref()
        .map(|r| r.normalised())
        .filter(|r| !r.is_empty())
        .map(|r| json!(r));
    let done = async {
        let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
        sqlx::query("UPDATE payment_links SET amount_mode = $2, suggested_amount_minor = $3, instrument_rules = $4 WHERE id = $1")
            .bind(&id)
            .bind(plan.mode.as_str())
            .bind(b.suggested_amount_minor)
            .bind(&rules)
            .execute(&mut *tx)
            .await?;
        crate::link_lifecycle::record_created(&mut tx, &tenant_id, &actor, &id).await?;
        tx.commit().await?;
        Ok::<(), sqlx::Error>(())
    }
    .await;
    if let Err(e) = done {
        // Do not leave a link that is missing the rule the merchant asked for. Nothing has been shared yet.
        tracing::error!(error = %e, link = %id, "could not finish a payment link; removing it");
        if let Ok(mut tx) = tenant::begin(&s.pool, &tenant_id).await {
            let _ = sqlx::query("DELETE FROM payment_links WHERE id = $1 AND uses = 0")
                .bind(&id)
                .execute(&mut *tx)
                .await;
            let _ = tx.commit().await;
        }
        return Err(ApiError::internal());
    }
    out["amount_mode"] = json!(plan.mode.as_str());
    out["lifecycle_status"] = json!("created");
    Ok((code, Json(out)))
}

#[derive(Deserialize, Default)]
pub struct InfoQuery {
    coupon: Option<String>,
}

/// `GET /v1/checkout/links/{tenant_id}/{token}`: the original page data plus the amount rule in words.
pub async fn link_info(
    State(s): State<AppState>,
    parts: axum::http::request::Parts,
    Path((tenant_id, token)): Path<(String, String)>,
    Query(q): Query<InfoQuery>,
) -> Result<Json<Value>, ApiError> {
    let inner: crate::payment_links::CouponQuery =
        serde_json::from_value(json!({ "coupon": q.coupon })).map_err(|_| ApiError::internal())?;
    let Json(mut out) = crate::payment_links::link_info(
        State(s.clone()),
        parts,
        Path((tenant_id.clone(), token.clone())),
        Query(inner),
    )
    .await?;
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let row: Option<(Option<String>, bool, Option<i64>, Option<i64>, Option<i64>, i64)> =
        sqlx::query_as("SELECT amount_mode, open_amount, min_amount_minor, max_amount_minor, suggested_amount_minor, COALESCE(amount_minor, 0) FROM payment_links WHERE token = $1")
            .bind(&token)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some((mode, open, lo, hi, suggested, amount)) = row {
        let mode = mode
            .as_deref()
            .and_then(AmountMode::parse)
            .unwrap_or(if open {
                AmountMode::PayWhatYouWant
            } else {
                AmountMode::Exact
            });
        let currency = out["currency"].as_str().unwrap_or("INR").to_string();
        let shown = out["amount_minor"].as_i64().unwrap_or(amount);
        out["amount_mode"] = json!(mode.as_str());
        out["suggested_amount_minor"] = json!(suggested);
        out["amount_rule"] = json!(describe(mode, lo, hi, shown, &currency));
    }
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(min: Option<i64>, max: Option<i64>) -> Given {
        Given {
            min,
            max,
            ..Default::default()
        }
    }

    #[test]
    fn older_requests_keep_their_meaning() {
        let p = plan(
            None,
            &Given {
                open_amount: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p.mode, AmountMode::PayWhatYouWant);
        assert!(p.open);
        let p = plan(
            None,
            &Given {
                has_amount: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!((p.mode, p.open), (AmountMode::Exact, false));
        assert!(
            plan(
                None,
                &Given {
                    suggested: Some(5),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn part_payments_only_combine_with_exact() {
        let part = |mode: Option<&str>, open: Option<bool>| {
            plan(
                mode,
                &Given {
                    allow_partial: true,
                    has_amount: true,
                    open_amount: open,
                    ..Default::default()
                },
            )
        };
        assert!(part(None, None).is_ok());
        assert!(part(Some("exact"), None).is_ok());
        assert!(part(Some("minimum"), None).is_err());
        assert!(part(Some("up_to"), None).is_err());
        assert!(part(Some("pay_what_you_want"), None).is_err());
        assert!(part(None, Some(true)).is_err());
    }

    #[test]
    fn each_mode_demands_its_own_fields() {
        // minimum needs a minimum and nothing above it
        assert!(plan(Some("minimum"), &g(None, None)).is_err());
        assert!(plan(Some("minimum"), &g(Some(500), Some(900))).is_err());
        assert_eq!(
            plan(Some("minimum"), &g(Some(500), None)).unwrap().mode,
            AmountMode::Minimum
        );
        // up_to needs a cap and no floor
        assert!(plan(Some("up_to"), &g(None, None)).is_err());
        assert!(plan(Some("up_to"), &g(Some(1), Some(900))).is_err());
        assert_eq!(
            plan(Some("up_to"), &g(None, Some(900))).unwrap().mode,
            AmountMode::UpTo
        );
        // pay what you want takes optional bounds, never inverted
        assert!(plan(Some("pay_what_you_want"), &g(None, None)).is_ok());
        assert!(plan(Some("pay_what_you_want"), &g(Some(100), Some(900))).is_ok());
        assert!(plan(Some("pay_what_you_want"), &g(Some(900), Some(100))).is_err());
        // exact takes none of them
        assert!(plan(Some("exact"), &g(Some(1), None)).is_err());
        let ok = plan(
            Some("exact"),
            &Given {
                has_amount: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!ok.open);
    }

    #[test]
    fn modes_cannot_be_mixed_with_amounts_or_unknown_names() {
        assert!(plan(Some("sometimes"), &g(None, None)).is_err());
        let with_amount = Given {
            has_amount: true,
            min: Some(5),
            ..Default::default()
        };
        assert!(plan(Some("minimum"), &with_amount).is_err());
        let contradiction = Given {
            open_amount: Some(false),
            min: Some(5),
            ..Default::default()
        };
        assert!(plan(Some("minimum"), &contradiction).is_err());
        // out-of-range numbers
        assert!(plan(Some("up_to"), &g(None, Some(0))).is_err());
        assert!(
            plan(
                Some("up_to"),
                &g(None, Some(zyvor_core::MAX_AMOUNT_MINOR + 1))
            )
            .is_err()
        );
        // suggested must sit inside the bounds
        let s = |v| Given {
            suggested: Some(v),
            min: Some(100),
            max: Some(900),
            ..Default::default()
        };
        assert!(plan(Some("pay_what_you_want"), &s(50)).is_err());
        assert!(plan(Some("pay_what_you_want"), &s(500)).is_ok());
    }

    #[test]
    fn payer_amounts_are_bounded_inclusively() {
        // minimum: at least the floor, no cap
        assert!(check_payer_amount(Some(500), None, 499).is_err());
        assert_eq!(check_payer_amount(Some(500), None, 500), Ok(500));
        assert!(check_payer_amount(Some(500), None, 5_000_000).is_ok());
        // up to: 1 .. cap
        assert!(check_payer_amount(None, Some(900), 0).is_err());
        assert_eq!(check_payer_amount(None, Some(900), 900), Ok(900));
        assert!(check_payer_amount(None, Some(900), 901).is_err());
        // never above the platform ceiling, whatever the link says
        assert!(check_payer_amount(None, None, zyvor_core::MAX_AMOUNT_MINOR + 1).is_err());
        assert!(check_payer_amount(None, None, -5).is_err());
    }

    #[test]
    fn the_sentence_says_what_the_payer_may_do() {
        let d = |m, lo, hi| describe(m, lo, hi, 0, "INR");
        assert_eq!(
            d(AmountMode::Minimum, Some(50_000), None),
            "Pay ₹500.00 or more. You choose the amount."
        );
        assert_eq!(
            d(AmountMode::UpTo, None, Some(250_000)),
            "Pay any amount up to ₹2500.00."
        );
        assert_eq!(
            d(AmountMode::PayWhatYouWant, Some(100), Some(900)),
            "Pay what you want, between ₹1.00 and ₹9.00."
        );
        assert_eq!(
            describe(AmountMode::Exact, None, None, 49_900, "INR"),
            "Pay exactly ₹499.00."
        );
    }
}
