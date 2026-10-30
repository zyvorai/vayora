//! India GST: tax invoices (line items with HSN/SAC and rate, split into CGST + SGST within a state
//! or IGST across states), GSTIN validation, tax-aware credit notes and a GSTR-1-style summary for
//! filing preparation. Zyvor calculates and reports; it does not file returns.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::{ApiError, AppState, auth::Auth, tenant};

/// Standard GST slabs, in basis points of the taxable value.
pub const RATES_BP: [i32; 9] = [0, 25, 100, 150, 300, 500, 1200, 1800, 2800];
/// A B2C inter-state invoice above this is reported invoice by invoice (B2CL), below it in aggregate.
const B2CL_THRESHOLD_MINOR: i64 = 250_000 * 100;

fn state_code_ok(c: &str) -> bool {
    c.len() == 2
        && c.parse::<u8>()
            .is_ok_and(|n| (1..=38).contains(&n) || n == 97 || n == 99)
}

fn gstin_value(c: u8) -> u32 {
    if c.is_ascii_digit() {
        (c - b'0') as u32
    } else {
        (c - b'A') as u32 + 10
    }
}

/// The check character for the first 14 characters of a GSTIN (base-36, alternating weights 1 and 2).
pub fn gstin_check_char(first14: &str) -> char {
    let sum: u32 = first14
        .bytes()
        .enumerate()
        .map(|(i, c)| {
            let p = gstin_value(c) * if i % 2 == 0 { 1 } else { 2 };
            p / 36 + p % 36
        })
        .sum();
    let v = (36 - sum % 36) % 36;
    char::from_digit(v, 36)
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('0')
}

/// GSTIN: 2-digit state, 10-character PAN, entity number, `Z`, and a base-36 check character.
pub fn valid_gstin(g: &str) -> bool {
    let b = g.as_bytes();
    b.len() == 15
        && g.chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
        && state_code_ok(&g[..2])
        && b[13] == b'Z'
        && g.ends_with(gstin_check_char(&g[..14]))
}

/// Round-half-up integer division of non-negative numbers.
fn div_round(n: i64, d: i64) -> i64 {
    (n * 2 + d) / (d * 2)
}

#[derive(Deserialize, Clone)]
pub struct LineIn {
    pub description: String,
    pub hsn_sac: String,
    pub quantity: i64,
    pub unit_price_minor: i64,
    /// A standard slab in percent × 100: 500 = 5%, 1800 = 18%.
    pub rate_bp: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineOut {
    pub taxable: i64,
    pub rate_bp: i32,
    pub cgst: i64,
    pub sgst: i64,
    pub igst: i64,
}

/// Tax for one line. Within a state the tax is split into equal CGST and SGST halves, each rounded
/// on its own as the rules require; across states it is IGST.
pub fn compute_line(l: &LineIn, intra_state: bool) -> LineOut {
    let taxable = l.quantity * l.unit_price_minor;
    if intra_state {
        let half = div_round(taxable * l.rate_bp as i64, 20_000);
        LineOut {
            taxable,
            rate_bp: l.rate_bp,
            cgst: half,
            sgst: half,
            igst: 0,
        }
    } else {
        LineOut {
            taxable,
            rate_bp: l.rate_bp,
            cgst: 0,
            sgst: 0,
            igst: div_round(taxable * l.rate_bp as i64, 10_000),
        }
    }
}

pub fn validate_lines(lines: &[LineIn]) -> Result<(), ApiError> {
    if lines.is_empty() || lines.len() > 100 {
        return Err(ApiError::invalid(
            "invalid_lines",
            "give between 1 and 100 lines",
        ));
    }
    for (i, l) in lines.iter().enumerate() {
        let n = i + 1;
        if l.description.trim().is_empty() {
            return Err(ApiError::invalid(
                "invalid_lines",
                format!("line {n}: description is required"),
            ));
        }
        if !(4..=8).contains(&l.hsn_sac.len()) || !l.hsn_sac.chars().all(|c| c.is_ascii_digit()) {
            return Err(ApiError::invalid(
                "invalid_hsn",
                format!("line {n}: HSN/SAC must be 4 to 8 digits"),
            ));
        }
        if l.quantity <= 0
            || l.unit_price_minor < 0
            || l.quantity
                .checked_mul(l.unit_price_minor)
                .is_none_or(|v| v > 1_000_000_000_000)
        {
            return Err(ApiError::invalid(
                "invalid_lines",
                format!("line {n}: quantity must be positive and the value reasonable"),
            ));
        }
        if !RATES_BP.contains(&l.rate_bp) {
            return Err(ApiError::invalid(
                "invalid_rate",
                format!("line {n}: rate_bp must be one of {RATES_BP:?}"),
            ));
        }
    }
    Ok(())
}

/// Everything the invoice insert needs, from the lines and the two parties.
pub struct Computed {
    pub lines: Vec<LineOut>,
    pub taxable: i64,
    pub tax: i64,
    pub total: i64,
    pub intra_state: bool,
}

pub fn compute(lines: &[LineIn], seller_state: &str, place_of_supply: &str) -> Computed {
    let intra = seller_state == place_of_supply;
    let out: Vec<LineOut> = lines.iter().map(|l| compute_line(l, intra)).collect();
    let taxable: i64 = out.iter().map(|l| l.taxable).sum();
    let tax: i64 = out.iter().map(|l| l.cgst + l.sgst + l.igst).sum();
    Computed {
        lines: out,
        taxable,
        tax,
        total: taxable + tax,
        intra_state: intra,
    }
}

/// Validate the parties and the seller's registration; returns `(seller_state, place_of_supply, buyer_gstin)`.
pub async fn parties(
    tx: &mut sqlx::PgConnection,
    place_of_supply: Option<&str>,
    buyer_gstin: Option<&str>,
    customer_id: Option<&str>,
) -> Result<(String, String, Option<String>), ApiError> {
    let seller: Option<Option<String>> =
        sqlx::query_scalar("SELECT gstin FROM tenant_settings LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?;
    let seller = seller.flatten().ok_or_else(|| {
        ApiError::invalid(
            "gst_not_configured",
            "set your GSTIN under settings (gst.gstin) before issuing tax invoices",
        )
    })?;
    let customer_gstin: Option<String> = match customer_id {
        Some(c) => sqlx::query_scalar("SELECT gstin FROM customers WHERE id = $1")
            .bind(c)
            .fetch_optional(&mut *tx)
            .await?
            .flatten(),
        None => None,
    };
    let buyer = buyer_gstin.map(str::to_string).or(customer_gstin);
    if let Some(g) = &buyer
        && !valid_gstin(g)
    {
        return Err(ApiError::invalid(
            "invalid_gstin",
            "buyer GSTIN is not valid",
        ));
    }
    let pos = place_of_supply
        .map(str::to_string)
        .or_else(|| buyer.as_ref().map(|g| g[..2].to_string()))
        .ok_or_else(|| {
            ApiError::invalid(
                "place_of_supply_required",
                "give place_of_supply (2-digit state code) or a buyer GSTIN",
            )
        })?;
    if !state_code_ok(&pos) {
        return Err(ApiError::invalid(
            "invalid_place_of_supply",
            "place_of_supply must be a GST state code such as 27 or 07",
        ));
    }
    Ok((seller[..2].to_string(), pos, buyer))
}

/// Tax reversed by a credit note of `credit` on an invoice of `total` carrying `tax`: the same share of
/// each component the invoice charged. Zero for invoices without GST lines.
pub async fn credit_note_tax(
    tx: &mut sqlx::PgConnection,
    invoice_id: &str,
    credit: i64,
    total: i64,
    tax: i64,
) -> Result<(i64, i64, i64), ApiError> {
    if tax == 0 {
        return Ok((0, 0, 0));
    }
    let (c, s, i): (i64, i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(cgst_minor),0)::BIGINT, COALESCE(SUM(sgst_minor),0)::BIGINT, COALESCE(SUM(igst_minor),0)::BIGINT FROM invoice_lines WHERE invoice_id = $1")
        .bind(invoice_id)
        .fetch_one(&mut *tx)
        .await?;
    if c + s + i == 0 {
        return Ok((0, 0, 0)); // tax was carried by the caller, not computed here
    }
    let share = |part: i64| div_round(part * credit, total);
    let target = div_round(tax * credit, total);
    let mut v = [share(c), share(s), share(i)];
    // keep the components adding up to the proportional tax
    let diff = target - v.iter().sum::<i64>();
    if diff != 0 {
        let big = (0..3).max_by_key(|k| [c, s, i][*k]).unwrap_or(0);
        v[big] += diff;
    }
    Ok((v[0].max(0), v[1].max(0), v[2].max(0)))
}

/// `GET /v1/invoices/{id}/lines`: the tax breakdown of an invoice.
pub async fn invoice_lines(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let head: Option<(String, Option<String>, Option<String>, i64, i64)> = sqlx::query_as("SELECT number, place_of_supply, buyer_gstin, amount_minor, tax_minor FROM invoices WHERE id = $1").bind(&id).fetch_optional(&mut *tx).await?;
    let (number, pos, gstin, total, tax) = head.ok_or_else(|| ApiError::not_found("invoice"))?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(i32, String, String, i64, i64, i64, i32, i64, i64, i64)> = sqlx::query_as("SELECT position, description, hsn_sac, quantity, unit_price_minor, taxable_minor, rate_bp, cgst_minor, sgst_minor, igst_minor FROM invoice_lines WHERE invoice_id = $1 ORDER BY position").bind(&id).fetch_all(&mut *tx).await?;
    let lines: Vec<Value> = rows.into_iter().map(|(p, d, h, q, u, t, r, c, sg, i)| json!({ "position": p, "description": d, "hsn_sac": h, "quantity": q, "unit_price_minor": u, "taxable_minor": t, "rate_bp": r, "cgst_minor": c, "sgst_minor": sg, "igst_minor": i })).collect();
    Ok(Json(
        json!({ "invoice_id": id, "number": number, "place_of_supply": pos, "buyer_gstin": gstin, "total_minor": total, "tax_minor": tax, "lines": lines }),
    ))
}

#[derive(Deserialize)]
pub struct PeriodQuery {
    /// `YYYY-MM`, in Indian time.
    period: String,
}

type Split = (i64, i64, i64, i64); // taxable, cgst, sgst, igst

fn add(a: &mut Split, b: Split) {
    a.0 += b.0;
    a.1 += b.1;
    a.2 += b.2;
    a.3 += b.3;
}

fn split_json(s: Split) -> Value {
    json!({ "taxable_minor": s.0, "cgst_minor": s.1, "sgst_minor": s.2, "igst_minor": s.3 })
}

/// `GET /v1/gst/gstr1?period=2026-10`: sales and credit notes in the month, grouped the way GSTR-1 asks
/// (B2B, B2C large, B2C small, credit notes, HSN summary). A preparation aid, not a filing.
pub async fn gstr1(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<PeriodQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let start = NaiveDate::parse_from_str(&format!("{}-01", q.period), "%Y-%m-%d")
        .map_err(|_| ApiError::invalid("invalid_period", "period must be YYYY-MM"))?;
    let end = start
        .checked_add_months(chrono::Months::new(1))
        .ok_or_else(|| ApiError::invalid("invalid_period", "period out of range"))?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let seller: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT gstin, gst_legal_name FROM tenant_settings LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?;
    let (gstin, legal) = seller.unwrap_or((None, None));

    #[allow(clippy::type_complexity)]
    let invs: Vec<(String, String, DateTime<Utc>, Option<String>, Option<String>, i64, i64, bool, i32, String, String, i64, i32, i64, i64, i64)> = sqlx::query_as(
        "SELECT i.id, i.number, i.created_at, i.place_of_supply, i.buyer_gstin, i.amount_minor, i.tax_minor, i.reverse_charge,
                l.position, l.description, l.hsn_sac, l.quantity, l.rate_bp, l.cgst_minor, l.sgst_minor, l.igst_minor
         FROM invoices i JOIN invoice_lines l ON l.invoice_id = i.id
         WHERE i.status <> 'void' AND (i.created_at AT TIME ZONE 'Asia/Kolkata')::date >= $1 AND (i.created_at AT TIME ZONE 'Asia/Kolkata')::date < $2
         ORDER BY i.created_at, i.id, l.position",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&mut *tx)
    .await?;
    // taxable per line is quantity x unit price; fetch it directly to avoid recomputing
    let taxable_by_line: BTreeMap<(String, i32), i64> = sqlx::query_as::<_, (String, i32, i64)>(
        "SELECT invoice_id, position, taxable_minor FROM invoice_lines",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(i, p, t)| ((i, p), t))
    .collect();

    let seller_state = gstin.as_deref().map(|g| g[..2].to_string());
    let (mut b2b, mut b2cl): (BTreeMap<String, BTreeMap<String, Value>>, Vec<Value>) =
        (BTreeMap::new(), Vec::new());
    let mut b2cs: BTreeMap<(String, i32), Split> = BTreeMap::new();
    let mut hsn: BTreeMap<(String, i32), (i64, Split)> = BTreeMap::new();
    let mut per_invoice: BTreeMap<
        String,
        (
            String,
            DateTime<Utc>,
            Option<String>,
            Option<String>,
            i64,
            bool,
            Vec<(i32, Split)>,
        ),
    > = BTreeMap::new();
    for (
        id,
        number,
        at,
        pos,
        buyer,
        total,
        _tax,
        rc,
        position,
        _d,
        hsn_code,
        qty,
        rate,
        c,
        sg,
        ig,
    ) in &invs
    {
        let taxable = taxable_by_line
            .get(&(id.clone(), *position))
            .copied()
            .unwrap_or(0);
        let e = per_invoice.entry(id.clone()).or_insert_with(|| {
            (
                number.clone(),
                *at,
                pos.clone(),
                buyer.clone(),
                *total,
                *rc,
                Vec::new(),
            )
        });
        e.6.push((*rate, (taxable, *c, *sg, *ig)));
        let h = hsn
            .entry((hsn_code.clone(), *rate))
            .or_insert((0, (0, 0, 0, 0)));
        h.0 += qty;
        add(&mut h.1, (taxable, *c, *sg, *ig));
    }
    for (_, (number, at, pos, buyer, total, rc, lines)) in per_invoice {
        let pos = pos.unwrap_or_default();
        let mut by_rate: BTreeMap<i32, Split> = BTreeMap::new();
        for (r, sp) in &lines {
            add(by_rate.entry(*r).or_default(), *sp);
        }
        let date = at
            .with_timezone(&chrono::FixedOffset::east_opt(19_800).expect("IST"))
            .date_naive();
        let inter = seller_state.as_deref().is_some_and(|s| s != pos);
        match buyer {
            Some(g) => {
                b2b.entry(g).or_default().insert(number.clone(), json!({ "number": number, "date": date, "value_minor": total, "place_of_supply": pos, "reverse_charge": rc, "rates": by_rate.iter().map(|(r, sp)| { let mut v = split_json(*sp); v["rate_bp"] = json!(r); v }).collect::<Vec<_>>() }));
            }
            None if inter && total > B2CL_THRESHOLD_MINOR => {
                b2cl.push(json!({ "number": number, "date": date, "value_minor": total, "place_of_supply": pos, "rates": by_rate.iter().map(|(r, sp)| { let mut v = split_json(*sp); v["rate_bp"] = json!(r); v }).collect::<Vec<_>>() }));
            }
            None => {
                for (r, sp) in by_rate {
                    add(b2cs.entry((pos.clone(), r)).or_default(), sp);
                }
            }
        }
    }

    #[allow(clippy::type_complexity)]
    let notes: Vec<(String, String, Option<String>, DateTime<Utc>, i64, i64, i64, i64, i64, String)> = sqlx::query_as(
        "SELECT n.id, i.number, i.buyer_gstin, n.created_at, n.amount_minor, n.taxable_minor, n.cgst_minor, n.sgst_minor, n.igst_minor, n.reason
         FROM credit_notes n JOIN invoices i ON i.id = n.invoice_id
         WHERE i.place_of_supply IS NOT NULL AND (n.created_at AT TIME ZONE 'Asia/Kolkata')::date >= $1 AND (n.created_at AT TIME ZONE 'Asia/Kolkata')::date < $2
         ORDER BY n.created_at, n.id",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&mut *tx)
    .await?;
    let (mut cdnr, mut cdnur): (Vec<Value>, Vec<Value>) = (Vec::new(), Vec::new());
    for (id, inv, buyer, at, value, taxable, c, sg, ig, reason) in notes {
        let date = at
            .with_timezone(&chrono::FixedOffset::east_opt(19_800).expect("IST"))
            .date_naive();
        let mut v = split_json((taxable, c, sg, ig));
        v["note_id"] = json!(id);
        v["invoice_number"] = json!(inv);
        v["date"] = json!(date);
        v["value_minor"] = json!(value);
        v["reason"] = json!(reason);
        match buyer {
            Some(g) => {
                v["buyer_gstin"] = json!(g);
                cdnr.push(v);
            }
            None => cdnur.push(v),
        }
    }

    let mut totals: Split = (0, 0, 0, 0);
    for (_, sp) in hsn.values() {
        add(&mut totals, *sp);
    }
    Ok(Json(json!({
        "note": "GSTR-1 style summary to prepare a return from; Zyvor does not file it",
        "period": q.period, "gstin": gstin, "legal_name": legal,
        "b2b": b2b.into_iter().map(|(g, invs)| json!({ "buyer_gstin": g, "invoices": invs.into_values().collect::<Vec<_>>() })).collect::<Vec<_>>(),
        "b2cl": b2cl,
        "b2cs": b2cs.into_iter().map(|((pos, r), sp)| { let mut v = split_json(sp); v["place_of_supply"] = json!(pos); v["rate_bp"] = json!(r); v }).collect::<Vec<_>>(),
        "cdnr": cdnr, "cdnur": cdnur,
        "hsn": hsn.into_iter().map(|((h, r), (q, sp))| { let mut v = split_json(sp); v["hsn_sac"] = json!(h); v["rate_bp"] = json!(r); v["quantity"] = json!(q); v }).collect::<Vec<_>>(),
        "totals": split_json(totals),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(q: i64, p: i64, r: i32) -> LineIn {
        LineIn {
            description: "x".into(),
            hsn_sac: "9983".into(),
            quantity: q,
            unit_price_minor: p,
            rate_bp: r,
        }
    }

    #[test]
    fn gstin_checksum() {
        assert!(
            valid_gstin("27AAPFU0939F1ZV"),
            "a widely published sample GSTIN"
        );
        assert!(!valid_gstin("27AAPFU0939F1ZW"), "wrong check character");
        for bad in [
            "",
            "27AAPFU0939F1Z",
            "27aapfu0939f1zv",
            "00AAPFU0939F1ZV",
            "27AAPFU0939F1XV",
        ] {
            assert!(!valid_gstin(bad), "{bad}");
        }
    }

    #[test]
    fn intra_state_splits_in_two_inter_state_is_igst() {
        let l = line(1, 100_000, 1800); // ₹1000 at 18%
        assert_eq!(
            compute_line(&l, true),
            LineOut {
                taxable: 100_000,
                rate_bp: 1800,
                cgst: 9_000,
                sgst: 9_000,
                igst: 0
            }
        );
        assert_eq!(
            compute_line(&l, false),
            LineOut {
                taxable: 100_000,
                rate_bp: 1800,
                cgst: 0,
                sgst: 0,
                igst: 18_000
            }
        );
        // rounding: ₹0.05 at 5% = 0.25 paise -> 0; ₹33.33 at 18% igst = 599.94 paise -> 600, halves 299.97 -> 300 each
        assert_eq!(compute_line(&line(1, 3_333, 1800), false).igst, 600);
        let c = compute_line(&line(1, 3_333, 1800), true);
        assert_eq!((c.cgst, c.sgst), (300, 300));
        assert_eq!(compute_line(&line(3, 1_000, 0), true).cgst, 0);
    }

    #[test]
    fn totals_add_up_and_state_decides_the_split() {
        let c = compute(&[line(2, 50_000, 500), line(1, 20_000, 1800)], "27", "27");
        assert_eq!(
            (c.taxable, c.tax, c.total, c.intra_state),
            (120_000, 5_000 + 3_600, 128_600, true)
        );
        let c = compute(&[line(2, 50_000, 500), line(1, 20_000, 1800)], "27", "07");
        assert_eq!((c.tax, c.intra_state), (8_600, false));
    }

    #[test]
    fn line_validation() {
        assert!(validate_lines(&[line(1, 100, 500)]).is_ok());
        assert!(validate_lines(&[]).is_err());
        assert!(validate_lines(&[line(0, 100, 500)]).is_err());
        assert!(
            validate_lines(&[line(1, 100, 700)]).is_err(),
            "7% is not a slab"
        );
        let mut l = line(1, 100, 500);
        l.hsn_sac = "99".into();
        assert!(validate_lines(&[l]).is_err());
    }
}
