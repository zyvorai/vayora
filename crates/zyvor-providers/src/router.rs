//! Safe, explainable routing.
//!
//! Hard filters (disabled, not allowed, capability mismatch, rule exclusion) can never be
//! overridden. Soft signals (health, unresolved payments) only reorder. Every decision records
//! which providers were considered and why each was chosen or rejected.

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Health {
    pub recent_attempts: u32,
    pub recent_failures: u32,
    /// Attempts stuck in an unknown outcome. Any at all deprioritises the provider.
    pub unresolved: u32,
}

impl Health {
    fn unhealthy(&self) -> Option<String> {
        if self.unresolved > 0 {
            return Some(format!("{} unresolved payment(s)", self.unresolved));
        }
        if self.recent_attempts >= 10 && self.recent_failures * 2 > self.recent_attempts {
            return Some(format!(
                "{}/{} recent attempts failed",
                self.recent_failures, self.recent_attempts
            ));
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub provider: String,
    pub caps: Capabilities,
    pub enabled: bool,
    pub health: Health,
    /// Configured fee per method.
    pub fees: BTreeMap<Method, Fee>,
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
}

impl Rule {
    fn matches(&self, req: &Request) -> bool {
        self.method.is_none_or(|m| m == req.method)
            && self.min_amount_minor.is_none_or(|v| req.amount_minor >= v)
            && self.max_amount_minor.is_none_or(|v| req.amount_minor <= v)
    }
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
}

#[derive(Debug, Clone, Serialize)]
pub struct Considered {
    pub provider: String,
    pub outcome: &'static str,
    pub reason: String,
    pub estimated_fee_minor: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub provider: String,
    pub reasons: Vec<String>,
    pub considered: Vec<Considered>,
}

#[derive(Debug, thiserror::Error)]
#[error("no provider can handle this payment: {0}")]
pub struct NoRoute(pub String);

pub fn choose(
    req: &Request,
    candidates: &[Candidate],
    rules: &[Rule],
) -> Result<Decision, NoRoute> {
    let active: Vec<&Rule> = rules.iter().filter(|r| r.matches(req)).collect();
    let mut considered = Vec::new();
    let mut eligible: Vec<(&Candidate, i64, Option<String>)> = Vec::new();

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
            Some(reason) => considered.push(Considered {
                provider: c.provider.clone(),
                outcome: "rejected",
                reason,
                estimated_fee_minor: None,
            }),
            None => {
                let fee = c
                    .fees
                    .get(&req.method)
                    .copied()
                    .unwrap_or_default()
                    .estimate(req.amount_minor);
                eligible.push((c, fee, c.health.unhealthy()));
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

    let mut reasons = Vec::new();
    let healthy: Vec<_> = eligible
        .iter()
        .filter(|(_, _, h)| h.is_none())
        .cloned()
        .collect();
    let pool = if healthy.is_empty() {
        reasons.push("all eligible providers are degraded; choosing among them anyway".to_string());
        eligible.clone()
    } else {
        healthy
    };
    for (c, fee, h) in &eligible {
        if let Some(h) = h
            && pool.iter().all(|(p, _, _)| p.provider != c.provider)
        {
            {
                considered.push(Considered {
                    provider: c.provider.clone(),
                    outcome: "deprioritised",
                    reason: format!("degraded: {h}"),
                    estimated_fee_minor: Some(*fee),
                });
            }
        }
    }

    let preferred = active
        .iter()
        .flat_map(|r| r.prefer.iter())
        .find_map(|p| pool.iter().find(|(c, _, _)| &c.provider == p));
    let (chosen, fee, _) = match preferred {
        Some(p) => {
            reasons.push(format!("routing rule prefers {}", p.0.provider));
            p
        }
        None => {
            let best = pool
                .iter()
                .min_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.provider.cmp(&b.0.provider)))
                .expect("pool non-empty");
            reasons.push(if pool.len() > 1 {
                format!("lowest estimated fee ({} minor units)", best.1)
            } else {
                "only eligible provider".to_string()
            });
            best
        }
    };
    for (c, f, _) in &pool {
        if c.provider != chosen.provider {
            considered.push(Considered {
                provider: c.provider.clone(),
                outcome: "not_chosen",
                reason: format!("estimated fee {f}; {} preferred", chosen.provider),
                estimated_fee_minor: Some(*f),
            });
        }
    }
    considered.push(Considered {
        provider: chosen.provider.clone(),
        outcome: "selected",
        reason: reasons.join("; "),
        estimated_fee_minor: Some(*fee),
    });
    Ok(Decision {
        provider: chosen.provider.clone(),
        reasons,
        considered,
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
            health: Health::default(),
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
    fn unhealthy_providers_are_deprioritised_not_banned() {
        let mut bad = cand("cheap", &[Method::Upi], 10);
        bad.health = Health {
            recent_attempts: 20,
            recent_failures: 15,
            unresolved: 0,
        };
        let ok = cand("dear", &[Method::Upi], 100);
        let d = choose(&req(Method::Upi, 10_000), &[bad.clone(), ok], &[]).unwrap();
        assert_eq!(d.provider, "dear");
        assert!(
            d.considered
                .iter()
                .any(|c| c.provider == "cheap" && c.outcome == "deprioritised")
        );
        // if it's the only option it is still used, with the reason recorded
        let d = choose(&req(Method::Upi, 10_000), &[bad], &[]).unwrap();
        assert_eq!(d.provider, "cheap");
        assert!(d.reasons.iter().any(|r| r.contains("degraded")));
    }

    #[test]
    fn unresolved_payments_deprioritise() {
        let mut stuck = cand("a", &[Method::Upi], 10);
        stuck.health.unresolved = 1;
        let d = choose(
            &req(Method::Upi, 10_000),
            &[stuck, cand("b", &[Method::Upi], 90)],
            &[],
        )
        .unwrap();
        assert_eq!(d.provider, "b");
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
}
