//! Cashfree Secure ID: PAN, bank account and GSTIN verification (the "Verification Suite" APIs). Written from
//! Cashfree's API reference (sandbox `https://sandbox.cashfree.com/verification`, production
//! `https://api.cashfree.com/verification`; headers `x-client-id` and `x-client-secret`).
//!
//! Three rules shape everything here:
//! - A provider answer that is not a clear yes or a clear no is **never** a pass. Bad credentials, an address that
//!   is not whitelisted, no balance, rate limits, outages and timeouts all come back `Inconclusive`, which sends
//!   the case to a person.
//! - A mismatching name is `Inconclusive`, not `Fail`: people write names in many ways, and a person decides.
//! - Name matching is a paid add-on at Cashfree; when its fields are missing we compare names ourselves.
//!
//! Nothing here stores or logs a PAN, an account number or a name.

use std::time::Duration;

use serde_json::{Value, json};

use crate::kyc::{Check, Outcome, names_match};

pub struct SecureId {
    base: String,
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

fn check(outcome: Outcome, detail: impl Into<String>) -> Check {
    Check {
        outcome,
        detail: detail.into(),
        registered_name: None,
    }
}

/// Characters Cashfree accepts in a name: letters, digits, space and `. - / &`.
fn clean_name(n: &str) -> String {
    n.chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '/' | '&'))
        .collect::<String>()
        .trim()
        .to_string()
}

impl SecureId {
    pub const SANDBOX: &'static str = "https://sandbox.cashfree.com/verification";
    pub const PRODUCTION: &'static str = "https://api.cashfree.com/verification";

    pub fn new(
        base: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client");
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            http,
        }
    }

    pub fn is_production(&self) -> bool {
        self.base == Self::PRODUCTION
    }

    /// `Err` only when no answer arrived at all (connection, timeout).
    async fn post(
        &self,
        path: &str,
        body: Value,
        api_version: Option<&str>,
    ) -> Result<(u16, Value), String> {
        let mut r = self
            .http
            .post(format!("{}{path}", self.base))
            .header("x-client-id", &self.client_id)
            .header("x-client-secret", &self.client_secret)
            .json(&body);
        if let Some(v) = api_version {
            r = r.header("x-api-version", v);
        }
        let res = r.send().await.map_err(|e| {
            if e.is_timeout() {
                "timed out".to_string()
            } else {
                "could not connect".to_string()
            }
        })?;
        let status = res.status().as_u16();
        Ok((status, res.json().await.unwrap_or(Value::Null)))
    }

    pub async fn verify_pan(&self, pan: &str, legal_name: &str) -> Check {
        let mut body = json!({ "pan": pan });
        let name = clean_name(legal_name);
        if !name.is_empty() {
            body["name"] = json!(name);
        }
        match self.post("/pan", body, Some("2024-08-01")).await {
            Ok((st, b)) => interpret_pan(st, &b, legal_name),
            Err(e) => unavailable(&e),
        }
    }

    pub async fn verify_bank(&self, ifsc: &str, account: &str, holder: &str) -> Check {
        let mut body = json!({ "bank_account": account, "ifsc": ifsc });
        let name = clean_name(holder);
        if !name.is_empty() {
            body["name"] = json!(name);
        }
        match self.post("/bank-account/sync", body, None).await {
            Ok((st, b)) => interpret_bank(st, &b),
            Err(e) => unavailable(&e),
        }
    }

    pub async fn verify_gstin(&self, gstin: &str, legal_name: &str) -> Check {
        let mut body = json!({ "GSTIN": gstin });
        let name = clean_name(legal_name);
        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == ' ') {
            body["business_name"] = json!(name);
        }
        match self.post("/gstin", body, None).await {
            Ok((st, b)) => interpret_gstin(st, &b, legal_name),
            Err(e) => unavailable(&e),
        }
    }
}

fn unavailable(why: &str) -> Check {
    tracing::error!(why, "KYC provider unreachable");
    check(
        Outcome::Inconclusive,
        format!(
            "the identity-check provider could not be reached ({why}); a person will review this"
        ),
    )
}

/// What a non-success HTTP answer means. Always `Inconclusive` (a person decides), and logged for the operator,
/// since most of these are configuration the merchant cannot fix.
fn provider_problem(status: u16, body: &Value) -> Check {
    let code = body["code"].as_str().unwrap_or("");
    tracing::error!(status, code, "KYC provider refused or failed a check");
    let detail = match (status, code) {
        (401, _) => "the identity-check provider did not accept our credentials",
        (403, "ip_validation_failed") => {
            "the identity-check provider does not accept requests from our server address"
        }
        (403, _) => "the identity-check provider refused the request",
        (400, "x-client-secret_value_invalid") => {
            "our identity-check credentials are for the wrong environment"
        }
        (400, _) => "the identity-check provider rejected the request",
        (422, "insufficient_balance") => "the identity-check provider account is out of balance",
        (429, _) => "the identity-check provider is limiting requests; try again later",
        (s, _) if s >= 500 => "the identity-check provider is unavailable",
        _ => "the identity-check provider gave an unexpected answer",
    };
    check(
        Outcome::Inconclusive,
        format!("{detail}; a person will review this"),
    )
}

pub fn interpret_pan(status: u16, body: &Value, legal_name: &str) -> Check {
    if status != 200 {
        return provider_problem(status, body);
    }
    let pan_status = body["pan_status"].as_str();
    if body["valid"].as_bool() == Some(false)
        || matches!(pan_status, Some("INVALID" | "DELETED" | "DEACTIVATED"))
    {
        return check(
            Outcome::Fail,
            "the PAN is not valid in the income-tax records",
        );
    }
    if body["valid"].as_bool() != Some(true) {
        return check(
            Outcome::Inconclusive,
            "the PAN check gave an unclear answer",
        );
    }
    match body["name_match_result"].as_str() {
        Some("DIRECT_MATCH" | "GOOD_PARTIAL_MATCH") => {
            check(Outcome::Pass, "PAN valid; the name matches the record")
        }
        Some("MODERATE_PARTIAL_MATCH" | "POOR_PARTIAL_MATCH" | "NO_MATCH") => check(
            Outcome::Inconclusive,
            "PAN valid, but the name does not clearly match the record",
        ),
        // the name-match add-on is not switched on for the account: compare the registered name ourselves
        _ => match body["registered_name"].as_str() {
            Some(r) if names_match(legal_name, r) => {
                check(Outcome::Pass, "PAN valid; the name matches the record")
            }
            Some(_) => check(
                Outcome::Inconclusive,
                "PAN valid, but the name does not match the record",
            ),
            None => check(
                Outcome::Inconclusive,
                "PAN valid, but no registered name was returned to compare",
            ),
        },
    }
}

pub fn interpret_bank(status: u16, body: &Value) -> Check {
    if status != 200 {
        return provider_problem(status, body);
    }
    match body["account_status"].as_str() {
        Some("VALID") => {
            // a clear mismatch from the provider is never a pass; otherwise the caller compares names itself
            if matches!(
                body["name_match_result"].as_str(),
                Some("NO_MATCH" | "POOR_PARTIAL_MATCH")
            ) {
                return check(
                    Outcome::Inconclusive,
                    "the account exists, but the holder name does not match the name given",
                );
            }
            Check {
                registered_name: body["name_at_bank"].as_str().map(str::to_string),
                ..check(Outcome::Pass, "account verified with the bank")
            }
        }
        Some("INVALID") => match body["account_status_code"].as_str() {
            Some("INVALID_ACCOUNT_FAIL") => {
                check(Outcome::Fail, "the account was not found at the bank")
            }
            Some("INVALID_IFSC_FAIL") => check(Outcome::Fail, "the IFSC was not found"),
            Some("ACCOUNT_BLOCKED") => check(Outcome::Fail, "the account is blocked"),
            Some("NRE_ACCOUNT_FAIL") => check(
                Outcome::Inconclusive,
                "NRE accounts cannot be checked automatically",
            ),
            _ => check(
                Outcome::Inconclusive,
                "the bank could not confirm the account",
            ),
        },
        _ => check(
            Outcome::Inconclusive,
            "the bank check gave an unclear answer",
        ),
    }
}

pub fn interpret_gstin(status: u16, body: &Value, legal_name: &str) -> Check {
    if status != 200 {
        return provider_problem(status, body);
    }
    if body["valid"].as_bool() != Some(true) {
        // Cashfree answers "GSTIN Doesn't Exist" without a `valid` field
        return check(Outcome::Fail, "the GSTIN was not found");
    }
    if body["gst_in_status"].as_str() != Some("Active") {
        return check(Outcome::Fail, "the GSTIN is not active");
    }
    let names = [
        body["legal_name_of_business"].as_str(),
        body["trade_name_of_business"].as_str(),
    ];
    if names.iter().flatten().any(|n| names_match(legal_name, n)) {
        check(
            Outcome::Pass,
            "GSTIN active; the name matches the registration",
        )
    } else {
        check(
            Outcome::Inconclusive,
            "GSTIN active, but the name does not match the registration",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Outcome::*;

    // the examples below are the ones in Cashfree's API reference

    #[test]
    fn pan_answers() {
        let ok = json!({ "pan": "ABCPV1234D", "type": "Individual", "valid": true, "registered_name": "JOHN DOE", "name_match_score": 100, "name_match_result": "DIRECT_MATCH", "pan_status": "VALID" });
        assert_eq!(interpret_pan(200, &ok, "John Doe").outcome, Pass);
        let invalid = json!({ "pan": "ABCPV1234D", "valid": false, "message": "Invalid PAN", "name_match_score": 0, "name_match_result": "-", "pan_status_code": "N" });
        assert_eq!(interpret_pan(200, &invalid, "John Doe").outcome, Fail);
        for s in ["DELETED", "DEACTIVATED", "INVALID"] {
            assert_eq!(
                interpret_pan(200, &json!({ "valid": true, "pan_status": s }), "x").outcome,
                Fail,
                "{s}"
            );
        }
        // valid PAN, name does not match: a person decides
        let poor = json!({ "valid": true, "registered_name": "SOMEONE", "name_match_result": "POOR_PARTIAL_MATCH", "name_match_score": "38.00" });
        assert_eq!(interpret_pan(200, &poor, "John Doe").outcome, Inconclusive);
        assert_eq!(
            interpret_pan(
                200,
                &json!({ "valid": true, "name_match_result": "GOOD_PARTIAL_MATCH" }),
                "x"
            )
            .outcome,
            Pass
        );
        // the name-match add-on is off: we compare the registered name ourselves
        assert_eq!(
            interpret_pan(
                200,
                &json!({ "valid": true, "registered_name": "KIRAN TRADERS" }),
                "Kiran Traders Private Limited"
            )
            .outcome,
            Pass
        );
        assert_eq!(
            interpret_pan(
                200,
                &json!({ "valid": true, "registered_name": "OTHER CO" }),
                "Kiran Traders"
            )
            .outcome,
            Inconclusive
        );
        assert_eq!(
            interpret_pan(200, &json!({ "valid": true }), "Kiran").outcome,
            Inconclusive
        );
        assert_eq!(
            interpret_pan(200, &json!({}), "Kiran").outcome,
            Inconclusive
        );
    }

    #[test]
    fn bank_answers() {
        let ok = json!({ "name_at_bank": "BHARATHTEST GKUMARUT", "name_match_score": "90.00", "name_match_result": "GOOD_PARTIAL_MATCH", "account_status": "VALID", "account_status_code": "ACCOUNT_IS_VALID", "utr": "404223241811" });
        let c = interpret_bank(200, &ok);
        assert_eq!(
            (c.outcome, c.registered_name.as_deref()),
            (Pass, Some("BHARATHTEST GKUMARUT"))
        );
        // no name match output (name not sent, or the add-on is off): pass on the account, the caller compares names
        assert_eq!(
            interpret_bank(
                200,
                &json!({ "name_at_bank": "A B", "account_status": "VALID" })
            )
            .outcome,
            Pass
        );
        assert_eq!(interpret_bank(200, &json!({ "name_at_bank": "A B", "account_status": "VALID", "name_match_result": "NO_MATCH" })).outcome, Inconclusive);
        let bad = |code: &str| json!({ "account_status": "INVALID", "account_status_code": code, "name_at_bank": null, "name_match_score": null });
        assert_eq!(
            interpret_bank(200, &bad("INVALID_ACCOUNT_FAIL")).outcome,
            Fail
        );
        assert_eq!(interpret_bank(200, &bad("INVALID_IFSC_FAIL")).outcome, Fail);
        assert_eq!(interpret_bank(200, &bad("ACCOUNT_BLOCKED")).outcome, Fail);
        assert_eq!(
            interpret_bank(200, &bad("NRE_ACCOUNT_FAIL")).outcome,
            Inconclusive
        );
        assert_eq!(
            interpret_bank(200, &bad("SOMETHING_NEW")).outcome,
            Inconclusive
        );
        assert_eq!(
            interpret_bank(200, &json!({ "account_status": "RECEIVED" })).outcome,
            Inconclusive
        );
    }

    #[test]
    fn gstin_answers() {
        let ok = json!({ "GSTIN": "29AAICP2912R1ZR", "legal_name_of_business": "UJJIVAN SMALL FINANCE BANK LIMITED", "trade_name_of_business": "UJJIVAN SMALL FINANCE BANK", "gst_in_status": "Active", "valid": true, "message": "GSTIN Exists" });
        assert_eq!(
            interpret_gstin(200, &ok, "Ujjivan Small Finance Bank").outcome,
            Pass
        );
        assert_eq!(
            interpret_gstin(200, &ok, "Some Other Company").outcome,
            Inconclusive
        );
        let missing = json!({ "reference_id": 20, "GSTIN": "22ABCDE1234F1Z5", "message": "GSTIN Doesn't Exist" });
        assert_eq!(interpret_gstin(200, &missing, "x").outcome, Fail);
        let cancelled =
            json!({ "valid": true, "gst_in_status": "Cancelled", "legal_name_of_business": "X" });
        assert_eq!(interpret_gstin(200, &cancelled, "X").outcome, Fail);
    }

    #[test]
    fn a_provider_problem_is_never_a_pass_or_a_fail() {
        let problems = [
            (
                401,
                json!({ "type": "authentication_error", "code": "authentication_failed" }),
            ),
            (
                403,
                json!({ "type": "authentication_error", "code": "ip_validation_failed" }),
            ),
            (
                400,
                json!({ "type": "validation_error", "code": "x-client-secret_value_invalid" }),
            ),
            (
                400,
                json!({ "type": "validation_error", "code": "pan_length_short" }),
            ),
            (
                422,
                json!({ "type": "validation_error", "code": "insufficient_balance" }),
            ),
            (
                429,
                json!({ "type": "rate_limit_error", "code": "too_many_requests_per_ip" }),
            ),
            (
                500,
                json!({ "type": "internal_error", "code": "verification_failed" }),
            ),
            (
                502,
                json!({ "type": "internal_error", "code": "verification_failed" }),
            ),
            (404, Value::Null),
        ];
        for (st, body) in problems {
            for c in [
                interpret_pan(st, &body, "x"),
                interpret_bank(st, &body),
                interpret_gstin(st, &body, "x"),
            ] {
                assert_eq!(c.outcome, Inconclusive, "{st} {body}");
                assert!(c.detail.contains("a person will review"), "{}", c.detail);
            }
        }
        // the operator is told the cause, in words
        assert!(
            interpret_pan(403, &json!({ "code": "ip_validation_failed" }), "x")
                .detail
                .contains("server address")
        );
        assert!(
            interpret_pan(422, &json!({ "code": "insufficient_balance" }), "x")
                .detail
                .contains("balance")
        );
    }

    #[test]
    fn names_are_cleaned_to_what_cashfree_accepts() {
        assert_eq!(
            clean_name("  Kiran (Traders) & Co.,  "),
            "Kiran Traders & Co."
        );
        assert_eq!(clean_name("M/S Roy-Singh"), "M/S Roy-Singh");
        assert_eq!(clean_name("***"), "");
    }
}
