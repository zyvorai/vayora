// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Many payment links from one CSV.
//!
//! The file is read strictly and every problem is reported with its line (and column) before anything is stored: an
//! unknown column, a wrong number of cells, an unreadable amount, a bad email or phone number, a reference used twice
//! in the file. A file is all or nothing. `dry_run` runs the very same checks, including the ones that need the
//! database (a reference that already belongs to a different link), and then rolls everything back.
//!
//! It is safe to send the same file twice: each row's `reference` (or, when the row has none, a reference derived
//! from its contents) is the link's `external_ref`, which is unique per business, so a second upload finds the links
//! it made the first time and creates nothing. A reference that already names a link with a different amount,
//! description or customer is an error, not a silent second link. The same file returns the same import.
//!
//! Columns (header names ignore case, spaces and punctuation; any order): `customer_name`, `customer_email`,
//! `customer_phone`, `amount` (rupees, e.g. `1250.50`; required), `description`, `reference`, `expires` (`YYYY-MM-DD`,
//! valid to the end of that day in India, or an RFC 3339 time) and `currency` (`INR` default). Customers are matched by
//! email, then by phone (a row with neither: by name alone), and created when new.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Connection;
use std::collections::HashMap;
use zyvor_core::{Currency, new_id};

use crate::{
    ApiError, AppState,
    auth::Auth,
    bankcsv::{Problem, detect_delimiter, norm_header, parse_rupees, read_records},
    payment_links::{NewLink, create_in_tx},
    tenant,
};

/// Links in one file.
pub const MAX_ROWS: usize = 500;
/// Size of the file in bytes.
pub const MAX_BYTES: usize = 512 * 1024;
const MAX_REPORTED: usize = 50;

#[derive(Deserialize, Default)]
pub struct BulkBody {
    /// The file's contents.
    csv: String,
    /// The file's name, kept with the import record.
    filename: Option<String>,
    /// Check everything and report, but store nothing.
    dry_run: Option<bool>,
    /// Applied to every link in the file (same meaning as on `POST /v1/payment_links`).
    message: Option<String>,
    thank_you: Option<String>,
    return_url: Option<String>,
    use_branding: Option<bool>,
    allowed_methods: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub line: usize,
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub amount_minor: i64,
    pub currency: String,
    pub description: Option<String>,
    pub reference: String,
    pub expires_at: Option<DateTime<Utc>>,
}

fn problem(line: usize, column: Option<&str>, message: impl Into<String>) -> Problem {
    Problem {
        line,
        column: column.map(str::to_string),
        message: message.into(),
    }
}

const COLUMNS: [(&str, &[&str]); 8] = [
    (
        "customer_name",
        &["customername", "name", "customer", "payername"],
    ),
    ("customer_email", &["customeremail", "email", "payeremail"]),
    (
        "customer_phone",
        &[
            "customerphone",
            "phone",
            "mobile",
            "mobilenumber",
            "payerphone",
        ],
    ),
    ("amount", &["amount", "amountinr", "amountrupees"]),
    ("description", &["description", "purpose"]),
    ("reference", &["reference", "ref", "externalref"]),
    (
        "expires",
        &[
            "expires",
            "expiry",
            "expiresat",
            "expireson",
            "expirydate",
            "validuntil",
        ],
    ),
    ("currency", &["currency"]),
];

fn column_of(header: &str) -> Option<&'static str> {
    let n = norm_header(header);
    COLUMNS
        .iter()
        .find(|(name, aliases)| norm_header(name) == n || aliases.contains(&n.as_str()))
        .map(|(name, _)| *name)
}

fn valid_email(e: &str) -> bool {
    let Some((local, domain)) = e.split_once('@') else {
        return false;
    };
    e.len() <= 254
        && !local.is_empty()
        && local.len() <= 64
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
        && !domain.contains('@')
        && !e
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "<>\",;".contains(c))
}

fn parse_expiry(raw: &str) -> Result<DateTime<Utc>, String> {
    if let Ok(d) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        let ist = chrono::FixedOffset::east_opt(19_800).expect("IST");
        return d
            .and_hms_opt(23, 59, 59)
            .and_then(|t| t.and_local_timezone(ist).single())
            .map(|t| t.with_timezone(&Utc))
            .ok_or_else(|| format!("`{raw}` is not a date"));
    }
    DateTime::parse_from_rfc3339(raw)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| format!("`{raw}` is not a date; use YYYY-MM-DD or a full RFC 3339 time"))
}

/// A reference for a row that gave none: the same row always gets the same one.
fn derived_reference(r: &Row) -> String {
    let key = format!(
        "v1|{}|{}|{}|{}|{}|{}|{}|{}",
        r.name.as_deref().unwrap_or(""),
        r.email.as_deref().unwrap_or("").to_lowercase(),
        r.phone.as_deref().unwrap_or(""),
        r.amount_minor,
        r.currency,
        r.description.as_deref().unwrap_or(""),
        r.expires_at.map(|e| e.to_rfc3339()).unwrap_or_default(),
        "",
    );
    format!("csv-{}", &hex::encode(Sha256::digest(key.as_bytes()))[..20])
}

/// Read the file. All problems are returned together; rows are only meaningful when there are none.
pub fn parse(csv: &str) -> (Vec<Row>, Vec<Problem>) {
    let mut errs: Vec<Problem> = Vec::new();
    if csv.len() > MAX_BYTES {
        return (
            vec![],
            vec![problem(
                0,
                None,
                format!("the file is larger than {} KB; split it", MAX_BYTES / 1024),
            )],
        );
    }
    let delim = detect_delimiter(csv);
    let records = match read_records(csv, delim) {
        Ok(r) => r,
        Err(e) => return (vec![], vec![problem(0, None, e)]),
    };
    let Some(((hline, header), body)) = records.split_first().map(|(h, b)| ((h.0, &h.1), b)) else {
        return (vec![], vec![problem(0, None, "the file is empty")]);
    };
    let mut at: HashMap<&'static str, usize> = HashMap::new();
    for (i, h) in header.iter().enumerate() {
        match column_of(h) {
            Some(c) if at.contains_key(c) => errs.push(problem(
                hline,
                Some(h),
                format!("column `{c}` appears twice"),
            )),
            Some(c) => {
                at.insert(c, i);
            }
            None => errs.push(problem(
                hline,
                Some(h),
                format!(
                    "unknown column `{h}`; expected {}",
                    COLUMNS.map(|c| c.0).join(", ")
                ),
            )),
        }
    }
    if !at.contains_key("amount") {
        errs.push(problem(hline, None, "the header has no `amount` column"));
    }
    if !errs.is_empty() {
        return (vec![], errs);
    }
    if body.is_empty() {
        return (vec![], vec![problem(0, None, "the file has no rows")]);
    }
    if body.len() > MAX_ROWS {
        return (
            vec![],
            vec![problem(
                0,
                None,
                format!(
                    "the file has {} rows; at most {MAX_ROWS} per file",
                    body.len()
                ),
            )],
        );
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (line, cells) in body {
        let line = *line;
        if cells.len() != header.len() {
            errs.push(problem(
                line,
                None,
                format!("{} cells, but the header has {}", cells.len(), header.len()),
            ));
            continue;
        }
        let get = |c: &str| -> Option<String> {
            at.get(c)
                .map(|i| cells[*i].trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let before = errs.len();
        let name = get("customer_name");
        if name
            .as_deref()
            .is_some_and(|n| n.chars().count() > 100 || n.chars().any(char::is_control))
        {
            errs.push(problem(
                line,
                Some("customer_name"),
                "the name is up to 100 characters",
            ));
        }
        let email = get("customer_email").map(|e| e.to_lowercase());
        if email.as_deref().is_some_and(|e| !valid_email(e)) {
            errs.push(problem(
                line,
                Some("customer_email"),
                "not a valid email address",
            ));
        }
        let phone = match get("customer_phone") {
            Some(p) => match crate::sms::normalise_phone(&p) {
                Some(n) => Some(n),
                None => {
                    errs.push(problem(
                        line,
                        Some("customer_phone"),
                        format!("`{p}` is not a readable phone number"),
                    ));
                    None
                }
            },
            None => None,
        };
        if name.is_none() && (email.is_some() || phone.is_some()) {
            errs.push(problem(
                line,
                Some("customer_name"),
                "a name is needed when an email or phone is given",
            ));
        }
        let amount_minor = match get("amount") {
            Some(a) => match parse_rupees(&a) {
                Ok(m) if m <= zyvor_core::MAX_AMOUNT_MINOR => m,
                Ok(_) => {
                    errs.push(problem(line, Some("amount"), "the amount is too large"));
                    0
                }
                Err(e) => {
                    errs.push(problem(line, Some("amount"), e));
                    0
                }
            },
            None => {
                errs.push(problem(line, Some("amount"), "the amount is empty"));
                0
            }
        };
        let currency = get("currency")
            .unwrap_or_else(|| "INR".into())
            .to_uppercase();
        if currency.parse::<Currency>().is_err() {
            errs.push(problem(
                line,
                Some("currency"),
                format!("unsupported currency `{currency}`; use INR or USD"),
            ));
        }
        let description = get("description");
        if description
            .as_deref()
            .is_some_and(|d| d.chars().count() > 200 || d.chars().any(char::is_control))
        {
            errs.push(problem(
                line,
                Some("description"),
                "the description is up to 200 characters, plain text",
            ));
        }
        let reference = get("reference");
        if let Some(r) = &reference
            && (r.chars().count() > 64
                || !r
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.:/#".contains(c)))
        {
            errs.push(problem(
                line,
                Some("reference"),
                "a reference is up to 64 letters, digits and - _ . : / #",
            ));
        }
        // an explicit reference is checked against earlier lines even when this line has other problems
        if let Some(r) = &reference {
            match seen.get(r) {
                Some(first) => errs.push(problem(
                    line,
                    Some("reference"),
                    format!("the same reference (or an identical row) as line {first}"),
                )),
                None => {
                    seen.insert(r.clone(), line);
                }
            }
        }
        let expires_at = match get("expires") {
            Some(e) => match parse_expiry(&e) {
                Ok(t) => Some(t),
                Err(m) => {
                    errs.push(problem(line, Some("expires"), m));
                    None
                }
            },
            None => None,
        };
        if errs.len() > before {
            continue;
        }
        let mut row = Row {
            line,
            name,
            email,
            phone,
            amount_minor,
            currency,
            description,
            reference: String::new(),
            expires_at,
        };
        match reference {
            Some(r) => row.reference = r,
            None => {
                row.reference = derived_reference(&row);
                if let Some(first) = seen.get(&row.reference) {
                    errs.push(problem(
                        line,
                        Some("reference"),
                        format!("the same reference (or an identical row) as line {first}"),
                    ));
                    continue;
                }
                seen.insert(row.reference.clone(), line);
            }
        }
        rows.push(row);
    }
    (rows, errs)
}

fn problems_json(p: &[Problem]) -> Value {
    json!(p.iter().take(MAX_REPORTED).collect::<Vec<_>>())
}

fn rejected(errs: &[Problem], dry_run: bool, rows: usize) -> Response {
    let first = errs
        .first()
        .map(|e| {
            if e.line > 0 {
                format!("line {}: {}", e.line, e.message)
            } else {
                e.message.clone()
            }
        })
        .unwrap_or_default();
    if dry_run {
        return (
            StatusCode::OK,
            Json(json!({ "dry_run": true, "ok": false, "rows": rows, "error_count": errs.len(), "errors": problems_json(errs), "stored": false })),
        )
            .into_response();
    }
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": {
            "code": "invalid_csv",
            "message": format!("no links were created: {first}"),
            "error_count": errs.len(),
            "errors": problems_json(errs),
            "stored": false,
        } })),
    )
        .into_response()
}

fn api_problem(line: usize, e: &ApiError) -> Problem {
    problem(line, None, format!("{} ({})", e.message, e.code))
}

async fn customer_for(
    tx: &mut sqlx::PgConnection,
    auth: &Auth,
    r: &Row,
) -> Result<Option<String>, ApiError> {
    let Some(name) = &r.name else {
        return Ok(None);
    };
    let found: Option<(String, Option<String>)> = if let Some(e) = &r.email {
        sqlx::query_as("SELECT id, phone FROM customers WHERE lower(email) = $1 ORDER BY created_at, id LIMIT 1")
            .bind(e)
            .fetch_optional(&mut *tx)
            .await?
    } else if let Some(p) = &r.phone {
        sqlx::query_as(
            "SELECT id, phone FROM customers WHERE phone = $1 ORDER BY created_at, id LIMIT 1",
        )
        .bind(p)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        // a customer with only a name: the same name with no contact details is the same customer
        sqlx::query_as("SELECT id, phone FROM customers WHERE name = $1 AND email IS NULL AND phone IS NULL ORDER BY created_at, id LIMIT 1")
            .bind(name)
            .fetch_optional(&mut *tx)
            .await?
    };
    if let Some((id, phone)) = found {
        if phone.is_none() && r.phone.is_some() {
            sqlx::query("UPDATE customers SET phone = $2 WHERE id = $1")
                .bind(&id)
                .bind(&r.phone)
                .execute(&mut *tx)
                .await?;
        }
        return Ok(Some(id));
    }
    let id = new_id("cus");
    sqlx::query(
        "INSERT INTO customers (id, tenant_id, name, email, phone) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&id)
    .bind(&auth.tenant_id)
    .bind(name)
    .bind(&r.email)
    .bind(&r.phone)
    .execute(&mut *tx)
    .await?;
    Ok(Some(id))
}

struct Made {
    row: Row,
    link_id: String,
    existing: bool,
}

/// `POST /v1/payment_links/bulk`: create the links in a CSV (or, with `dry_run`, say what would happen).
pub async fn bulk_create(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<BulkBody>,
) -> Result<Response, ApiError> {
    auth.require("payments:write")?;
    let dry = b.dry_run.unwrap_or(false);
    let defaults = json!([
        b.message,
        b.thank_you,
        b.return_url,
        b.use_branding,
        b.allowed_methods
    ]);
    let hash = hex::encode(Sha256::digest(
        format!("{}\n--\n{defaults}", b.csv).as_bytes(),
    ));
    let (rows, errs) = parse(&b.csv);
    if !errs.is_empty() {
        return Ok(rejected(&errs, dry, rows.len()));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let earlier: Option<String> =
        sqlx::query_scalar("SELECT id FROM payment_link_imports WHERE file_hash = $1")
            .bind(&hash)
            .fetch_optional(&mut *tx)
            .await?;
    if let (Some(id), false) = (&earlier, dry) {
        let links = import_lines(&mut tx, &s, &auth.tenant_id, id).await?;
        let (c, e) = (
            links.iter().filter(|l| l["outcome"] == "created").count(),
            links.iter().filter(|l| l["outcome"] == "existing").count(),
        );
        return Ok((
            StatusCode::OK,
            Json(json!({ "import_id": id, "repeat": true, "dry_run": false, "rows": links.len(), "created": c, "existing": e, "links": links, "export_path": format!("/v1/payment_links/imports/{id}/export.csv") })),
        )
            .into_response());
    }

    let mut problems: Vec<Problem> = Vec::new();
    let mut made: Vec<Made> = Vec::new();
    for r in rows {
        // each row in its own savepoint: one bad row must not poison the transaction the rest are judged in
        let mut sp = Connection::begin(&mut *tx).await?;
        let res: Result<Made, ApiError> = async {
            let customer_id = customer_for(&mut sp, &auth, &r).await?;
            let nl = NewLink {
                description: r.description.clone(),
                amount_minor: Some(r.amount_minor),
                currency: Some(r.currency.clone()),
                customer_id,
                expires_at: r.expires_at,
                external_ref: Some(r.reference.clone()),
                message: b.message.clone(),
                thank_you: b.thank_you.clone(),
                return_url: b.return_url.clone(),
                use_branding: b.use_branding,
                allowed_methods: b.allowed_methods.clone(),
                ..Default::default()
            };
            let c = create_in_tx(&s, &mut sp, &auth, &nl).await?;
            Ok(Made {
                row: r.clone(),
                link_id: c.id,
                existing: c.existing,
            })
        }
        .await;
        match res {
            Ok(m) => {
                sp.commit().await?;
                made.push(m);
            }
            Err(e) if e.status.is_client_error() => {
                sp.rollback().await?;
                problems.push(api_problem(r.line, &e));
            }
            Err(e) => return Err(e),
        }
    }
    if !problems.is_empty() {
        tx.rollback().await?;
        return Ok(rejected(&problems, dry, made.len() + problems.len()));
    }
    let (created, existing) = (
        made.iter().filter(|m| !m.existing).count(),
        made.iter().filter(|m| m.existing).count(),
    );
    if dry {
        let preview: Vec<Value> = made
            .iter()
            .take(20)
            .map(|m| json!({ "line": m.row.line, "reference": m.row.reference, "amount_minor": m.row.amount_minor, "currency": m.row.currency, "customer": m.row.name, "outcome": if m.existing { "existing" } else { "would_create" } }))
            .collect();
        tx.rollback().await?;
        return Ok((
            StatusCode::OK,
            Json(json!({ "dry_run": true, "ok": true, "rows": made.len(), "would_create": created, "existing": existing, "already_imported_as": earlier, "errors": [], "error_count": 0, "preview": preview, "stored": false })),
        )
            .into_response());
    }
    let import_id = new_id("pli");
    sqlx::query("INSERT INTO payment_link_imports (id, tenant_id, file_hash, filename, row_count, created_count, existing_count, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(&import_id)
        .bind(&auth.tenant_id)
        .bind(&hash)
        .bind(b.filename.as_deref().map(|f| f.chars().take(200).collect::<String>()))
        .bind(made.len() as i32)
        .bind(created as i32)
        .bind(existing as i32)
        .bind(&auth.actor)
        .execute(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict(
                "import_in_progress",
                "this file is being imported right now; send it again to see the result",
            ),
            e => e.into(),
        })?;
    for m in &made {
        sqlx::query("INSERT INTO payment_link_import_rows (import_id, tenant_id, line_no, link_id, outcome) VALUES ($1,$2,$3,$4,$5)")
            .bind(&import_id)
            .bind(&auth.tenant_id)
            .bind(m.row.line as i32)
            .bind(&m.link_id)
            .bind(if m.existing { "existing" } else { "created" })
            .execute(&mut *tx)
            .await?;
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payment_link.bulk_created",
        Some(&import_id),
        json!({ "rows": made.len(), "created": created, "existing": existing }),
    )
    .await?;
    let links = import_lines(&mut tx, &s, &auth.tenant_id, &import_id).await?;
    tx.commit().await?;
    Ok((
        if created > 0 {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(json!({ "import_id": import_id, "repeat": false, "dry_run": false, "rows": links.len(), "created": created, "existing": existing, "links": links, "export_path": format!("/v1/payment_links/imports/{import_id}/export.csv") })),
    )
        .into_response())
}

#[derive(sqlx::FromRow)]
struct ImportLine {
    line_no: i32,
    outcome: String,
    id: String,
    token: String,
    amount_minor: Option<i64>,
    currency: String,
    external_ref: Option<String>,
    description: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    canceled_at: Option<DateTime<Utc>>,
    customer_id: Option<String>,
    name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
}

async fn import_rows(
    tx: &mut sqlx::PgConnection,
    import_id: &str,
) -> Result<Vec<ImportLine>, ApiError> {
    Ok(sqlx::query_as(
        "SELECT r.line_no, r.outcome, l.id, l.token, l.amount_minor, l.currency, l.external_ref, l.description, l.expires_at, l.canceled_at, l.customer_id, c.name, c.email, c.phone
           FROM payment_link_import_rows r JOIN payment_links l ON l.id = r.link_id LEFT JOIN customers c ON c.id = l.customer_id
          WHERE r.import_id = $1 ORDER BY r.line_no",
    )
    .bind(import_id)
    .fetch_all(&mut *tx)
    .await?)
}

fn link_url(s: &AppState, tenant_id: &str, token: &str) -> (String, Option<String>) {
    let path = format!("/pay/link/{tenant_id}/{token}");
    let url = s
        .checkout_public_url
        .as_ref()
        .map(|b| format!("{}{path}", b.trim_end_matches('/')));
    (path, url)
}

async fn import_lines(
    tx: &mut sqlx::PgConnection,
    s: &AppState,
    tenant_id: &str,
    import_id: &str,
) -> Result<Vec<Value>, ApiError> {
    Ok(import_rows(tx, import_id)
        .await?
        .into_iter()
        .map(|l| {
            let (path, url) = link_url(s, tenant_id, &l.token);
            json!({ "line": l.line_no, "outcome": l.outcome, "id": l.id, "reference": l.external_ref, "path": path, "url": url, "amount_minor": l.amount_minor, "currency": l.currency, "customer_id": l.customer_id })
        })
        .collect())
}

/// A cell safe to open in a spreadsheet: quoted when needed, and never starting with a character a spreadsheet
/// would read as a formula (customer names and descriptions are typed by other people).
pub fn csv_cell(v: &str) -> String {
    let guarded = if v.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{v}")
    } else {
        v.to_string()
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

/// `GET /v1/payment_links/imports/{id}/export.csv`: the links an upload made or found, with their addresses.
pub async fn export_import(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let found: Option<String> =
        sqlx::query_scalar("SELECT id FROM payment_link_imports WHERE id = $1")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?;
    found.ok_or_else(|| ApiError::not_found("import"))?;
    let rows = import_rows(&mut tx, &id).await?;
    let mut out = String::from(
        "line,reference,outcome,link_id,url,path,amount,currency,customer_name,customer_email,customer_phone,description,expires_at,link_status\r\n",
    );
    for l in rows {
        let (path, url) = link_url(&s, &auth.tenant_id, &l.token);
        let amount = l
            .amount_minor
            .map(|a| {
                zyvor_core::minor_to_decimal(
                    a,
                    l.currency.parse().unwrap_or(zyvor_core::Currency::Inr),
                )
            })
            .unwrap_or_default();
        let status = if l.canceled_at.is_some() {
            "canceled"
        } else if l.expires_at.is_some_and(|e| e <= Utc::now()) {
            "expired"
        } else {
            "active"
        };
        let cells = [
            l.line_no.to_string(),
            l.external_ref.unwrap_or_default(),
            l.outcome,
            l.id,
            url.unwrap_or_default(),
            path,
            amount,
            l.currency,
            l.name.unwrap_or_default(),
            l.email.unwrap_or_default(),
            l.phone.unwrap_or_default(),
            l.description.unwrap_or_default(),
            l.expires_at.map(|e| e.to_rfc3339()).unwrap_or_default(),
            status.to_string(),
        ];
        out.push_str(
            &cells
                .iter()
                .map(|c| csv_cell(c))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push_str("\r\n");
    }
    let mut res = out.into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    res.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"payment-links-{id}.csv\""))
            .map_err(|_| ApiError::internal())?,
    );
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str =
        "customer_name,customer_email,customer_phone,amount,description,reference,expires\n";

    #[test]
    fn a_good_file_is_read_with_rupees_as_paise_and_a_derived_reference() {
        let (rows, errs) = parse(&format!(
            "{HEAD}Asha Rao,Asha@Example.com,98765 43210,\"1,250.50\",Term 2 fee,INV-1,2099-12-31\nBala,,,499,,,\n"
        ));
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].amount_minor, 125_050);
        assert_eq!(rows[0].email.as_deref(), Some("asha@example.com"));
        assert_eq!(rows[0].phone.as_deref(), Some("+919876543210"));
        assert_eq!(rows[0].reference, "INV-1");
        assert!(rows[0].expires_at.unwrap() > Utc::now());
        assert!(rows[1].reference.starts_with("csv-") && rows[1].reference.len() == 24);
        // the same row always gets the same reference
        let (again, _) = parse(&format!("{HEAD}Bala,,,499,,,\n"));
        assert_eq!(again[0].reference, rows[1].reference);
    }

    #[test]
    fn every_problem_is_listed_with_its_line_and_column() {
        let (rows, errs) = parse(&format!(
            "{HEAD}A,bad-email,,100,,R1,\nB,,12345,100,,R2,\n,x@y.in,,100,,R3,\nC,,,abc,,R4,\nD,,,-5,,R5,\nH,,,5,,R9,\nE,,,5,,R9,\nF,,,5,,R6,31/12/2030\nG,,,5,x,\n"
        ));
        assert!(rows.is_empty() || !errs.is_empty());
        let at = |line: usize, col: &str| {
            errs.iter()
                .any(|e| e.line == line && e.column.as_deref() == Some(col))
        };
        assert!(at(2, "customer_email"), "{errs:?}");
        assert!(at(3, "customer_phone"));
        assert!(at(4, "customer_name"));
        assert!(at(5, "amount"));
        assert!(at(6, "amount"));
        assert!(at(8, "reference"), "a repeated reference");
        assert!(at(9, "expires"));
        assert!(
            errs.iter()
                .any(|e| e.line == 10 && e.message.contains("cells"))
        );
    }

    #[test]
    fn an_unknown_column_or_a_missing_amount_stops_the_file_and_identical_rows_are_refused() {
        let (_, e) = parse("name,amount,colour\nA,1,red\n");
        assert!(
            e.iter()
                .any(|p| p.message.contains("unknown column `colour`"))
        );
        let (_, e) = parse("name,email\nA,a@b.co\n");
        assert!(e.iter().any(|p| p.message.contains("no `amount` column")));
        let (_, e) = parse("name,amount,amount\nA,1,1\n");
        assert!(e.iter().any(|p| p.message.contains("appears twice")));
        let (_, e) = parse("amount,description\n100,Fee\n100,Fee\n");
        assert!(
            e.iter()
                .any(|p| p.line == 3 && p.message.contains("line 2")),
            "{e:?}"
        );
        let (_, e) = parse("");
        assert!(!e.is_empty());
        let (_, e) = parse("amount\n");
        assert!(e.iter().any(|p| p.message.contains("no rows")));
    }

    #[test]
    fn a_date_expiry_runs_to_the_end_of_that_day_in_india() {
        let t = parse_expiry("2099-01-31").unwrap();
        assert_eq!(t.to_rfc3339(), "2099-01-31T18:29:59+00:00");
        assert!(parse_expiry("2099-01-31T10:00:00+05:30").is_ok());
        assert!(parse_expiry("31-01-2099").is_err());
    }

    #[test]
    fn exported_cells_cannot_become_formulas_and_are_quoted() {
        assert_eq!(csv_cell("=1+1"), "'=1+1");
        assert_eq!(csv_cell("+91 98"), "'+91 98");
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("plain"), "plain");
    }
}
