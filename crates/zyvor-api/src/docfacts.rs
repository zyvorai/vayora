//! What an onboarding document says, found in its text, and whether that agrees with what the merchant entered.
//! Plain scanning for the fixed shapes of Indian business identifiers (GSTIN, PAN, CIN, IFSC) and for the few
//! labelled lines that carry the legal name. A reading is a signal for a person, never a verdict: a mismatch or a
//! missing fact sends the submission to review, and a perfect match still only counts as "the document agrees".
//!
//! Only these facts are kept from a document: GSTIN, CIN and IFSC (public identifiers), the last four characters of
//! the PAN, and the legal name. The full PAN is used while reading and then dropped.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::gst;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facts {
    pub gstin: Option<String>,
    /// Last four characters of the PAN found on the document (or inside its GSTIN).
    pub pan_last4: Option<String>,
    /// When both a PAN and a GSTIN were found: whether the GSTIN carries that PAN.
    pub pan_in_gstin: Option<bool>,
    pub cin: Option<String>,
    pub legal_name: Option<String>,
    pub ifsc: Option<String>,
}

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
}

fn is_pan(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() == 10
        && b[..5].iter().all(u8::is_ascii_uppercase)
        && b[5..9].iter().all(u8::is_ascii_digit)
        && b[9].is_ascii_uppercase()
        // the fourth letter says what kind of holder it is
        && b"PCHFATBLJG".contains(&b[3])
}

fn is_cin(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() == 21
        && (b[0] == b'L' || b[0] == b'U')
        && b[1..6].iter().all(u8::is_ascii_digit)
        && b[6..8].iter().all(u8::is_ascii_uppercase)
        && b[8..12].iter().all(u8::is_ascii_digit)
        && b[12..15].iter().all(u8::is_ascii_uppercase)
        && b[15..].iter().all(u8::is_ascii_digit)
}

fn is_ifsc(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() == 11
        && b[..4].iter().all(u8::is_ascii_uppercase)
        && b[4] == b'0'
        && b[5..]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The legal name from the GST certificate's labelled line ("1. Legal Name   ACME ..."): the text after the label,
/// up to the next wide gap (the columns of the printed form), or the next line when the value wraps.
fn name_after_legal_name_label(text: &str) -> Option<String> {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(i) = line.find("Legal Name") else {
            continue;
        };
        let mut rest = line[i + "Legal Name".len()..].trim().to_string();
        if rest.is_empty() {
            rest = lines
                .by_ref()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string();
        }
        let first = rest.split("   ").next().unwrap_or("").trim();
        let name = first
            .trim_matches(|c: char| !c.is_alphanumeric() && c != ')')
            .to_string();
        if name.chars().count() >= 3 {
            return Some(collapse(&name));
        }
    }
    None
}

/// The name on a PAN card or similar: a line that is only "Name" (or "Name:" with the value after it), with the
/// value on that line or the next. Tried last, because a bare "Name" label is the least specific.
fn name_after_plain_name_label(text: &str) -> Option<String> {
    let mut lines = text.lines().map(str::trim);
    while let Some(line) = lines.next() {
        let rest = if line.eq_ignore_ascii_case("name") {
            lines.by_ref().find(|l| !l.is_empty()).unwrap_or("")
        } else if let Some(r) = line
            .strip_prefix("Name:")
            .or_else(|| line.strip_prefix("NAME:"))
        {
            r.trim()
        } else {
            continue;
        };
        let name = rest.split("   ").next().unwrap_or("").trim();
        if name.chars().count() >= 3 && name.chars().any(|c| c.is_alphabetic()) {
            return Some(collapse(name));
        }
    }
    None
}

/// The legal name from a certificate of incorporation: "... certify that ACME LTD is incorporated ...".
fn name_in_incorporation(text: &str) -> Option<String> {
    let flat = collapse(text);
    let i = flat.find("certify that ")? + "certify that ".len();
    let rest = &flat[i..];
    let j = rest.find(" is incorporated")?;
    let name = rest[..j].trim();
    (name.chars().count() >= 3 && name.chars().count() <= 150).then(|| name.to_string())
}

pub fn extract(text: &str) -> Facts {
    let mut f = Facts::default();
    let mut pan_full: Option<&str> = None;
    for t in tokens(text) {
        if f.gstin.is_none() && t.len() == 15 && gst::valid_gstin(t) {
            f.gstin = Some(t.to_string());
        } else if pan_full.is_none() && is_pan(t) {
            pan_full = Some(t);
        } else if f.cin.is_none() && is_cin(t) {
            f.cin = Some(t.to_string());
        } else if f.ifsc.is_none() && is_ifsc(t) {
            f.ifsc = Some(t.to_string());
        }
    }
    let gstin_pan = f.gstin.as_deref().map(|g| &g[2..12]);
    if let (Some(p), Some(g)) = (pan_full, gstin_pan) {
        f.pan_in_gstin = Some(p == g);
    }
    if let Some(p) = pan_full.or(gstin_pan) {
        f.pan_last4 = Some(p[p.len() - 4..].to_string());
    }
    f.legal_name = name_in_incorporation(text)
        .or_else(|| name_after_legal_name_label(text))
        .or_else(|| name_after_plain_name_label(text));
    f
}

/// True when the text looks like an Aadhaar card or letter: the word Aadhaar (or the authority's name) together
/// with a 12-digit number in the usual groups of four. Those are refused, not stored.
pub fn looks_like_aadhaar(text: &str) -> bool {
    let low = text.to_lowercase();
    if !(low.contains("aadhaar")
        || low.contains("aadhar")
        || low.contains("uidai")
        || low.contains("unique identification authority"))
    {
        return false;
    }
    let digits: Vec<&str> = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|g| !g.is_empty())
        .collect();
    digits.windows(3).any(|w| w.iter().all(|g| g.len() == 4))
        || digits.iter().any(|g| g.len() == 12)
}

/// A company name with the usual spelling variants folded together, for comparing two spellings.
pub fn name_key(s: &str) -> String {
    let spaced: String = s
        .to_uppercase()
        .replace('&', " AND ")
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    spaced
        .split_whitespace()
        .map(|w| match w {
            "PVT" => "PRIVATE",
            "LTD" => "LIMITED",
            "CO" => "COMPANY",
            "LLP" => "LLP",
            w => w,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, PartialEq, Eq)]
pub enum NameMatch {
    Same,
    /// Nearly the same (a word added, dropped or misspelt): worth a look, not worth refusing.
    Close,
    Different,
}

pub fn names_agree(a: &str, b: &str) -> NameMatch {
    let (ka, kb) = (name_key(a), name_key(b));
    if ka == kb {
        return NameMatch::Same;
    }
    let sa: std::collections::HashSet<&str> = ka.split(' ').collect();
    let sb: std::collections::HashSet<&str> = kb.split(' ').collect();
    let common = sa.intersection(&sb).count() as f64;
    let union = sa.union(&sb).count() as f64;
    if union > 0.0 && common / union >= 0.7 {
        NameMatch::Close
    } else {
        NameMatch::Different
    }
}

/// One comparison between a document and the profile. `result` is `match`, `mismatch`, `not_found` (the document
/// did not show it) or `not_checked` (nothing to compare it with).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub result: &'static str,
    pub detail: String,
}

impl Check {
    fn new(name: &'static str, result: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            result,
            detail: detail.into(),
        }
    }
    /// Whether a person needs to look because of this check.
    pub fn needs_a_person(&self) -> bool {
        matches!(self.result, "mismatch" | "not_found")
    }
}

/// What the merchant entered, to compare a document with.
pub struct Entered<'a> {
    pub legal_name: &'a str,
    pub pan_last4: &'a str,
    pub gstin: Option<&'a str>,
    pub bank_ifscs: &'a [String],
}

fn pan_check(f: &Facts, e: &Entered) -> Check {
    match &f.pan_last4 {
        None => Check::new(
            "pan_matches_profile",
            "not_found",
            "no PAN found on the document",
        ),
        Some(p) if p.eq_ignore_ascii_case(e.pan_last4.trim()) => Check::new(
            "pan_matches_profile",
            "match",
            "the PAN ends the same way as the one entered",
        ),
        Some(_) => Check::new(
            "pan_matches_profile",
            "mismatch",
            "the PAN on the document ends differently from the one entered",
        ),
    }
}

fn name_check(f: &Facts, e: &Entered) -> Check {
    match f.legal_name.as_deref() {
        None => Check::new(
            "legal_name_matches_profile",
            "not_found",
            "no legal name found on the document",
        ),
        Some(n) => match names_agree(n, e.legal_name) {
            NameMatch::Same => Check::new(
                "legal_name_matches_profile",
                "match",
                format!("the document names {n}"),
            ),
            NameMatch::Close => Check::new(
                "legal_name_matches_profile",
                "mismatch",
                format!(
                    "the document names \"{n}\", close to but not the same as \"{}\"",
                    e.legal_name
                ),
            ),
            NameMatch::Different => Check::new(
                "legal_name_matches_profile",
                "mismatch",
                format!("the document names \"{n}\", not \"{}\"", e.legal_name),
            ),
        },
    }
}

/// The comparisons that make sense for this kind of document.
pub fn cross_check(kind: &str, f: &Facts, e: &Entered) -> Vec<Check> {
    match kind {
        "gst_certificate" => {
            let found = match &f.gstin {
                Some(_) => Check::new(
                    "gstin_found",
                    "match",
                    "a GSTIN with a valid check character is on the document",
                ),
                None => Check::new(
                    "gstin_found",
                    "not_found",
                    "no GSTIN with a valid check character found",
                ),
            };
            let same = match (&f.gstin, e.gstin) {
                (Some(d), Some(p)) if d.eq_ignore_ascii_case(p.trim()) => Check::new(
                    "gstin_matches_profile",
                    "match",
                    "the same GSTIN as entered",
                ),
                (Some(_), Some(_)) => Check::new(
                    "gstin_matches_profile",
                    "mismatch",
                    "the GSTIN on the document is not the one entered",
                ),
                (_, None) => Check::new(
                    "gstin_matches_profile",
                    "not_checked",
                    "no GSTIN was entered to compare with",
                ),
                (None, Some(_)) => Check::new(
                    "gstin_matches_profile",
                    "not_found",
                    "no GSTIN on the document to compare",
                ),
            };
            vec![found, same, pan_check(f, e), name_check(f, e)]
        }
        "incorporation" => {
            let mut v = vec![match &f.cin {
                Some(_) => Check::new(
                    "cin_found",
                    "match",
                    "a company identity number is on the document",
                ),
                None => Check::new(
                    "cin_found",
                    "not_found",
                    "no company identity number (CIN) found",
                ),
            }];
            v.push(pan_check(f, e));
            v.push(name_check(f, e));
            v
        }
        "pan" => vec![pan_check(f, e), name_check(f, e)],
        "cancelled_cheque" => vec![match &f.ifsc {
            None => Check::new(
                "ifsc_matches_account",
                "not_found",
                "no IFSC found on the document",
            ),
            Some(i) if e.bank_ifscs.iter().any(|b| b.eq_ignore_ascii_case(i)) => Check::new(
                "ifsc_matches_account",
                "match",
                "the IFSC is that of a bank account entered",
            ),
            Some(_) => Check::new(
                "ifsc_matches_account",
                "mismatch",
                "the IFSC on the document is not that of any bank account entered",
            ),
        }],
        _ => vec![Check::new(
            "kept_for_the_reviewer",
            "not_checked",
            "kept for the person reviewing; nothing on it is compared automatically",
        )],
    }
}

/// What is kept about a document, as stored JSON.
pub fn facts_json(f: &Facts) -> Value {
    json!(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A syntactically valid GSTIN for tests, built from a made-up PAN.
    fn gstin(state: &str, pan: &str) -> String {
        let first14 = format!("{state}{pan}1Z");
        format!("{first14}{}", gst::gstin_check_char(&first14))
    }

    const PAN: &str = "ABCCE1234F";

    fn gst_text(g: &str, name: &str) -> String {
        format!(
            "                  Government of India\n                  Form GST REG-06\n      Registration Number : {g}\n 1.   Legal Name                         {name}\n 2.   Trade Name, if any                 {name}                             x\n 4.   Constitution of Business           Private Limited Company\n"
        )
    }

    fn entered<'a>(name: &'a str, g: Option<&'a str>, ifscs: &'a [String]) -> Entered<'a> {
        Entered {
            legal_name: name,
            pan_last4: "234F",
            gstin: g,
            bank_ifscs: ifscs,
        }
    }

    #[test]
    fn a_gst_certificate_gives_its_gstin_pan_and_legal_name() {
        let g = gstin("27", PAN);
        let f = extract(&gst_text(&g, "ACME TESTING PRIVATE LIMITED"));
        assert_eq!(f.gstin.as_deref(), Some(g.as_str()));
        assert_eq!(f.pan_last4.as_deref(), Some("234F"));
        assert_eq!(
            f.legal_name.as_deref(),
            Some("ACME TESTING PRIVATE LIMITED")
        );
        assert_eq!(
            f.pan_in_gstin, None,
            "no separate PAN on the page to compare"
        );
    }

    #[test]
    fn a_gstin_with_a_wrong_check_character_is_not_taken() {
        let g = gstin("27", PAN);
        let bad = format!("{}{}", &g[..14], if g.ends_with('A') { 'B' } else { 'A' });
        assert_eq!(extract(&format!("Registration Number : {bad}")).gstin, None);
    }

    #[test]
    fn a_scanned_certificate_of_incorporation_gives_its_cin_pan_and_name_despite_ocr_marks() {
        let text = "Certificate of Incorporation\nI hereby certify that ACME TESTING PRIVATE LIMITED is incorporated on this SIXTH day of MAY\nThe Corporate Identity Number of the company is U12345MH2026PTC123456\nThe Permanent Account Number (PAN) of the company is ABCCE1234F*\nThe Tax Deduction and Collection Account Number (TAN) of the company is MUMA12345B*\n";
        let f = extract(text);
        assert_eq!(f.cin.as_deref(), Some("U12345MH2026PTC123456"));
        assert_eq!(f.pan_last4.as_deref(), Some("234F"));
        assert_eq!(
            f.legal_name.as_deref(),
            Some("ACME TESTING PRIVATE LIMITED")
        );
        // the TAN is not mistaken for a PAN
        assert_eq!(f.pan_in_gstin, None);
    }

    #[test]
    fn a_pan_card_gives_the_pan_and_the_name_under_its_name_label() {
        let f = extract(
            "INCOME TAX DEPARTMENT\nPermanent Account Number\nABCCE1234F\nName\nACME TESTING PRIVATE LIMITED\nDate of Incorporation\n01/01/2020\n",
        );
        assert_eq!(f.pan_last4.as_deref(), Some("234F"));
        assert_eq!(
            f.legal_name.as_deref(),
            Some("ACME TESTING PRIVATE LIMITED")
        );
        let g = extract("Name: ACME TESTING PRIVATE LIMITED\nPAN ABCCE1234F");
        assert_eq!(
            g.legal_name.as_deref(),
            Some("ACME TESTING PRIVATE LIMITED")
        );
        // a line that merely begins with the word is not a label
        assert_eq!(
            extract("Names of directors are listed overleaf\nABCCE1234F").legal_name,
            None
        );
    }

    #[test]
    fn a_pan_and_a_gstin_on_one_document_are_checked_against_each_other() {
        let g = gstin("27", PAN);
        let same = extract(&format!("PAN {PAN}\nGSTIN {g}"));
        assert_eq!(same.pan_in_gstin, Some(true));
        let other = extract(&format!("PAN ZZZZZ9999Z\nGSTIN {g}"));
        assert_eq!(
            other.pan_in_gstin, None,
            "ZZZZZ9999Z is not a PAN shape: its fourth letter is not a holder type"
        );
        let other = extract(&format!("PAN XYZPQ9999R\nGSTIN {g}"));
        assert_eq!(other.pan_in_gstin, Some(false));
    }

    #[test]
    fn names_that_differ_only_in_punctuation_and_the_usual_abbreviations_are_the_same() {
        assert_eq!(
            names_agree("ACME TESTING PRIVATE LIMITED", "Acme Testing Pvt. Ltd."),
            NameMatch::Same
        );
        assert_eq!(
            names_agree("Sharma & Sons", "SHARMA AND SONS"),
            NameMatch::Same
        );
        assert_eq!(
            names_agree(
                "ACME TESTING PRIVATE LIMITED",
                "ACME TESTING LABS PRIVATE LIMITED"
            ),
            NameMatch::Close
        );
        assert_eq!(
            names_agree("ACME TESTING PRIVATE LIMITED", "GLOBEX CORPORATION"),
            NameMatch::Different
        );
    }

    #[test]
    fn a_gst_certificate_is_compared_with_what_was_entered() {
        let g = gstin("27", PAN);
        let f = extract(&gst_text(&g, "ACME TESTING PRIVATE LIMITED"));
        let ok = cross_check(
            "gst_certificate",
            &f,
            &entered("Acme Testing Pvt Ltd", Some(&g), &[]),
        );
        assert!(ok.iter().all(|c| c.result == "match"), "{ok:?}");
        let other = gstin("29", "ZYXWV9876P");
        let bad = cross_check(
            "gst_certificate",
            &f,
            &entered("Globex Corporation", Some(&other), &[]),
        );
        let names: Vec<&str> = bad
            .iter()
            .filter(|c| c.result == "mismatch")
            .map(|c| c.name)
            .collect();
        assert_eq!(
            names,
            ["gstin_matches_profile", "legal_name_matches_profile"],
            "{bad:?}"
        );
        assert!(bad.iter().any(Check::needs_a_person));
        // nothing entered to compare a GSTIN with is not a problem by itself
        let none = cross_check(
            "gst_certificate",
            &f,
            &entered("Acme Testing Pvt Ltd", None, &[]),
        );
        assert!(!none.iter().any(Check::needs_a_person), "{none:?}");
    }

    #[test]
    fn a_document_that_shows_nothing_is_not_taken_as_agreeing() {
        let f = extract("a blurry page with nothing useful on it, long enough to count as text");
        for kind in [
            "gst_certificate",
            "incorporation",
            "pan",
            "cancelled_cheque",
        ] {
            let c = cross_check(kind, &f, &entered("Acme", Some("27ABCCE1234F1Z5"), &[]));
            assert!(c.iter().any(Check::needs_a_person), "{kind}: {c:?}");
        }
        let other = cross_check("other", &f, &entered("Acme", None, &[]));
        assert_eq!(other[0].result, "not_checked");
    }

    #[test]
    fn a_cancelled_cheque_is_matched_by_its_ifsc() {
        let f = extract("Pay to the order of ACME   IFSC: HDFC0001234   A/c 1234567890");
        assert_eq!(f.ifsc.as_deref(), Some("HDFC0001234"));
        let mine = ["HDFC0001234".to_string()];
        assert_eq!(
            cross_check("cancelled_cheque", &f, &entered("Acme", None, &mine))[0].result,
            "match"
        );
        let others = ["SBIN0000001".to_string()];
        assert_eq!(
            cross_check("cancelled_cheque", &f, &entered("Acme", None, &others))[0].result,
            "mismatch"
        );
    }

    #[test]
    fn an_aadhaar_card_is_recognised_so_it_can_be_refused() {
        assert!(looks_like_aadhaar(
            "Government of India\nAadhaar\n1234 5678 9012\nDOB: 01/01/1990"
        ));
        assert!(looks_like_aadhaar(
            "Unique Identification Authority of India  123456789012"
        ));
        assert!(!looks_like_aadhaar(
            "Udyam Registration Certificate mentions Aadhaar only in passing, no number"
        ));
        assert!(!looks_like_aadhaar("Invoice 1234 5678 9012 paid"));
    }
}
