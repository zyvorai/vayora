//! Safe, explainable routing.
//!
//! Hard filters (disabled, not allowed, capability mismatch, rule exclusion) can never be
//! overridden. Soft signals (the success-rate signal and the circuit breaker, both computed in the API's `health`
//! module) only reorder. There is one source of truth for provider health: the breaker. Every decision records
//! which providers were considered, why each was chosen or rejected, and a plain-language sentence for each.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zyvor_core::Currency;

use crate::{Capabilities, Method};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Fee {
    pub bps: u32,
    pub fixed_minor: i64,
}

impl Fee {
    /// Estimated fee in minor units (rounded half up), integer arithmetic only.
    pub fn estimate(&self, amount_minor: i64) -> i64 {
        (amount_minor * self.bps as i64 + 5_000) / 10_000 + self.fixed_minor
    }
}

/// Where the circuit breaker stands for one provider and method.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Breaker {
    /// Taking new payments normally.
    #[default]
    Healthy,
    /// Skipped for new payments while it cools down.
    Degraded,
    /// Degraded, the cool-down is over and no probe is out: one new payment may test it.
    ProbeDue,
}

/// The recent outcome of one provider for one method, derived from stored attempts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Signal {
    /// Resolved attempts in the window (successes + failures + unresolved). Merchant cancellations, payer
    /// declines and attempts still in flight are not counted.
    pub samples: u32,
    pub successes: u32,
    /// Definitive failures that count against the provider: it failed, refused the request, or the decline
    /// could not be traced to the payer (unknown causes count here, the conservative choice).
    pub failures: u32,
    /// Declines that were the payer's doing (insufficient funds, the payer left or refused). Not counted in
    /// `samples`, `failures` or the success rate: they say nothing about the provider. Shown so they stay visible.
    #[serde(default)]
    pub payer_declines: u32,
    /// Attempts whose outcome is unknown. They count against the provider for the breaker.
    pub unresolved: u32,
    /// Successes over samples, in basis points. `None` until there are at least `min_samples`
    /// attempts: a tiny sample never moves routing.
    pub success_rate_bps: Option<u32>,
    pub min_samples: u32,
    pub breaker: Breaker,
}

/// How much better (basis points) one provider's success rate must be before it beats a cheaper one.
pub const DEFAULT_SUCCESS_RATE_MARGIN_BPS: u32 = 200;

#[derive(Debug, Clone)]
pub struct Candidate {
    pub provider: String,
    pub caps: Capabilities,
    pub enabled: bool,
    /// Configured fee per method.
    pub fees: BTreeMap<Method, Fee>,
    /// Success-rate signal and breaker state for the method being routed (none when not computed).
    pub signal: Option<Signal>,
    /// What this payment would cost on this provider, from fee rules and observed fees (none when not computed).
    /// Used only when the request carries a `CostPolicy`.
    pub cost: Option<Cost>,
}

/// The estimated cost of one payment on one provider, and where the number comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cost {
    pub minor: i64,
    /// `observed` (fees the provider actually deducted), `fee_rule` (a contract rule the merchant entered) or
    /// `configured` (the routing fee table).
    pub basis: &'static str,
}

/// Cost-aware routing, off unless a request carries it. See `docs/zyvor/ROUTING.md` ("Cost-aware routing").
///
/// Among the eligible providers the cheapest wins, but only a provider with enough recent attempts (the signal's
/// `min_samples`) and a success rate at or above `floor_bps` can win on cost. A provider whose success rate is
/// better by at least `success_rate_margin_bps` still wins over a cheaper one if it costs no more than
/// `premium_bps` extra. Nothing here can pick a provider the hard filters, failover or the circuit breaker removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostPolicy {
    /// Success rate (basis points) a provider needs before cost may decide.
    pub floor_bps: u32,
    /// How much more (basis points of the cheapest cost) a clearly more reliable provider may cost.
    pub premium_bps: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Rule {
    pub method: Option<Method>,
    pub min_amount_minor: Option<i64>,
    pub max_amount_minor: Option<i64>,
    #[serde(default)]
    pub prefer: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Among the eligible providers, prefer the one with the better recent success rate. Applies only when every
    /// remaining provider has enough samples and the best leads by `success_rate_margin_bps`; otherwise the
    /// lowest fee decides. A `prefer` list, when it names an eligible provider, still wins.
    #[serde(default)]
    pub prefer_success_rate: bool,
    /// Lead (basis points) the best provider needs. Default 200.
    #[serde(default)]
    pub success_rate_margin_bps: Option<u32>,
}

impl Rule {
    fn matches(&self, req: &Request) -> bool {
        self.method.is_none_or(|m| m == req.method)
            && self.min_amount_minor.is_none_or(|v| req.amount_minor >= v)
            && self.max_amount_minor.is_none_or(|v| req.amount_minor <= v)
    }
}

/// A provider that already failed this payment definitively (nothing was taken, nothing is pending there).
#[derive(Debug, Clone)]
pub struct Avoid {
    pub provider: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub amount_minor: i64,
    pub currency: Currency,
    /// Caller restriction (e.g. per-intent allowed providers).
    pub allowed: Option<Vec<String>>,
    /// A UPI payment with no payer-typed UPI ID: the provider must issue the QR / intent itself.
    pub upi_intent: bool,
    /// Failover: providers that definitively failed this payment already. They are skipped when another
    /// eligible provider exists, and used again only when nothing else is eligible.
    pub avoid: Vec<Avoid>,
    /// Cost-aware routing (none = the lowest configured fee decides, as before).
    pub cost: Option<CostPolicy>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Considered {
    pub provider: String,
    pub outcome: &'static str,
    pub reason: String,
    pub estimated_fee_minor: Option<i64>,
    /// One plain-language sentence for a person: "Chosen because ...", "Skipped because ...".
    pub explanation: String,
    /// The success-rate signal this decision saw.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<Signal>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub provider: String,
    /// Why this provider, in one plain-language sentence (the same text as its `considered` entry).
    pub explanation: String,
    pub reasons: Vec<String>,
    pub considered: Vec<Considered>,
    /// Providers skipped because they already failed this payment definitively.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failover_from: Vec<String>,
    /// Set when this payment is the circuit breaker's probe of a degraded provider. The caller must claim the
    /// probe slot atomically and choose again if it loses the race.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("no provider can handle this payment: {0}")]
pub struct NoRoute(pub String);

#[derive(Clone)]
struct Elig<'a> {
    c: &'a Candidate,
    fee: i64,
    /// Where `fee` comes from, for the sentence (`None` = the configured fee table).
    basis: Option<&'static str>,
}

fn note(
    c: &Candidate,
    outcome: &'static str,
    reason: String,
    explanation: String,
    fee: Option<i64>,
) -> Considered {
    Considered {
        provider: c.provider.clone(),
        outcome,
        reason,
        explanation,
        estimated_fee_minor: fee,
        signal: c.signal.clone(),
    }
}

fn breaker(c: &Candidate) -> Breaker {
    c.signal.as_ref().map(|s| s.breaker).unwrap_or_default()
}

/// Basis points as a percentage with one decimal ("95.0%").
fn pct(bps: u32) -> String {
    format!("{}.{}%", bps / 100, (bps % 100) / 10)
}

/// "3 of its last 20 payments failed" for the sentence that explains a degraded provider.
fn degraded_detail(c: &Candidate) -> String {
    match &c.signal {
        Some(s) if s.samples > 0 => format!(
            ": {} of its last {} payments failed or are unresolved",
            s.failures + s.unresolved,
            s.samples
        ),
        _ => String::new(),
    }
}

/// How an estimated cost is described: "INR 5.00, from fees the provider actually deducted".
fn cost_text(e: &Elig, money: &dyn Fn(i64) -> String) -> String {
    let src = match e.basis {
        Some("observed") => "from fees the provider actually deducted",
        Some("fee_rule") => "from your fee rule",
        _ => "from your configured fee",
    };
    format!("{}, {src}", money(e.fee))
}

/// The cost-aware choice among two or more providers (see `CostPolicy`): the index of the chosen provider and the
/// end of the sentence "Chosen because it ...", naming what was traded off. Guardrails, in order:
/// 1. only a provider with a known success rate (enough recent attempts) at or above the floor may win on cost;
/// 2. if none qualifies, a provider with no success-rate data may still be chosen on cost (it is not known to be
///    below the floor); when every provider has data and all are below the floor, the best success rate wins,
///    never the cheapest.
fn cost_choice(
    pool: &[Elig],
    policy: &CostPolicy,
    money: &dyn Fn(i64) -> String,
    reasons: &mut Vec<String>,
) -> (usize, String) {
    let rate = |e: &Elig| e.c.signal.as_ref().and_then(|s| s.success_rate_bps);
    let floor = policy.floor_bps;
    let cheapest_of = |idx: &[usize]| {
        *idx.iter()
            .min_by(|&&a, &&b| {
                pool[a]
                    .fee
                    .cmp(&pool[b].fee)
                    .then_with(|| rate(&pool[b]).cmp(&rate(&pool[a])))
                    .then_with(|| pool[a].c.provider.cmp(&pool[b].c.provider))
            })
            .expect("non-empty")
    };
    let good: Vec<usize> = (0..pool.len())
        .filter(|&i| rate(&pool[i]).is_some_and(|r| r >= floor))
        .collect();
    let unknown: Vec<usize> = (0..pool.len())
        .filter(|&i| rate(&pool[i]).is_none())
        .collect();
    let below: Vec<usize> = (0..pool.len())
        .filter(|&i| rate(&pool[i]).is_some_and(|r| r < floor))
        .collect();

    if !good.is_empty() {
        let cheapest = cheapest_of(&good);
        let cr = rate(&pool[cheapest]).unwrap_or(0);
        // Trade a little cost for a clearly better success rate.
        let better = good
            .iter()
            .copied()
            .filter(|&i| {
                rate(&pool[i]).unwrap_or(0) >= cr + DEFAULT_SUCCESS_RATE_MARGIN_BPS
                    && i128::from(pool[i].fee) * 10_000
                        <= i128::from(pool[cheapest].fee) * i128::from(10_000 + policy.premium_bps)
            })
            .max_by(|&a, &b| {
                rate(&pool[a])
                    .cmp(&rate(&pool[b]))
                    .then_with(|| pool[b].fee.cmp(&pool[a].fee))
                    .then_with(|| pool[b].c.provider.cmp(&pool[a].c.provider))
            });
        let mut left_out = String::new();
        let excluded: Vec<String> = below
            .iter()
            .chain(unknown.iter())
            .filter(|&&i| pool[i].fee < pool[cheapest].fee)
            .map(|&i| {
                format!(
                    "{} (cheaper at {}, but {})",
                    pool[i].c.provider,
                    money(pool[i].fee),
                    if rate(&pool[i]).is_some() {
                        format!("its success rate is below your {} floor", pct(floor))
                    } else {
                        "it has too few recent attempts to show it is above your success-rate floor"
                            .to_string()
                    }
                )
            })
            .collect();
        if !excluded.is_empty() {
            left_out = format!(
                " (left out of the cost comparison: {})",
                excluded.join("; ")
            );
        }
        if let Some(b) = better {
            let (p, c) = (&pool[b], &pool[cheapest]);
            reasons.push(format!(
                "cost-aware: {} costs {} more than the cheapest ({}) but its success rate is {} bps against {} bps (premium allowed {} bps)",
                p.c.provider,
                p.fee - c.fee,
                c.c.provider,
                rate(p).unwrap_or(0),
                cr,
                policy.premium_bps
            ));
            return (
                b,
                format!(
                    "has a better recent success rate ({} against {} for {}) and costs {} more ({} against {}), within the {} extra you allow for reliability{left_out}",
                    pct(rate(p).unwrap_or(0)),
                    pct(cr),
                    c.c.provider,
                    money(p.fee - c.fee),
                    cost_text(p, money),
                    money(c.fee),
                    pct(policy.premium_bps),
                ),
            );
        }
        reasons.push(format!(
            "cost-aware: lowest cost among providers at or above the {floor} bps success floor"
        ));
        let c = &pool[cheapest];
        let basis = if good.len() > 1 {
            format!(
                "has the lowest estimated cost ({}) among providers with a success rate of at least {}{left_out}",
                cost_text(c, money),
                pct(floor)
            )
        } else {
            format!(
                "is the only provider with enough recent attempts and a success rate of at least {} (estimated cost {}){left_out}",
                pct(floor),
                cost_text(c, money)
            )
        };
        return (cheapest, basis);
    }
    if !unknown.is_empty() {
        let i = cheapest_of(&unknown);
        reasons.push("cost-aware: no provider has enough recent attempts above the success floor; lowest cost among those without data".to_string());
        return (
            i,
            format!(
                "has the lowest estimated cost ({}); no provider has enough recent attempts to show a success rate above your {} floor, so cost decided",
                cost_text(&pool[i], money),
                pct(floor)
            ),
        );
    }
    // every provider has data and every one is below the floor: reliability, not price
    let best = *below
        .iter()
        .max_by(|&&a, &&b| {
            rate(&pool[a])
                .cmp(&rate(&pool[b]))
                .then_with(|| pool[b].fee.cmp(&pool[a].fee))
                .then_with(|| pool[b].c.provider.cmp(&pool[a].c.provider))
        })
        .expect("non-empty");
    reasons.push(
        "cost-aware: every provider is below the success floor; best success rate wins, not the cheapest"
            .to_string(),
    );
    (
        best,
        format!(
            "has the best recent success rate ({}); every provider is below your {} floor, so cost was not used to choose",
            pct(rate(&pool[best]).unwrap_or(0)),
            pct(floor)
        ),
    )
}

pub fn choose(
    req: &Request,
    candidates: &[Candidate],
    rules: &[Rule],
) -> Result<Decision, NoRoute> {
    let active: Vec<&Rule> = rules.iter().filter(|r| r.matches(req)).collect();
    let mut considered = Vec::new();
    let mut eligible: Vec<Elig> = Vec::new();

    for c in candidates {
        let hard = if !c.enabled {
            Some("provider disabled".to_string())
        } else if req
            .allowed
            .as_ref()
            .is_some_and(|a| !a.contains(&c.provider))
        {
            Some("not allowed for this payment".to_string())
        } else if let Err(e) = c.caps.supports(req.method, req.currency) {
            Some(e)
        } else if req.method == Method::Upi && req.upi_intent && !c.caps.upi_intent {
            Some("does not issue a UPI QR or intent".to_string())
        } else if active.iter().any(|r| r.exclude.contains(&c.provider)) {
            Some("excluded by routing rule".to_string())
        } else {
            None
        };
        match hard {
            Some(reason) => {
                let explanation = format!("Not considered: {reason}.");
                considered.push(note(c, "rejected", reason, explanation, None))
            }
            None => {
                let (fee, basis) = match (req.cost.as_ref(), c.cost) {
                    (Some(_), Some(cost)) => (cost.minor, Some(cost.basis)),
                    _ => (
                        c.fees
                            .get(&req.method)
                            .copied()
                            .unwrap_or_default()
                            .estimate(req.amount_minor),
                        None,
                    ),
                };
                eligible.push(Elig { c, fee, basis });
            }
        }
    }

    if eligible.is_empty() {
        let why = considered
            .iter()
            .map(|c| format!("{}: {}", c.provider, c.reason))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(NoRoute(if why.is_empty() {
            "no providers configured".into()
        } else {
            why
        }));
    }

    let money = |minor: i64| {
        format!(
            "{} {}",
            req.currency.code(),
            zyvor_core::minor_to_decimal(minor, req.currency)
        )
    };
    let mut reasons = Vec::new();
    let mut failover_from = Vec::new();
    // Providers set aside before the final choice, each with the cause: they explain "the only provider left".
    let mut skipped: Vec<String> = Vec::new();
    let mut caveat = String::new();
    let mut pool = eligible.clone();

    // Failover: skip what already failed this payment, unless nothing else is eligible.
    let avoided = |name: &str| req.avoid.iter().find(|a| a.provider == name);
    let fresh: Vec<Elig> = pool
        .iter()
        .filter(|e| avoided(&e.c.provider).is_none())
        .cloned()
        .collect();
    if fresh.is_empty() {
        if pool.iter().any(|e| avoided(&e.c.provider).is_some()) {
            reasons.push(
                "no other eligible provider: trying the one that failed this payment again"
                    .to_string(),
            );
            caveat = " It failed this payment before, but no other provider can take it.".into();
        }
    } else if fresh.len() < pool.len() {
        for e in &pool {
            if let Some(a) = avoided(&e.c.provider) {
                failover_from.push(e.c.provider.clone());
                skipped.push(format!("{} (it already failed this payment)", e.c.provider));
                considered.push(note(
                    e.c,
                    "skipped_failover",
                    format!(
                        "failed this payment already ({}); trying another provider",
                        a.reason
                    ),
                    "Skipped because it already failed this payment; another provider is tried instead."
                        .to_string(),
                    Some(e.fee),
                ));
            }
        }
        reasons.push(format!(
            "failover: {} failed this payment definitively, using another provider",
            failover_from.join(", ")
        ));
        pool = fresh;
    }

    // Circuit breaker, half-open: one new payment may test a degraded provider once its cool-down is over.
    if let Some(probe) = pool
        .iter()
        .filter(|e| breaker(e.c) == Breaker::ProbeDue)
        .min_by(|a, b| a.c.provider.cmp(&b.c.provider))
        .cloned()
    {
        for e in &pool {
            if e.c.provider == probe.c.provider {
                continue;
            }
            if matches!(breaker(e.c), Breaker::Degraded | Breaker::ProbeDue) {
                considered.push(note(
                    e.c,
                    "skipped_degraded",
                    "degraded by the circuit breaker".to_string(),
                    format!(
                        "Skipped because the circuit breaker has it degraded{}.",
                        degraded_detail(e.c)
                    ),
                    Some(e.fee),
                ));
            } else {
                considered.push(note(
                    e.c,
                    "not_chosen",
                    format!("{} is being probed after its cool-down", probe.c.provider),
                    format!(
                        "Not chosen: this payment tests {} after its cool-down.",
                        probe.c.provider
                    ),
                    Some(e.fee),
                ));
            }
        }
        reasons.push(format!(
            "circuit breaker probe: {} was degraded and its cool-down is over; this payment tests it",
            probe.c.provider
        ));
        let explanation = format!(
            "Chosen as a test: {} was degraded, its cool-down is over, and this payment checks whether it works again.",
            probe.c.provider
        );
        considered.push(note(
            probe.c,
            "probe",
            reasons.join("; "),
            explanation.clone(),
            Some(probe.fee),
        ));
        return Ok(Decision {
            provider: probe.c.provider.clone(),
            explanation,
            reasons,
            considered,
            failover_from,
            probe: Some(probe.c.provider.clone()),
        });
    }

    // Circuit breaker, open: degraded providers are skipped for new payments. This is the only health rule.
    let live: Vec<Elig> = pool
        .iter()
        .filter(|e| breaker(e.c) != Breaker::Degraded)
        .cloned()
        .collect();
    if live.is_empty() {
        reasons.push(
            "every eligible provider is degraded by the circuit breaker; choosing among them anyway"
                .to_string(),
        );
        caveat.push_str(
            " Every eligible provider is degraded, so the circuit breaker could not skip any.",
        );
    } else if live.len() < pool.len() {
        let names: Vec<_> = pool
            .iter()
            .filter(|e| breaker(e.c) == Breaker::Degraded)
            .map(|e| {
                considered.push(note(
                    e.c,
                    "skipped_degraded",
                    format!(
                        "degraded: recent failure rate crossed the threshold ({})",
                        e.c.signal
                            .as_ref()
                            .map(|s| format!(
                                "{} failed or unresolved of {}",
                                s.failures + s.unresolved,
                                s.samples
                            ))
                            .unwrap_or_default()
                    ),
                    format!(
                        "Skipped because the circuit breaker has it degraded{}.",
                        degraded_detail(e.c)
                    ),
                    Some(e.fee),
                ));
                skipped.push(format!(
                    "{} (degraded by the circuit breaker)",
                    e.c.provider
                ));
                e.c.provider.clone()
            })
            .collect();
        reasons.push(format!("skipped degraded: {}", names.join(", ")));
        pool = live;
    }

    let preferred = active
        .iter()
        .flat_map(|r| r.prefer.iter())
        .find_map(|p| pool.iter().find(|e| &e.c.provider == p));
    let by_rate = active.iter().any(|r| r.prefer_success_rate);
    let margin = active
        .iter()
        .filter(|r| r.prefer_success_rate)
        .filter_map(|r| r.success_rate_margin_bps)
        .min()
        .unwrap_or(DEFAULT_SUCCESS_RATE_MARGIN_BPS);
    let rate = |e: &Elig| e.c.signal.as_ref().and_then(|s| s.success_rate_bps);

    // `basis` completes both "Chosen because it ..." and "..., which ...".
    let (chosen, basis): (&Elig, String) = match preferred {
        Some(p) => {
            reasons.push(format!("routing rule prefers {}", p.c.provider));
            (p, "is preferred by your routing rule".to_string())
        }
        None => {
            let mut by_success: Option<(&Elig, String)> = None;
            if by_rate && pool.len() > 1 {
                if pool.iter().all(|e| rate(e).is_some()) {
                    let mut ranked: Vec<&Elig> = pool.iter().collect();
                    ranked.sort_by(|a, b| {
                        rate(b)
                            .cmp(&rate(a))
                            .then_with(|| a.fee.cmp(&b.fee))
                            .then_with(|| a.c.provider.cmp(&b.c.provider))
                    });
                    let (top, next) = (ranked[0], ranked[1]);
                    let (tr, nr) = (rate(top).unwrap_or(0), rate(next).unwrap_or(0));
                    if tr >= nr + margin {
                        reasons.push(format!(
                            "best recent success rate: {} {} bps vs {} {} bps (margin {margin})",
                            top.c.provider, tr, next.c.provider, nr
                        ));
                        by_success = Some((
                            top,
                            format!(
                                "has the best recent success rate ({} against {} for {})",
                                pct(tr),
                                pct(nr),
                                next.c.provider
                            ),
                        ));
                    } else {
                        reasons.push(format!(
                            "success rates within {margin} bps of each other; fell back to fee"
                        ));
                    }
                } else {
                    let missing: Vec<_> = pool
                        .iter()
                        .filter(|e| rate(e).is_none())
                        .map(|e| e.c.provider.as_str())
                        .collect();
                    reasons.push(format!(
                        "not enough recent attempts for {} to compare success rates; fell back to fee",
                        missing.join(", ")
                    ));
                }
            }
            let by_cost = match (by_success.is_none(), req.cost.as_ref()) {
                (true, Some(policy)) if pool.len() > 1 => {
                    let (i, basis) = cost_choice(&pool, policy, &money, &mut reasons);
                    Some((&pool[i], basis))
                }
                _ => None,
            };
            match by_success.or(by_cost) {
                Some(x) => x,
                None => {
                    let best = pool
                        .iter()
                        .min_by(|a, b| {
                            a.fee
                                .cmp(&b.fee)
                                .then_with(|| a.c.provider.cmp(&b.c.provider))
                        })
                        .expect("pool non-empty");
                    if pool.len() > 1 {
                        reasons.push(format!("lowest estimated fee ({} minor units)", best.fee));
                        (
                            best,
                            format!("has the lowest estimated fee ({})", money(best.fee)),
                        )
                    } else if skipped.is_empty() {
                        reasons.push("only eligible provider".to_string());
                        (
                            best,
                            "is the only provider that can take this payment".to_string(),
                        )
                    } else {
                        // The pool was narrowed before the choice: say what was set aside, not "only eligible".
                        let why = skipped.join(", ");
                        reasons.push(format!("only provider left after skipping {why}"));
                        (
                            best,
                            format!("is the only provider left after skipping {why}"),
                        )
                    }
                }
            }
        }
    };
    for e in &pool {
        if e.c.provider != chosen.c.provider {
            considered.push(note(
                e.c,
                "not_chosen",
                format!("estimated fee {}; {} preferred", e.fee, chosen.c.provider),
                format!(
                    "Not chosen: {} was used instead, which {basis}.",
                    chosen.c.provider
                ),
                Some(e.fee),
            ));
        }
    }
    let explanation = format!("Chosen because it {basis}.{caveat}");
    considered.push(note(
        chosen.c,
        "selected",
        reasons.join("; "),
        explanation.clone(),
        Some(chosen.fee),
    ));
    Ok(Decision {
        provider: chosen.c.provider.clone(),
        explanation,
        reasons,
        considered,
        failover_from,
        probe: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(methods: &[Method], currencies: &[Currency]) -> Capabilities {
        Capabilities {
            methods: methods.to_vec(),
            currencies: currencies.to_vec(),
            partial_refund: true,
            manual_capture: false,
            server_initiated_upi: false,
            upi_intent: false,
            mandates: false,
            refund_window_days: None,
        }
    }
    fn cand(name: &str, methods: &[Method], bps: u32) -> Candidate {
        Candidate {
            provider: name.into(),
            caps: caps(methods, &[Currency::Inr]),
            enabled: true,
            signal: None,
            cost: None,
            fees: methods
                .iter()
                .map(|m| {
                    (
                        *m,
                        Fee {
                            bps,
                            fixed_minor: 0,
                        },
                    )
                })
                .collect(),
        }
    }
    fn req(method: Method, amount: i64) -> Request {
        Request {
            method,
            amount_minor: amount,
            currency: Currency::Inr,
            allowed: None,
            upi_intent: false,
            avoid: vec![],
            cost: None,
        }
    }

    #[test]
    fn capability_matrix_blocks_unsupported_methods_and_currencies() {
        let c = [
            cand("a", &[Method::Upi], 100),
            cand("b", &[Method::Card], 50),
        ];
        let d = choose(&req(Method::Card, 10_000), &c, &[]).unwrap();
        assert_eq!(d.provider, "b");
        assert!(
            d.considered
                .iter()
                .any(|x| x.provider == "a" && x.outcome == "rejected" && x.reason.contains("card"))
        );
        assert!(choose(&req(Method::Wallet, 1), &c, &[]).is_err());
        let usd = Request {
            currency: Currency::Usd,
            ..req(Method::Upi, 1)
        };
        assert!(choose(&usd, &c, &[]).unwrap_err().0.contains("USD"));
    }

    #[test]
    fn picks_lowest_fee_and_records_why() {
        let c = [
            cand("a", &[Method::Upi], 100),
            cand("b", &[Method::Upi], 30),
        ];
        let d = choose(&req(Method::Upi, 100_000), &c, &[]).unwrap();
        assert_eq!(d.provider, "b");
        assert!(d.reasons[0].contains("lowest estimated fee"));
        assert_eq!(d.considered.len(), 2);
    }

    #[test]
    fn rules_prefer_but_cannot_override_hard_filters() {
        let c = [
            cand("a", &[Method::Upi], 100),
            cand("b", &[Method::Upi], 30),
        ];
        let prefer_a = Rule {
            method: Some(Method::Upi),
            prefer: vec!["a".into()],
            ..Default::default()
        };
        assert_eq!(
            choose(
                &req(Method::Upi, 1_000),
                &c,
                std::slice::from_ref(&prefer_a)
            )
            .unwrap()
            .provider,
            "a"
        );
        // preference for a provider that cannot do the method is ignored
        let c2 = [
            cand("a", &[Method::Card], 100),
            cand("b", &[Method::Upi], 30),
        ];
        assert_eq!(
            choose(&req(Method::Upi, 1_000), &c2, &[prefer_a])
                .unwrap()
                .provider,
            "b"
        );
        // amount-bounded rule only applies inside its band
        let big = Rule {
            min_amount_minor: Some(500_000),
            prefer: vec!["a".into()],
            ..Default::default()
        };
        assert_eq!(
            choose(&req(Method::Upi, 1_000), &c, std::slice::from_ref(&big))
                .unwrap()
                .provider,
            "b"
        );
        assert_eq!(
            choose(&req(Method::Upi, 600_000), &c, &[big])
                .unwrap()
                .provider,
            "a"
        );
        // exclusion is hard
        let ex = Rule {
            exclude: vec!["b".into()],
            ..Default::default()
        };
        assert_eq!(
            choose(&req(Method::Upi, 1_000), &c, &[ex])
                .unwrap()
                .provider,
            "a"
        );
    }

    #[test]
    fn every_decision_explains_itself_in_a_plain_sentence() {
        let c = [
            cand("a", &[Method::Upi], 100),
            cand("b", &[Method::Upi], 30),
            cand("c", &[Method::Card], 30),
        ];
        let d = choose(&req(Method::Upi, 100_000), &c, &[]).unwrap();
        assert!(d.considered.iter().all(|x| !x.explanation.is_empty()));
        let by = |name: &str| d.considered.iter().find(|x| x.provider == name).unwrap();
        assert!(
            by("b")
                .explanation
                .starts_with("Chosen because it has the lowest estimated fee (INR 3.00)"),
            "{}",
            by("b").explanation
        );
        assert_eq!(d.explanation, by("b").explanation);
        assert!(
            by("a")
                .explanation
                .starts_with("Not chosen: b was used instead")
        );
        assert!(by("c").explanation.starts_with("Not considered:"));
    }

    #[test]
    fn the_only_provider_left_names_what_was_set_aside_instead_of_claiming_to_be_the_only_one() {
        // genuinely the only provider
        let d = choose(
            &req(Method::Upi, 1_000),
            &[cand("a", &[Method::Upi], 10)],
            &[],
        )
        .unwrap();
        assert!(d.reasons.contains(&"only eligible provider".to_string()));
        // two eligible, one degraded by the breaker: not "only eligible"
        let bad = with_signal(cand("a", &[Method::Upi], 10), 0, 20, 10, Breaker::Degraded);
        let ok = cand("b", &[Method::Upi], 90);
        let d = choose(&req(Method::Upi, 1_000), &[bad, ok.clone()], &[]).unwrap();
        assert_eq!(d.provider, "b");
        assert!(!d.reasons.iter().any(|r| r.contains("only eligible")));
        assert!(
            d.reasons.iter().any(|r| r
                .contains("only provider left after skipping a (degraded by the circuit breaker)")),
            "{:?}",
            d.reasons
        );
        let a = d.considered.iter().find(|x| x.provider == "a").unwrap();
        assert_eq!(a.outcome, "skipped_degraded");
        assert!(a.explanation.contains("20 of its last 20 payments failed"));
        // failover is named the same way
        let r = Request {
            avoid: vec![Avoid {
                provider: "a".into(),
                reason: "declined".into(),
            }],
            ..req(Method::Upi, 1_000)
        };
        let d = choose(&r, &[cand("a", &[Method::Upi], 10), ok], &[]).unwrap();
        assert!(
            d.explanation
                .contains("only provider left after skipping a (it already failed this payment)")
        );
    }

    #[test]
    fn unresolved_payments_no_longer_reorder_anything_the_breaker_is_the_only_health_rule() {
        // a provider with unresolved attempts but a healthy breaker is still the cheapest choice
        let mut stuck = with_signal(cand("a", &[Method::Upi], 10), 8, 0, 10, Breaker::Healthy);
        if let Some(s) = stuck.signal.as_mut() {
            s.unresolved = 1;
        }
        let d = choose(
            &req(Method::Upi, 10_000),
            &[stuck, cand("b", &[Method::Upi], 90)],
            &[],
        )
        .unwrap();
        assert_eq!(d.provider, "a");
        assert!(!d.considered.iter().any(|c| c.outcome == "deprioritised"));
    }

    #[test]
    fn a_probe_and_an_all_degraded_pool_are_explained() {
        let due = with_signal(cand("a", &[Method::Upi], 10), 0, 20, 10, Breaker::ProbeDue);
        let d = choose(
            &req(Method::Upi, 1_000),
            &[due, cand("b", &[Method::Upi], 90)],
            &[],
        )
        .unwrap();
        assert_eq!(d.probe.as_deref(), Some("a"));
        assert!(
            d.explanation
                .starts_with("Chosen as a test: a was degraded")
        );
        let b = d.considered.iter().find(|x| x.provider == "b").unwrap();
        assert!(
            b.explanation
                .starts_with("Not chosen: this payment tests a")
        );

        let x = with_signal(cand("a", &[Method::Upi], 10), 0, 20, 10, Breaker::Degraded);
        let y = with_signal(cand("b", &[Method::Upi], 90), 0, 20, 10, Breaker::Degraded);
        let d = choose(&req(Method::Upi, 1_000), &[x, y], &[]).unwrap();
        assert!(
            d.explanation
                .contains("Every eligible provider is degraded")
        );
    }

    #[test]
    fn disabled_and_allowlist_are_hard() {
        let mut off = cand("a", &[Method::Upi], 1);
        off.enabled = false;
        let c = [
            off,
            cand("b", &[Method::Upi], 50),
            cand("c", &[Method::Upi], 5),
        ];
        let r = Request {
            allowed: Some(vec!["b".into()]),
            ..req(Method::Upi, 1_000)
        };
        assert_eq!(choose(&r, &c, &[]).unwrap().provider, "b");
    }

    fn with_signal(
        mut c: Candidate,
        successes: u32,
        failures: u32,
        min: u32,
        b: Breaker,
    ) -> Candidate {
        let samples = successes + failures;
        c.signal = Some(Signal {
            samples,
            successes,
            failures,
            payer_declines: 0,
            unresolved: 0,
            success_rate_bps: (samples >= min && samples > 0).then(|| successes * 10_000 / samples),
            min_samples: min,
            breaker: b,
        });
        c
    }
    fn rate_rule() -> Rule {
        Rule {
            prefer_success_rate: true,
            ..Default::default()
        }
    }

    #[test]
    fn better_success_rate_beats_a_cheaper_provider_only_with_enough_data_and_margin() {
        let cheap = with_signal(
            cand("cheap", &[Method::Upi], 10),
            80,
            20,
            10,
            Breaker::Healthy,
        );
        let dear = with_signal(
            cand("dear", &[Method::Upi], 100),
            97,
            3,
            10,
            Breaker::Healthy,
        );
        let r = [rate_rule()];
        let d = choose(
            &req(Method::Upi, 10_000),
            &[cheap.clone(), dear.clone()],
            &r,
        )
        .unwrap();
        assert_eq!(d.provider, "dear");
        assert!(
            d.reasons
                .iter()
                .any(|x| x.contains("best recent success rate"))
        );
        // every considered provider carries the signal it was judged on
        assert!(d.considered.iter().all(|c| c.signal.is_some()));

        // without the rule fee still decides
        assert_eq!(
            choose(
                &req(Method::Upi, 10_000),
                &[cheap.clone(), dear.clone()],
                &[]
            )
            .unwrap()
            .provider,
            "cheap"
        );

        // a tiny sample never flips routing
        let few = with_signal(
            cand("dear", &[Method::Upi], 100),
            3,
            0,
            10,
            Breaker::Healthy,
        );
        let d = choose(&req(Method::Upi, 10_000), &[cheap.clone(), few], &r).unwrap();
        assert_eq!(d.provider, "cheap");
        assert!(
            d.reasons
                .iter()
                .any(|x| x.contains("not enough recent attempts"))
        );

        // a lead inside the margin is noise
        let close = with_signal(
            cand("dear", &[Method::Upi], 100),
            81,
            19,
            10,
            Breaker::Healthy,
        );
        let d = choose(&req(Method::Upi, 10_000), &[cheap, close], &r).unwrap();
        assert_eq!(d.provider, "cheap");
        assert!(d.reasons.iter().any(|x| x.contains("within")));
    }

    #[test]
    fn an_explicit_prefer_list_still_beats_the_success_rate_option() {
        let a = with_signal(cand("a", &[Method::Upi], 10), 50, 50, 10, Breaker::Healthy);
        let b = with_signal(cand("b", &[Method::Upi], 10), 99, 1, 10, Breaker::Healthy);
        let r = Rule {
            prefer: vec!["a".into()],
            prefer_success_rate: true,
            ..Default::default()
        };
        assert_eq!(
            choose(&req(Method::Upi, 1_000), &[a, b], &[r])
                .unwrap()
                .provider,
            "a"
        );
    }

    #[test]
    fn a_degraded_provider_is_skipped_for_new_payments_and_a_probe_is_marked() {
        let bad = with_signal(
            cand("cheap", &[Method::Upi], 10),
            2,
            18,
            10,
            Breaker::Degraded,
        );
        let ok = cand("dear", &[Method::Upi], 100);
        let d = choose(&req(Method::Upi, 10_000), &[bad.clone(), ok.clone()], &[]).unwrap();
        assert_eq!(d.provider, "dear");
        let skipped = d.considered.iter().find(|c| c.provider == "cheap").unwrap();
        assert_eq!(skipped.outcome, "skipped_degraded");
        assert!(skipped.signal.is_some());
        assert!(d.probe.is_none());

        // only option: still used (a signal never leaves a business unable to take payments), and it says so
        let d = choose(&req(Method::Upi, 10_000), std::slice::from_ref(&bad), &[]).unwrap();
        assert_eq!(d.provider, "cheap");
        assert!(
            d.reasons
                .iter()
                .any(|r| r.contains("every eligible provider is degraded"))
        );

        // cool-down over: exactly one payment probes it, and the decision says so
        let due = with_signal(
            cand("cheap", &[Method::Upi], 10),
            2,
            18,
            10,
            Breaker::ProbeDue,
        );
        let d = choose(&req(Method::Upi, 10_000), &[due, ok], &[]).unwrap();
        assert_eq!(d.provider, "cheap");
        assert_eq!(d.probe.as_deref(), Some("cheap"));
        assert!(d.considered.iter().any(|c| c.outcome == "probe"));
    }

    #[test]
    fn failover_skips_what_failed_this_payment_but_never_leaves_no_route() {
        let a = cand("a", &[Method::Upi], 10);
        let b = cand("b", &[Method::Upi], 90);
        let mut r = req(Method::Upi, 10_000);
        r.avoid = vec![Avoid {
            provider: "a".into(),
            reason: "provider_rejected".into(),
        }];
        let d = choose(&r, &[a.clone(), b.clone()], &[]).unwrap();
        assert_eq!(d.provider, "b");
        assert_eq!(d.failover_from, vec!["a".to_string()]);
        assert!(
            d.considered
                .iter()
                .any(|c| c.provider == "a" && c.outcome == "skipped_failover")
        );
        // nothing else eligible: the same provider is tried again, as a plain retry always was
        let d = choose(&r, std::slice::from_ref(&a), &[]).unwrap();
        assert_eq!(d.provider, "a");
        assert!(d.failover_from.is_empty());
        // a hard filter still wins over failover: b is excluded, so a is all that is left
        let ex = Rule {
            exclude: vec!["b".into()],
            ..Default::default()
        };
        assert_eq!(choose(&r, &[a, b], &[ex]).unwrap().provider, "a");
    }

    #[test]
    fn fee_estimate_is_integer_and_rounds_half_up() {
        assert_eq!(
            Fee {
                bps: 200,
                fixed_minor: 0
            }
            .estimate(49_900),
            998
        );
        assert_eq!(
            Fee {
                bps: 25,
                fixed_minor: 100
            }
            .estimate(1_000),
            103
        );
        assert_eq!(
            Fee {
                bps: 150,
                fixed_minor: 0
            }
            .estimate(1),
            0
        );
    }

    // ---- cost-aware routing ----

    fn priced(mut c: Candidate, minor: i64, basis: &'static str) -> Candidate {
        c.cost = Some(Cost { minor, basis });
        c
    }
    fn cost_req(floor_bps: u32, premium_bps: u32) -> Request {
        Request {
            cost: Some(CostPolicy {
                floor_bps,
                premium_bps,
            }),
            ..req(Method::Upi, 100_000)
        }
    }

    #[test]
    fn cost_awareness_is_off_unless_the_request_asks_for_it() {
        // `cost` data on the candidates is ignored without a policy: the configured fee (a: 100 bps, b: 30 bps) decides
        let c = [
            priced(cand("a", &[Method::Upi], 100), 1, "observed"),
            priced(cand("b", &[Method::Upi], 30), 999_999, "observed"),
        ];
        assert_eq!(
            choose(&req(Method::Upi, 100_000), &c, &[])
                .unwrap()
                .provider,
            "b"
        );
    }

    #[test]
    fn a_cheaper_provider_below_the_success_floor_is_never_chosen_for_being_cheaper() {
        let cheap = with_signal(
            priced(cand("cheap", &[Method::Upi], 10), 100, "fee_rule"),
            60,
            40,
            10,
            Breaker::Healthy,
        );
        let sound = with_signal(
            priced(cand("sound", &[Method::Upi], 10), 900, "fee_rule"),
            95,
            5,
            10,
            Breaker::Healthy,
        );
        // 60% against a floor of 80%: excluded from the cost comparison, said so in words
        let d = choose(&cost_req(8_000, 500), &[cheap.clone(), sound.clone()], &[]).unwrap();
        assert_eq!(d.provider, "sound", "{:?}", d.reasons);
        assert!(
            d.explanation
                .contains("only provider with enough recent attempts"),
            "{}",
            d.explanation
        );
        assert!(
            d.explanation.contains("cheap (cheaper at INR 1.00"),
            "{}",
            d.explanation
        );
        assert!(
            d.explanation.contains("below your 80.0% floor"),
            "{}",
            d.explanation
        );
        // with a floor both clear, price decides
        let d = choose(&cost_req(5_000, 500), &[cheap, sound], &[]).unwrap();
        assert_eq!(d.provider, "cheap");
        assert!(
            d.explanation
                .contains("lowest estimated cost (INR 1.00, from your fee rule)"),
            "{}",
            d.explanation
        );
    }

    #[test]
    fn a_provider_without_enough_samples_cannot_win_on_cost_while_a_proven_one_exists() {
        let new = priced(cand("new", &[Method::Upi], 10), 50, "configured"); // no signal at all
        let proven = with_signal(
            priced(cand("proven", &[Method::Upi], 10), 500, "observed"),
            90,
            10,
            10,
            Breaker::Healthy,
        );
        let few = with_signal(
            priced(cand("few", &[Method::Upi], 10), 20, "observed"),
            2,
            0,
            10, // 2 samples < 10: no success rate
            Breaker::Healthy,
        );
        let d = choose(&cost_req(8_000, 500), &[new, few, proven], &[]).unwrap();
        assert_eq!(d.provider, "proven");
        assert!(
            d.explanation.contains("too few recent attempts"),
            "{}",
            d.explanation
        );
    }

    #[test]
    fn with_no_success_data_at_all_cost_decides_and_says_so() {
        let c = [
            priced(cand("a", &[Method::Upi], 10), 300, "observed"),
            priced(cand("b", &[Method::Upi], 10), 200, "fee_rule"),
        ];
        let d = choose(&cost_req(8_000, 500), &c, &[]).unwrap();
        assert_eq!(d.provider, "b");
        assert!(
            d.explanation
                .contains("no provider has enough recent attempts"),
            "{}",
            d.explanation
        );
    }

    #[test]
    fn when_everyone_is_below_the_floor_reliability_wins_not_price() {
        let a = with_signal(
            priced(cand("a", &[Method::Upi], 10), 100, "observed"),
            50,
            50,
            10,
            Breaker::Healthy,
        );
        let b = with_signal(
            priced(cand("b", &[Method::Upi], 10), 900, "observed"),
            70,
            30,
            10,
            Breaker::Healthy,
        );
        let d = choose(&cost_req(9_000, 500), &[a, b], &[]).unwrap();
        assert_eq!(d.provider, "b");
        assert!(
            d.explanation
                .contains("every provider is below your 90.0% floor"),
            "{}",
            d.explanation
        );
    }

    #[test]
    fn a_clearly_more_reliable_provider_wins_if_it_costs_only_a_little_more_and_the_sentence_says_so()
     {
        let cheap = with_signal(
            priced(cand("cheap", &[Method::Upi], 10), 1_000, "observed"),
            88,
            12,
            10,
            Breaker::Healthy,
        );
        let steady = with_signal(
            priced(cand("steady", &[Method::Upi], 10), 1_040, "observed"),
            97,
            3,
            10,
            Breaker::Healthy,
        );
        // 4% more cost, premium allowed 5%, success 97.0% vs 88.0% (margin 2%): the traded-off sentence
        let d = choose(&cost_req(8_000, 500), &[cheap.clone(), steady.clone()], &[]).unwrap();
        assert_eq!(d.provider, "steady");
        assert!(
            d.explanation.contains(
                "better recent success rate (97.0% against 88.0% for cheap) and costs INR 0.40 more"
            ),
            "{}",
            d.explanation
        );
        // premium allowed only 3%: the extra 4% is too much, the cheaper provider keeps it
        let d = choose(&cost_req(8_000, 300), &[cheap, steady], &[]).unwrap();
        assert_eq!(d.provider, "cheap");
    }

    #[test]
    fn cost_never_overrides_hard_filters_failover_the_breaker_or_a_preference() {
        let a = with_signal(
            priced(cand("a", &[Method::Upi], 10), 100, "observed"),
            99,
            1,
            10,
            Breaker::Degraded,
        );
        let b = with_signal(
            priced(cand("b", &[Method::Upi], 10), 900, "observed"),
            90,
            10,
            10,
            Breaker::Healthy,
        );
        let c = with_signal(
            priced(cand("c", &[Method::Upi], 10), 500, "observed"),
            90,
            10,
            10,
            Breaker::Healthy,
        );
        // a is degraded: skipped however cheap
        assert_eq!(
            choose(
                &cost_req(8_000, 500),
                &[a.clone(), b.clone(), c.clone()],
                &[]
            )
            .unwrap()
            .provider,
            "c"
        );
        // c already failed this payment: skipped
        let r = Request {
            avoid: vec![Avoid {
                provider: "c".into(),
                reason: "declined".into(),
            }],
            ..cost_req(8_000, 500)
        };
        assert_eq!(
            choose(&r, &[a.clone(), b.clone(), c.clone()], &[])
                .unwrap()
                .provider,
            "b"
        );
        // a routing-rule preference still wins
        let prefer_b = Rule {
            prefer: vec!["b".into()],
            ..Default::default()
        };
        assert_eq!(
            choose(&cost_req(8_000, 500), &[b.clone(), c.clone()], &[prefer_b])
                .unwrap()
                .provider,
            "b"
        );
        // excluded by rule: never chosen
        let ex = Rule {
            exclude: vec!["c".into()],
            ..Default::default()
        };
        assert_eq!(
            choose(&cost_req(8_000, 500), &[b, c], &[ex])
                .unwrap()
                .provider,
            "b"
        );
    }
}
