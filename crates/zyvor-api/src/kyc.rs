//! Identity and bank checks for onboarding, and the rule that decides whether a submission can be
//! approved without a person. Only a simulator exists: it answers from fixed patterns so the whole flow
//! can be exercised, and verifies nothing about a real business. A real provider is a separate piece of
//! work that needs a licensed vendor; until then the simulator must be switched on explicitly
//! (`ZYVOR_KYC_PROVIDER=simulator`) and every submission goes to a person.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail,
    Inconclusive,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Pass => "pass",
            Outcome::Fail => "fail",
            Outcome::Inconclusive => "inconclusive",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Check {
    pub outcome: Outcome,
    pub detail: String,
    /// The name the bank holds for the account, when the check returns one. Compared with the name the
    /// merchant gave; it is never stored.
    pub registered_name: Option<String>,
}

fn check(outcome: Outcome, detail: &str) -> Check {
    Check {
        outcome,
        detail: detail.to_string(),
        registered_name: None,
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Kyc {
    Simulator,
}

impl Kyc {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "simulator" => Some(Kyc::Simulator),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kyc::Simulator => "simulator",
        }
    }

    /// Does this PAN exist, and does it belong to this name? The simulator: a PAN ending in `X` is not
    /// found, one ending in `Y` cannot be matched to the name, anything else passes.
    pub async fn verify_pan(self, pan: &str, _legal_name: &str) -> Check {
        match (self, pan.chars().last()) {
            (Kyc::Simulator, Some('X')) => check(Outcome::Fail, "PAN not found"),
            (Kyc::Simulator, Some('Y')) => check(
                Outcome::Inconclusive,
                "PAN holder name could not be matched",
            ),
            (Kyc::Simulator, _) => check(Outcome::Pass, "PAN found and name matches"),
        }
    }

    /// A penny-drop: does this account exist and is it held by this name? The simulator: an account
    /// number ending `0000` does not exist, one ending `1111` has a different holder name.
    pub async fn verify_bank(self, _ifsc: &str, account: &str, holder: &str) -> Check {
        match self {
            Kyc::Simulator if account.ends_with("0000") => {
                check(Outcome::Fail, "account not found")
            }
            // the bank's records say someone else: the caller's name comparison catches it
            Kyc::Simulator if account.ends_with("1111") => Check {
                registered_name: Some("SOMEONE ELSE ENTIRELY".into()),
                ..check(Outcome::Pass, "account exists")
            },
            Kyc::Simulator => Check {
                registered_name: Some(holder.to_string()),
                ..check(Outcome::Pass, "account verified by penny drop")
            },
        }
    }
}

/// Do two account-holder names refer to the same party? Case, punctuation, honorifics and company suffixes
/// are ignored, and the shorter name may be a subset of the longer ("KIRAN TRADERS" and "KIRAN TRADERS
/// PRIVATE LIMITED"). It needs at least one real word in common and rejects an empty name.
pub fn names_match(a: &str, b: &str) -> bool {
    fn words(s: &str) -> std::collections::BTreeSet<String> {
        const NOISE: [&str; 14] = [
            "MR", "MRS", "MS", "MISS", "DR", "SHRI", "SMT", "SRI", "M", "S", "PVT", "LTD",
            "LIMITED", "PRIVATE",
        ];
        s.to_uppercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty() && !NOISE.contains(w))
            .map(str::to_string)
            .collect()
    }
    let (x, y) = (words(a), words(b));
    if x.is_empty() || y.is_empty() {
        return false;
    }
    let (short, long) = if x.len() <= y.len() {
        (&x, &y)
    } else {
        (&y, &x)
    };
    short.is_subset(long)
}

/// A GSTIN embeds the holder's PAN in characters 3 to 12.
pub fn gstin_matches_pan(gstin: &str, pan: &str) -> bool {
    gstin.get(2..12) == Some(pan)
}

/// The fourth character of a PAN says what kind of holder it is; check it fits the business type.
pub fn pan_fits_business(pan: &str, business_type: &str) -> bool {
    match pan.chars().nth(3) {
        Some('P') => business_type == "proprietorship",
        Some('C') => business_type == "private_limited",
        Some('F') => matches!(business_type, "partnership" | "llp"),
        Some('T') => business_type == "trust_or_society",
        // companies, HUFs, associations, local authorities and so on: not decided here
        Some(_) => true,
        None => false,
    }
}

pub enum Decision {
    Approve,
    /// Needs a person, with the reasons to show them.
    Review(Vec<String>),
}

/// Approve only when every check passed and the case is ordinary; otherwise say what a person should look at.
pub fn decide(
    pan: Option<Outcome>,
    gstin: Option<Outcome>,
    banks: &[Outcome],
    business_type: &str,
) -> Decision {
    let mut why = Vec::new();
    match pan {
        Some(Outcome::Pass) => {}
        Some(Outcome::Fail) => why.push("PAN check failed".to_string()),
        Some(Outcome::Inconclusive) => why.push("PAN check was inconclusive".to_string()),
        None => why.push("PAN has not been checked".to_string()),
    }
    match gstin {
        None | Some(Outcome::Pass) => {}
        Some(_) => why.push("GSTIN does not match the PAN".to_string()),
    }
    if !banks.contains(&Outcome::Pass) {
        why.push(if banks.contains(&Outcome::Fail) {
            "bank account check failed".to_string()
        } else {
            "no bank account passed verification".to_string()
        });
    }
    if business_type == "other" {
        why.push("business type is 'other'".to_string());
    }
    if why.is_empty() {
        Decision::Approve
    } else {
        Decision::Review(why)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulator_patterns() {
        let r = tokio_test_block(async {
            (
                Kyc::Simulator.verify_pan("ABCPE1234F", "A").await.outcome,
                Kyc::Simulator.verify_pan("ABCPE1234X", "A").await.outcome,
                Kyc::Simulator.verify_pan("ABCPE1234Y", "A").await.outcome,
                Kyc::Simulator
                    .verify_bank("HDFC0001234", "123456789012", "A")
                    .await
                    .outcome,
                Kyc::Simulator
                    .verify_bank("HDFC0001234", "123456780000", "A")
                    .await
                    .outcome,
                Kyc::Simulator
                    .verify_bank("HDFC0001234", "123456781111", "A")
                    .await
                    .outcome,
            )
        });
        use Outcome::*;
        // an account held under another name still "passes" the existence check; the caller compares names
        assert_eq!(r, (Pass, Fail, Inconclusive, Pass, Fail, Pass));
        let other = tokio_test_block(Kyc::Simulator.verify_bank(
            "HDFC0001234",
            "123456781111",
            "Kiran Traders",
        ));
        assert!(!names_match(
            "Kiran Traders",
            other.registered_name.as_deref().unwrap()
        ));
        let same = tokio_test_block(Kyc::Simulator.verify_bank(
            "HDFC0001234",
            "123456789012",
            "Kiran Traders",
        ));
        assert!(names_match(
            "Kiran Traders",
            same.registered_name.as_deref().unwrap()
        ));
    }

    fn tokio_test_block<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn account_holder_names_are_compared_loosely_but_not_carelessly() {
        assert!(names_match("Kiran Traders", "KIRAN TRADERS"));
        assert!(names_match("Mr. Kiran Kumar", "KIRAN KUMAR"));
        assert!(names_match(
            "Kiran Traders",
            "KIRAN TRADERS PRIVATE LIMITED"
        ));
        assert!(names_match("M/S Kiran Traders", "Kiran  Traders"));
        assert!(!names_match("Kiran Traders", "SOMEONE ELSE ENTIRELY"));
        assert!(!names_match("Kiran Traders", "Kiran Brothers"));
        assert!(!names_match("", "Kiran"));
        assert!(!names_match("Mr", "Mrs"), "only honorifics is not a name");
    }

    #[test]
    fn gstin_and_pan_type_rules() {
        assert!(gstin_matches_pan("27ABCPE1234F1Z5", "ABCPE1234F"));
        assert!(!gstin_matches_pan("27ABCPE1234F1Z5", "ABCDE1234F"));
        assert!(pan_fits_business("ABCPE1234F", "proprietorship"));
        assert!(!pan_fits_business("ABCPE1234F", "private_limited"));
        assert!(pan_fits_business("ABCCE1234F", "private_limited"));
        assert!(pan_fits_business("ABCHE1234F", "other"));
    }

    #[test]
    fn only_a_clean_case_is_approved() {
        use Outcome::*;
        assert!(matches!(
            decide(Some(Pass), None, &[Pass], "llp"),
            Decision::Approve
        ));
        assert!(matches!(
            decide(Some(Pass), Some(Pass), &[Fail, Pass], "llp"),
            Decision::Approve
        ));
        for d in [
            decide(Some(Fail), None, &[Pass], "llp"),
            decide(Some(Inconclusive), None, &[Pass], "llp"),
            decide(None, None, &[Pass], "llp"),
            decide(Some(Pass), Some(Fail), &[Pass], "llp"),
            decide(Some(Pass), None, &[Inconclusive], "llp"),
            decide(Some(Pass), None, &[], "llp"),
            decide(Some(Pass), None, &[Pass], "other"),
        ] {
            assert!(matches!(d, Decision::Review(r) if !r.is_empty()));
        }
    }
}
