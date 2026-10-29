//! Attempt and refund state transitions. Pure functions, so every rule is unit-tested.
//!
//! Webhooks arrive duplicated and out of order. Rules:
//! * `captured` is the only state that moves money and is never left, except by refunds (tracked separately);
//! * money always wins: a capture is accepted from any non-captured state, even after `failed`
//!   (the customer retried inside the provider's checkout) and is flagged for review;
//! * everything else only moves forward.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Attempt {
    #[default]
    Created,
    Pending,
    Unknown,
    Authorized,
    Captured,
    Failed,
}

impl Attempt {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "created" => Self::Created,
            "pending" => Self::Pending,
            "unknown" => Self::Unknown,
            "authorized" => Self::Authorized,
            "captured" => Self::Captured,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Pending => "pending",
            Self::Unknown => "unknown",
            Self::Authorized => "authorized",
            Self::Captured => "captured",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Transition {
    Apply,
    /// Already in this state; nothing to do.
    NoOp,
    /// Moving there would regress or contradict the current state.
    Ignore,
}

/// `same_payment`: the event refers to the provider payment already recorded on the attempt
/// (or none is recorded yet). A `failed` for a *different* provider payment says nothing about ours.
pub fn attempt_transition(current: Attempt, incoming: Attempt, same_payment: bool) -> Transition {
    use Attempt::*;
    if current == incoming {
        return Transition::NoOp;
    }
    match (current, incoming) {
        (Captured, _) => Transition::Ignore,
        (_, Captured) => Transition::Apply,
        (Authorized, Failed)
        | (Authorized, Pending)
        | (Authorized, Unknown)
        | (Authorized, Created) => Transition::Ignore,
        (Failed, _) => Transition::Ignore,
        (_, Failed) if !same_payment => Transition::Ignore,
        (Created | Pending | Unknown, Failed) => Transition::Apply,
        (Created | Unknown, Pending) => Transition::Apply,
        (Created | Pending | Unknown, Authorized) => Transition::Apply,
        (Created, Unknown) => Transition::Apply,
        (Pending, Unknown) | (Pending, Created) | (Unknown, Created) => Transition::Ignore,
        _ => Transition::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::Attempt::*;
    use super::*;

    #[test]
    fn forward_progress_applies() {
        assert_eq!(
            attempt_transition(Created, Pending, true),
            Transition::Apply
        );
        assert_eq!(
            attempt_transition(Pending, Authorized, true),
            Transition::Apply
        );
        assert_eq!(
            attempt_transition(Authorized, Captured, true),
            Transition::Apply
        );
        assert_eq!(
            attempt_transition(Unknown, Captured, true),
            Transition::Apply
        );
        assert_eq!(attempt_transition(Unknown, Failed, true), Transition::Apply);
        assert_eq!(
            attempt_transition(Created, Unknown, true),
            Transition::Apply,
            "a lost response parks the attempt as unknown"
        );
    }

    #[test]
    fn duplicates_are_noops() {
        for s in [Created, Pending, Unknown, Authorized, Captured, Failed] {
            assert_eq!(attempt_transition(s, s, true), Transition::NoOp);
        }
    }

    #[test]
    fn out_of_order_events_cannot_regress_state() {
        // captured, then a late "authorized" or "pending" or "failed"
        for late in [Created, Pending, Unknown, Authorized, Failed] {
            assert_eq!(
                attempt_transition(Captured, late, true),
                Transition::Ignore,
                "{late:?}"
            );
        }
        assert_eq!(
            attempt_transition(Authorized, Pending, true),
            Transition::Ignore
        );
        assert_eq!(
            attempt_transition(Authorized, Failed, true),
            Transition::Ignore
        );
        assert_eq!(
            attempt_transition(Failed, Pending, true),
            Transition::Ignore
        );
        assert_eq!(
            attempt_transition(Failed, Authorized, true),
            Transition::Ignore
        );
    }

    #[test]
    fn money_wins_over_failure_but_a_foreign_failure_is_ignored() {
        assert_eq!(
            attempt_transition(Failed, Captured, true),
            Transition::Apply,
            "late success after failure"
        );
        assert_eq!(
            attempt_transition(Pending, Failed, false),
            Transition::Ignore,
            "failure of another provider payment"
        );
        assert_eq!(attempt_transition(Pending, Failed, true), Transition::Apply);
    }
}
