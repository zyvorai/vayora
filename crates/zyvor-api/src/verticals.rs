// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! School fee collection and society / association dues: one small engine, two vocabularies (`kind` is `school` or
//! `society`). A group is a class or a block, a member is a student or a resident, a fee head is an amount owed by
//! everyone in a group, and an assignment turns the heads into invoices (one, or an instalment plan with due dates).
//! Everything else is what already exists: the invoices are ordinary invoices, paid through payment links and the
//! checkout, so the existing reminders, dunning, refunds, ledger and reconciliation apply unchanged.
//!
//! Children's data is kept to a name, a class and the school's own reference (never a government ID number: a
//! twelve-digit reference is refused). The person who pays is an ordinary customer. See docs/zyvor/VERTICALS.md.

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use chrono::{Days, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use std::collections::BTreeMap;
use zyvor_core::{Currency, new_id};

use crate::{
    ApiError, AppState,
    auth::Auth,
    installments::{PlanRows, due_dates, insert_plan, split},
    storefront::{clean_email, clean_phone, clean_text},
    tenant,
};

const MAX_IMPORT_ROWS: usize = 5000;
const MAX_ASSIGN: usize = 2000;

pub(crate) fn kind_of(raw: &str) -> Result<&'static str, ApiError> {
    match raw {
        "school" => Ok("school"),
        "society" => Ok("society"),
        _ => Err(ApiError::not_found("template")),
    }
}

// ---------- CSV helpers (shared with donations) ----------

/// One CSV cell. A cell a spreadsheet would run as a formula gets a leading apostrophe; quotes are doubled.
pub(crate) fn csv_cell(s: &str) -> String {
    let mut t = s.to_string();
    if t.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        t.insert(0, '\'');
    }
    if t.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", t.replace('"', "\"\""))
    } else {
        t
    }
}

pub(crate) fn csv_line(cells: &[String]) -> String {
    cells
        .iter()
        .map(|c| csv_cell(c))
        .collect::<Vec<_>>()
        .join(",")
        + "\n"
}

/// Whole paise as rupees with two decimals, for a CSV column.
pub(crate) fn rupees(minor: i64) -> String {
    format!(
        "{}{}.{:02}",
        if minor < 0 { "-" } else { "" },
        minor.abs() / 100,
        minor.abs() % 100
    )
}

pub(crate) fn csv_response(name: &str, body: String) -> Result<Response, ApiError> {
    let mut res = Response::new(Body::from(body));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
            .map_err(|_| ApiError::internal())?,
    );
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(res)
}

fn member_ref_ok(r: &str) -> Result<(), ApiError> {
    let shape = !r.is_empty()
        && r.len() <= 30
        && r.starts_with(|c: char| c.is_ascii_alphanumeric())
        && r.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-'));
    if !shape {
        return Err(ApiError::invalid(
            "invalid_ref",
            "the reference is 1 to 30 letters, digits, / _ or -, starting with a letter or digit",
        ));
    }
    if r.len() == 12 && r.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ApiError::invalid(
            "invalid_ref",
            "use the school's or society's own reference, never a government ID number",
        ));
    }
    Ok(())
}

// ---------- groups and heads ----------

#[derive(Deserialize)]
pub struct DuesNameIn {
    name: String,
}

/// `POST /v1/dues/{kind}/groups`: a class (school) or a block / wing (society).
pub async fn create_group(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
    Json(b): Json<DuesNameIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("verticals:write")?;
    let kind = kind_of(&kind)?;
    let name = clean_text(&b.name, "name", 40, false, true)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let id = new_id("dgr");
    sqlx::query("INSERT INTO dues_groups (id, tenant_id, kind, name) VALUES ($1,$2,$3,$4)")
        .bind(&id)
        .bind(&auth.tenant_id)
        .bind(kind)
        .bind(&name)
        .execute(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => {
                ApiError::conflict("duplicate", "a group with this name exists")
            }
            e => e.into(),
        })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "dues.group_created",
        Some(&id),
        json!({ "kind": kind }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "name": name, "kind": kind })),
    ))
}

/// `GET /v1/dues/{kind}/groups`
pub async fn list_groups(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verticals:read")?;
    let kind = kind_of(&kind)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT g.id, g.name, (SELECT count(*) FROM dues_members m WHERE m.group_id = g.id AND m.active), (SELECT COALESCE(SUM(amount_minor), 0)::BIGINT FROM dues_heads h WHERE h.group_id = g.id)
           FROM dues_groups g WHERE g.kind = $1 ORDER BY g.name",
    )
    .bind(kind)
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, name, members, heads)| json!({ "id": id, "name": name, "members": members, "heads_total_minor": heads })).collect::<Vec<_>>() }),
    ))
}

#[derive(Deserialize)]
pub struct DuesHeadIn {
    group_id: String,
    name: String,
    amount_minor: i64,
}

/// `POST /v1/dues/{kind}/heads`: a fee head (tuition, transport) or a due (maintenance, sinking fund) for one group.
pub async fn create_head(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
    Json(b): Json<DuesHeadIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("verticals:write")?;
    let kind = kind_of(&kind)?;
    let name = clean_text(&b.name, "name", 60, false, true)?;
    if b.amount_minor <= 0 || b.amount_minor > zyvor_core::MAX_AMOUNT_MINOR {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_minor must be a positive whole number of paise",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let ok: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM dues_groups WHERE id = $1 AND kind = $2)")
            .bind(&b.group_id)
            .bind(kind)
            .fetch_one(&mut *tx)
            .await?;
    if !ok {
        return Err(ApiError::not_found("group"));
    }
    let id = new_id("dhd");
    sqlx::query("INSERT INTO dues_heads (id, tenant_id, kind, group_id, name, amount_minor) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(&id)
        .bind(&auth.tenant_id)
        .bind(kind)
        .bind(&b.group_id)
        .bind(&name)
        .bind(b.amount_minor)
        .execute(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("duplicate", "this group already has a head with this name"),
            e => e.into(),
        })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "dues.head_created",
        Some(&id),
        json!({ "kind": kind, "amount_minor": b.amount_minor }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "group_id": b.group_id, "name": name, "amount_minor": b.amount_minor }),
        ),
    ))
}

#[derive(Deserialize)]
pub struct DuesGroupQuery {
    group_id: Option<String>,
    limit: Option<i64>,
}

/// `GET /v1/dues/{kind}/heads`
pub async fn list_heads(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
    Query(q): Query<DuesGroupQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verticals:read")?;
    let kind = kind_of(&kind)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, i64)> = sqlx::query_as("SELECT id, group_id, name, amount_minor FROM dues_heads WHERE kind = $1 AND ($2::text IS NULL OR group_id = $2) ORDER BY group_id, name").bind(kind).bind(q.group_id).fetch_all(&mut *tx).await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, g, n, a)| json!({ "id": id, "group_id": g, "name": n, "amount_minor": a })).collect::<Vec<_>>() }),
    ))
}

// ---------- members ----------

struct MemberFields {
    group: String,
    name: String,
    member_ref: String,
    payer_name: Option<String>,
    payer_email: Option<String>,
    payer_phone: Option<String>,
}

/// Add or update a member by reference; the group is made when new. A payer given by name becomes (or updates) a
/// customer for this member. Returns whether the member is new.
async fn upsert_member(
    tx: &mut PgConnection,
    tenant_id: &str,
    kind: &str,
    f: &MemberFields,
    payer_customer_id: Option<&str>,
) -> Result<(String, bool), ApiError> {
    let group_id: String = sqlx::query_scalar("INSERT INTO dues_groups (id, tenant_id, kind, name) VALUES ($1,$2,$3,$4) ON CONFLICT (tenant_id, kind, name) DO UPDATE SET name = EXCLUDED.name RETURNING id")
        .bind(new_id("dgr"))
        .bind(tenant_id)
        .bind(kind)
        .bind(&f.group)
        .fetch_one(&mut *tx)
        .await?;
    let mut payer: Option<String> = payer_customer_id.map(str::to_string);
    if let Some(pn) = &f.payer_name {
        let ext = format!("dues:{kind}:{}", f.member_ref);
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM customers WHERE external_ref = $1 LIMIT 1")
                .bind(&ext)
                .fetch_optional(&mut *tx)
                .await?;
        let cid = match existing {
            Some(id) => {
                sqlx::query("UPDATE customers SET name = $2, email = COALESCE($3, email), phone = COALESCE($4, phone) WHERE id = $1").bind(&id).bind(pn).bind(&f.payer_email).bind(&f.payer_phone).execute(&mut *tx).await?;
                id
            }
            None => {
                let id = new_id("cus");
                sqlx::query("INSERT INTO customers (id, tenant_id, name, email, phone, external_ref) VALUES ($1,$2,$3,$4,$5,$6)").bind(&id).bind(tenant_id).bind(pn).bind(&f.payer_email).bind(&f.payer_phone).bind(&ext).execute(&mut *tx).await?;
                id
            }
        };
        payer = Some(cid);
    }
    let (id, created): (String, bool) = sqlx::query_as(
        "INSERT INTO dues_members (id, tenant_id, kind, group_id, name, member_ref, payer_customer_id) VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (tenant_id, kind, member_ref) DO UPDATE SET group_id = EXCLUDED.group_id, name = EXCLUDED.name,
            payer_customer_id = COALESCE(EXCLUDED.payer_customer_id, dues_members.payer_customer_id), active = true
         RETURNING id, (xmax = 0)",
    )
    .bind(new_id("dmb"))
    .bind(tenant_id)
    .bind(kind)
    .bind(&group_id)
    .bind(&f.name)
    .bind(&f.member_ref)
    .bind(&payer)
    .fetch_one(&mut *tx)
    .await?;
    Ok((id, created))
}

fn fields(
    group: &str,
    name: &str,
    member_ref: &str,
    pn: Option<&str>,
    pe: Option<&str>,
    pp: Option<&str>,
) -> Result<MemberFields, ApiError> {
    fn opt(v: Option<&str>) -> Option<&str> {
        v.map(str::trim).filter(|x| !x.is_empty())
    }
    member_ref_ok(member_ref.trim())?;
    Ok(MemberFields {
        group: clean_text(group, "group", 40, false, true)?,
        name: clean_text(name, "name", 80, false, true)?,
        member_ref: member_ref.trim().to_string(),
        payer_name: opt(pn)
            .map(|v| clean_text(v, "payer name", 80, false, true))
            .transpose()?,
        payer_email: opt(pe).map(clean_email).transpose()?,
        payer_phone: opt(pp).map(clean_phone).transpose()?,
    })
}

#[derive(Deserialize)]
pub struct DuesMemberIn {
    /// The class or block by name; made if new.
    group: String,
    name: String,
    /// The school's or society's own reference (roll number, flat number). Unique per template.
    member_ref: String,
    /// An existing customer who pays; or give the payer's details below.
    payer_customer_id: Option<String>,
    payer_name: Option<String>,
    payer_email: Option<String>,
    payer_phone: Option<String>,
}

/// `POST /v1/dues/{kind}/members`: add a student or resident, or update the one with this reference.
pub async fn upsert(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
    Json(b): Json<DuesMemberIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("verticals:write")?;
    let kind = kind_of(&kind)?;
    let f = fields(
        &b.group,
        &b.name,
        &b.member_ref,
        b.payer_name.as_deref(),
        b.payer_email.as_deref(),
        b.payer_phone.as_deref(),
    )?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    if let Some(c) = &b.payer_customer_id {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM customers WHERE id = $1)")
            .bind(c)
            .fetch_one(&mut *tx)
            .await?;
        if !ok {
            return Err(ApiError::not_found("customer"));
        }
    }
    let (id, created) = upsert_member(
        &mut tx,
        &auth.tenant_id,
        kind,
        &f,
        b.payer_customer_id.as_deref(),
    )
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "dues.member_saved",
        Some(&id),
        json!({ "kind": kind, "created": created }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(json!({ "id": id, "created": created })),
    ))
}

type MemberRow = (
    String,
    String,
    String,
    String,
    String,
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
);

async fn member_rows(
    conn: &mut PgConnection,
    kind: &str,
    group: Option<&str>,
    limit: i64,
) -> Result<Vec<MemberRow>, ApiError> {
    Ok(sqlx::query_as(
        "SELECT m.id, g.id, g.name, m.member_ref, m.name, m.active, c.name, c.email, c.phone
           FROM dues_members m JOIN dues_groups g ON g.id = m.group_id LEFT JOIN customers c ON c.id = m.payer_customer_id
          WHERE m.kind = $1 AND ($2::text IS NULL OR m.group_id = $2) ORDER BY g.name, m.member_ref LIMIT $3",
    )
    .bind(kind)
    .bind(group)
    .bind(limit)
    .fetch_all(conn)
    .await?)
}

/// `GET /v1/dues/{kind}/members`
pub async fn list_members(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
    Query(q): Query<DuesGroupQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verticals:read")?;
    let kind = kind_of(&kind)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows = member_rows(
        &mut tx,
        kind,
        q.group_id.as_deref(),
        q.limit.unwrap_or(200).clamp(1, 1000),
    )
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|r| json!({ "id": r.0, "group_id": r.1, "group": r.2, "member_ref": r.3, "name": r.4, "active": r.5, "payer": r.6.map(|n| json!({ "name": n, "email": r.7, "phone": r.8 })) })).collect::<Vec<_>>() }),
    ))
}

/// `GET /v1/dues/{kind}/members.csv`: the same list as a file the school or society can edit and import again.
pub async fn export_members(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
) -> Result<Response, ApiError> {
    auth.require("verticals:read")?;
    let kind = kind_of(&kind)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows = member_rows(&mut tx, kind, None, 20_000).await?;
    let mut out = csv_line(
        &[
            "group",
            "member_ref",
            "name",
            "payer_name",
            "payer_email",
            "payer_phone",
        ]
        .map(String::from),
    );
    for r in rows {
        out.push_str(&csv_line(&[
            r.2,
            r.3,
            r.4,
            r.6.unwrap_or_default(),
            r.7.unwrap_or_default(),
            r.8.unwrap_or_default(),
        ]));
    }
    csv_response(&format!("{kind}-members.csv"), out)
}

#[derive(Deserialize)]
pub struct DuesImportIn {
    /// The file's contents. Header row first; see docs/zyvor/VERTICALS.md for the accepted column names.
    csv: String,
    /// Check the file and report, saving nothing.
    dry_run: Option<bool>,
}

fn header_key(h: &str) -> Option<&'static str> {
    let k: String = h
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    Some(match k.as_str() {
        "group" | "class" | "classname" | "block" | "wing" | "building" => "group",
        "name" | "studentname" | "student" | "membername" | "member" | "resident"
        | "residentname" => "name",
        "memberref" | "ref" | "reference" | "roll" | "rollno" | "rollnumber" | "admissionno"
        | "studentref" | "flat" | "flatno" | "unit" | "unitno" => "ref",
        "payername" | "guardian" | "guardianname" | "parent" | "parentname" | "payer" => {
            "payer_name"
        }
        "payeremail" | "guardianemail" | "parentemail" | "email" => "payer_email",
        "payerphone" | "guardianphone" | "parentphone" | "phone" | "mobile" => "payer_phone",
        _ => return None,
    })
}

/// `POST /v1/dues/{kind}/members/import`: add or update many students or residents from a CSV. All or nothing: a
/// file with any problem saves nothing and lists each problem with its line.
pub async fn import_members(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
    Json(b): Json<DuesImportIn>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verticals:write")?;
    let kind = kind_of(&kind)?;
    if b.csv.len() > 1_000_000 {
        return Err(ApiError::invalid(
            "file_too_large",
            "the file is limited to 1 MB",
        ));
    }
    let delim = crate::bankcsv::detect_delimiter(&b.csv);
    let records = crate::bankcsv::read_records(&b.csv, delim)
        .map_err(|e| ApiError::invalid("invalid_csv", e))?;
    let mut it = records.into_iter();
    let (_, head) = it
        .next()
        .ok_or_else(|| ApiError::invalid("invalid_csv", "the file is empty"))?;
    let mut col: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (i, h) in head.iter().enumerate() {
        if let Some(k) = header_key(h) {
            col.entry(k).or_insert(i);
        }
    }
    for need in ["group", "name", "ref"] {
        if !col.contains_key(need) {
            return Err(ApiError::invalid(
                "missing_column",
                format!(
                    "the header row needs a column for {}",
                    match need {
                        "group" => "the class or block",
                        "name" => "the name",
                        _ => "the reference",
                    }
                ),
            ));
        }
    }
    let rows: Vec<_> = it.collect();
    if rows.len() > MAX_IMPORT_ROWS {
        return Err(ApiError::invalid(
            "too_many_rows",
            format!("at most {MAX_IMPORT_ROWS} rows per file"),
        ));
    }
    let get = |r: &'_ [String], k: &str| -> Option<String> {
        col.get(k).and_then(|i| r.get(*i)).cloned()
    };
    let (mut parsed, mut problems, mut seen) =
        (Vec::new(), Vec::new(), std::collections::BTreeSet::new());
    for (line, r) in &rows {
        let (g, n, rf, pn, pe, pp) = (
            get(r, "group"),
            get(r, "name"),
            get(r, "ref"),
            get(r, "payer_name"),
            get(r, "payer_email"),
            get(r, "payer_phone"),
        );
        match fields(
            g.as_deref().unwrap_or(""),
            n.as_deref().unwrap_or(""),
            rf.as_deref().unwrap_or(""),
            pn.as_deref(),
            pe.as_deref(),
            pp.as_deref(),
        ) {
            Ok(f) if !seen.insert(f.member_ref.clone()) => problems.push(
                json!({ "line": line, "message": "this reference appears twice in the file" }),
            ),
            Ok(f) => parsed.push(f),
            Err(e) => problems.push(json!({ "line": line, "message": e.message })),
        }
    }
    if !problems.is_empty() {
        return Ok(Json(
            json!({ "saved": false, "rows": rows.len(), "problems": problems }),
        ));
    }
    if b.dry_run.unwrap_or(false) {
        return Ok(Json(
            json!({ "saved": false, "dry_run": true, "rows": parsed.len(), "problems": [] }),
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (mut created, mut updated) = (0, 0);
    for f in &parsed {
        if upsert_member(&mut tx, &auth.tenant_id, kind, f, None)
            .await?
            .1
        {
            created += 1;
        } else {
            updated += 1;
        }
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "dues.members_imported",
        None,
        json!({ "kind": kind, "created": created, "updated": updated }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "saved": true, "created": created, "updated": updated, "problems": [] }),
    ))
}

// ---------- assignment ----------

#[derive(Deserialize)]
pub struct DuesAssignIn {
    /// Names the term or period: "2026-27 Term 1", "Apr-Jun 2026". One assignment per member per label.
    label: String,
    /// Everyone active in this group, or just these members.
    group_id: Option<String>,
    member_ids: Option<Vec<String>>,
    /// Only these heads (default: all of the member's group).
    head_ids: Option<Vec<String>>,
    /// 1 to 60. One means a single invoice.
    instalments: i32,
    first_due_date: NaiveDate,
    /// `month` (default) or `day`.
    every_unit: Option<String>,
    every: Option<u32>,
    /// Invoice numbers become `{number_prefix}-{reference}-{n}`.
    number_prefix: String,
}

fn number_part(r: &str) -> String {
    r.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// `POST /v1/dues/{kind}/assign`: turn the fee heads into invoices for a group or some members. Safe to repeat: a
/// member who already has this label is left alone.
pub async fn assign(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
    Json(b): Json<DuesAssignIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("verticals:write")?;
    let kind = kind_of(&kind)?;
    let label = clean_text(&b.label, "label", 60, false, true)?;
    let prefix = clean_text(&b.number_prefix, "number_prefix", 20, false, true)?;
    if !prefix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(ApiError::invalid(
            "invalid_prefix",
            "the number prefix is letters, digits and hyphens",
        ));
    }
    let unit = b.every_unit.as_deref().unwrap_or("month");
    let every = b.every.unwrap_or(1);
    if !(1..=60).contains(&b.instalments)
        || !matches!(unit, "month" | "day")
        || every == 0
        || every > 366
    {
        return Err(ApiError::invalid(
            "invalid_plan",
            "instalments is 1 to 60; every_unit is month or day; every is 1 to 366",
        ));
    }
    if b.group_id.is_some() == b.member_ids.is_some() {
        return Err(ApiError::invalid(
            "invalid_target",
            "give exactly one of group_id or member_ids",
        ));
    }
    let dates = due_dates(b.first_due_date, b.instalments, unit, every)
        .ok_or_else(|| ApiError::invalid("invalid_plan", "due dates are out of range"))?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let members: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, group_id, member_ref, payer_customer_id FROM dues_members WHERE kind = $1 AND active
            AND (($2::text IS NOT NULL AND group_id = $2) OR ($3::text[] IS NOT NULL AND id = ANY($3))) ORDER BY member_ref LIMIT $4",
    )
    .bind(kind)
    .bind(&b.group_id)
    .bind(&b.member_ids)
    .bind(MAX_ASSIGN as i64 + 1)
    .fetch_all(&mut *tx)
    .await?;
    if members.len() > MAX_ASSIGN {
        return Err(ApiError::invalid(
            "too_many_members",
            format!("at most {MAX_ASSIGN} members per call; assign a group at a time"),
        ));
    }
    if members.is_empty() {
        return Err(ApiError::not_found("member"));
    }
    let (mut created, mut already, mut no_payer, mut no_heads) = (0, 0, 0, 0);
    for (member, group, mref, payer) in members {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM dues_assignments WHERE member_id = $1 AND label = $2)",
        )
        .bind(&member)
        .bind(&label)
        .fetch_one(&mut *tx)
        .await?;
        if exists {
            already += 1;
            continue;
        }
        let Some(payer) = payer else {
            no_payer += 1;
            continue;
        };
        let heads: Vec<(String, i64)> = sqlx::query_as("SELECT name, amount_minor FROM dues_heads WHERE group_id = $1 AND ($2::text[] IS NULL OR id = ANY($2)) ORDER BY name").bind(&group).bind(&b.head_ids).fetch_all(&mut *tx).await?;
        let total: i64 = heads.iter().map(|h| h.1).sum();
        if heads.is_empty() || b.instalments as i64 > total {
            no_heads += 1;
            continue;
        }
        let amounts = split(total, b.instalments);
        let assignment = new_id("das");
        let num = format!("{prefix}-{}", number_part(&mref));
        let invoices: Vec<String> = if b.instalments == 1 {
            let inv = new_id("inv");
            sqlx::query("INSERT INTO invoices (id, tenant_id, customer_id, number, currency, amount_minor, due_date, external_ref) VALUES ($1,$2,$3,$4,'INR',$5,$6,$7)")
                .bind(&inv)
                .bind(&auth.tenant_id)
                .bind(&payer)
                .bind(format!("{num}-1"))
                .bind(total)
                .bind(dates[0])
                .bind(format!("{assignment}:1"))
                .execute(&mut *tx)
                .await
                .map_err(number_taken)?;
            vec![inv]
        } else {
            let (plan_id, invs) = insert_plan(
                &mut tx,
                &auth.tenant_id,
                &auth.actor,
                &PlanRows {
                    customer_id: &payer,
                    name: &label,
                    total_minor: total,
                    currency: Currency::Inr.code(),
                    instalments: b.instalments,
                    external_ref: Some(&assignment),
                    number_prefix: &num,
                    amounts: &amounts,
                    dates: &dates,
                },
            )
            .await
            .map_err(|e| {
                if e.code == "duplicate" {
                    ApiError::conflict(
                        "number_taken",
                        "an invoice number is already used; choose a different number_prefix",
                    )
                } else {
                    e
                }
            })?;
            sqlx::query("INSERT INTO dues_assignments (id, tenant_id, member_id, label, total_minor, instalments, plan_id, heads, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
                .bind(&assignment)
                .bind(&auth.tenant_id)
                .bind(&member)
                .bind(&label)
                .bind(total)
                .bind(b.instalments)
                .bind(&plan_id)
                .bind(json!(heads.iter().map(|h| json!({ "name": h.0, "amount_minor": h.1 })).collect::<Vec<_>>()))
                .bind(&auth.actor)
                .execute(&mut *tx)
                .await?;
            link_invoices(&mut tx, &auth.tenant_id, &assignment, &invs).await?;
            created += 1;
            continue;
        };
        sqlx::query("INSERT INTO dues_assignments (id, tenant_id, member_id, label, total_minor, instalments, heads, created_by) VALUES ($1,$2,$3,$4,$5,1,$6,$7)")
            .bind(&assignment)
            .bind(&auth.tenant_id)
            .bind(&member)
            .bind(&label)
            .bind(total)
            .bind(json!(heads.iter().map(|h| json!({ "name": h.0, "amount_minor": h.1 })).collect::<Vec<_>>()))
            .bind(&auth.actor)
            .execute(&mut *tx)
            .await?;
        link_invoices(&mut tx, &auth.tenant_id, &assignment, &invoices).await?;
        created += 1;
    }
    tenant::audit(&mut tx, &auth.tenant_id, &auth.actor, "dues.assigned", None, json!({ "kind": kind, "created": created, "already": already, "no_payer": no_payer, "no_heads": no_heads })).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "created": created, "already_assigned": already, "skipped_no_payer": no_payer, "skipped_no_fee_heads": no_heads }),
        ),
    ))
}

fn number_taken(e: sqlx::Error) -> ApiError {
    match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict(
            "number_taken",
            "an invoice number is already used; choose a different number_prefix",
        ),
        e => e.into(),
    }
}

async fn link_invoices(
    tx: &mut PgConnection,
    tenant_id: &str,
    assignment: &str,
    invoices: &[String],
) -> Result<(), ApiError> {
    for (i, inv) in invoices.iter().enumerate() {
        sqlx::query("INSERT INTO dues_assignment_invoices (invoice_id, tenant_id, assignment_id, installment_no) VALUES ($1,$2,$3,$4)").bind(inv).bind(tenant_id).bind(assignment).bind(i as i32 + 1).execute(&mut *tx).await?;
    }
    Ok(())
}

// ---------- the paid / due report ----------

#[derive(Deserialize)]
pub struct DuesReportQuery {
    group_id: Option<String>,
    label: Option<String>,
    /// `due` keeps only members who still owe something.
    status: Option<String>,
    limit: Option<i64>,
}

async fn report(
    conn: &mut PgConnection,
    kind: &str,
    q: &DuesReportQuery,
) -> Result<Value, ApiError> {
    let today = crate::clock::today(conn).await?;
    let limit = q.limit.unwrap_or(500).clamp(1, 2000);
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<i32>, Option<i64>, Option<i64>, Option<NaiveDate>, Option<String>)> = sqlx::query_as(
        "SELECT m.id, m.name, m.group_name, m.member_ref, a.id, a.label, i.id, i.number, ai.installment_no, i.amount_minor,
                LEAST(invoice_settled(i.id), i.amount_minor)::BIGINT, i.due_date, i.status
           FROM (SELECT m.id, m.name, g.name AS group_name, m.member_ref FROM dues_members m JOIN dues_groups g ON g.id = m.group_id
                  WHERE m.kind = $1 AND ($2::text IS NULL OR m.group_id = $2) ORDER BY g.name, m.member_ref LIMIT $4) m
           LEFT JOIN dues_assignments a ON a.member_id = m.id AND ($3::text IS NULL OR a.label = $3)
           LEFT JOIN dues_assignment_invoices ai ON ai.assignment_id = a.id
           LEFT JOIN invoices i ON i.id = ai.invoice_id AND i.status <> 'void'
          ORDER BY m.group_name, m.member_ref, a.label, ai.installment_no",
    )
    .bind(kind)
    .bind(&q.group_id)
    .bind(&q.label)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    struct M {
        v: Value,
        billed: i64,
        paid: i64,
        overdue: i64,
        assignments: Vec<(String, String, Vec<Value>, i64, i64)>,
    }
    let mut members: Vec<M> = Vec::new();
    for (mid, name, group, mref, aid, label, iid, number, no, amount, paid, due, status) in rows {
        if members.last().is_none_or(|m| m.v["id"] != mid) {
            members.push(M {
                v: json!({ "id": mid, "name": name, "group": group, "member_ref": mref }),
                billed: 0,
                paid: 0,
                overdue: 0,
                assignments: vec![],
            });
        }
        let m = members.last_mut().expect("just pushed");
        let (Some(aid), Some(label), Some(iid), Some(amount), Some(paid)) =
            (aid, label, iid, amount, paid)
        else {
            continue;
        };
        m.billed += amount;
        m.paid += paid;
        if due.is_some_and(|d| d < today) {
            m.overdue += amount - paid;
        }
        if m.assignments.last().is_none_or(|a| a.0 != aid) {
            m.assignments.push((aid.clone(), label, vec![], 0, 0));
        }
        let a = m.assignments.last_mut().expect("just pushed");
        a.3 += amount;
        a.4 += paid;
        a.2.push(json!({ "invoice_id": iid, "number": number, "installment": no, "amount_minor": amount, "paid_minor": paid, "due_minor": amount - paid, "due_date": due, "status": status }));
    }
    let want_due = q.status.as_deref() == Some("due");
    let (mut tb, mut tp, mut to) = (0, 0, 0);
    let mut data = Vec::new();
    for m in members {
        if want_due && m.billed - m.paid <= 0 {
            continue;
        }
        tb += m.billed;
        tp += m.paid;
        to += m.overdue;
        let mut v = m.v;
        v["billed_minor"] = json!(m.billed);
        v["paid_minor"] = json!(m.paid);
        v["due_minor"] = json!(m.billed - m.paid);
        v["overdue_minor"] = json!(m.overdue);
        v["assignments"] = json!(m.assignments.into_iter().map(|(id, label, invoices, total, paid)| json!({ "id": id, "label": label, "total_minor": total, "paid_minor": paid, "due_minor": total - paid, "invoices": invoices })).collect::<Vec<_>>());
        data.push(v);
    }
    Ok(
        json!({ "kind": kind, "as_of": today, "totals": { "billed_minor": tb, "paid_minor": tp, "due_minor": tb - tp, "overdue_minor": to, "members": data.len() }, "data": data }),
    )
}

/// `GET /v1/dues/{kind}/report`: per member, what was billed, what is paid, what is due and what is overdue.
pub async fn get_report(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
    Query(q): Query<DuesReportQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verticals:read")?;
    let kind = kind_of(&kind)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(report(&mut tx, kind, &q).await?))
}

/// `GET /v1/dues/{kind}/report.csv`: one row per member.
pub async fn report_csv(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
    Query(q): Query<DuesReportQuery>,
) -> Result<Response, ApiError> {
    auth.require("verticals:read")?;
    let kind = kind_of(&kind)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let r = report(&mut tx, kind, &q).await?;
    let mut out = csv_line(
        &[
            "group",
            "member_ref",
            "name",
            "billed",
            "paid",
            "due",
            "overdue",
        ]
        .map(String::from),
    );
    for m in r["data"].as_array().into_iter().flatten() {
        let n = |k: &str| rupees(m[k].as_i64().unwrap_or(0));
        out.push_str(&csv_line(&[
            m["group"].as_str().unwrap_or("").into(),
            m["member_ref"].as_str().unwrap_or("").into(),
            m["name"].as_str().unwrap_or("").into(),
            n("billed_minor"),
            n("paid_minor"),
            n("due_minor"),
            n("overdue_minor"),
        ]));
    }
    csv_response(&format!("{kind}-report.csv"), out)
}

// ---------- reminders and receipts ----------

#[derive(Deserialize, Default)]
pub struct DuesRemindIn {
    group_id: Option<String>,
    label: Option<String>,
    /// Also remind for instalments falling due within this many days (default 3, at most 30). Overdue ones are always included.
    days_before: Option<u64>,
}

/// `POST /v1/dues/{kind}/reminders`: email the payer of every unpaid instalment that is due soon or overdue, once per
/// invoice per day, with a payment link when the checkout address is configured. People who unsubscribed are skipped.
/// Answers with counts only.
pub async fn send_reminders(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
    body: Option<Json<DuesRemindIn>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verticals:write")?;
    let kind = kind_of(&kind)?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let ahead = b.days_before.unwrap_or(3).min(30);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let today = crate::clock::today(&mut tx).await?;
    let until = today.checked_add_days(Days::new(ahead)).unwrap_or(today);
    #[allow(clippy::type_complexity)]
    let due: Vec<(String, String, i64, i64, NaiveDate, String, String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT i.id, i.number, i.amount_minor, invoice_settled(i.id), i.due_date, m.name, g.name, c.name, c.email, i.customer_id
           FROM dues_assignment_invoices ai JOIN dues_assignments a ON a.id = ai.assignment_id
           JOIN dues_members m ON m.id = a.member_id AND m.kind = $1 JOIN dues_groups g ON g.id = m.group_id
           JOIN invoices i ON i.id = ai.invoice_id JOIN customers c ON c.id = i.customer_id
          WHERE i.status IN ('open', 'partially_paid') AND i.due_date IS NOT NULL AND i.due_date <= $2
            AND ($3::text IS NULL OR a.label = $3) AND ($4::text IS NULL OR m.group_id = $4)
          ORDER BY i.due_date, i.number LIMIT 500",
    )
    .bind(kind)
    .bind(until)
    .bind(&b.label)
    .bind(&b.group_id)
    .fetch_all(&mut *tx)
    .await?;
    let merchant: String = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1").bind(&auth.tenant_id).fetch_optional(&mut *tx).await?.unwrap_or_else(|| "the business".into());
    let who = if kind == "school" {
        "Student"
    } else {
        "Member"
    };
    let (mut queued, mut no_email, mut already) = (0, 0, 0);
    for (inv, number, amount, settled, due_date, member, group, payer, email, customer) in due {
        let Some(email) = email.filter(|e| e.contains('@')) else {
            no_email += 1;
            continue;
        };
        let owed = (amount - settled).max(0);
        let when = if due_date < today {
            format!("was due on {due_date}")
        } else {
            format!("is due on {due_date}")
        };
        let mut wording =
            format!("This is a reminder that a payment {when}.\n{who}: {member} ({group})");
        if let Some(base) = &s.checkout_public_url {
            let token = crate::payment_links::ensure_invoice_link(
                &mut tx,
                &auth.tenant_id,
                &inv,
                customer.as_deref(),
                "INR",
            )
            .await?;
            wording.push_str(&format!(
                "\nPay online: {}/pay/link/{}/{token}",
                base.trim_end_matches('/'),
                auth.tenant_id
            ));
        }
        let (subject, text) = crate::mail::reminder(
            &merchant,
            &payer,
            &wording,
            &number,
            &crate::ai::money(owed, "INR"),
        );
        if crate::mail::enqueue_once_as(
            &mut tx,
            &auth.tenant_id,
            &format!("dues_remind:{inv}:{today}"),
            &email,
            &subject,
            &text,
            true,
        )
        .await?
        {
            queued += 1;
        } else {
            already += 1;
        }
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "dues.reminders_sent",
        None,
        json!({ "kind": kind, "queued": queued }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "queued": queued, "skipped_no_email": no_email, "skipped_already_sent_today": already }),
    ))
}

/// `GET /v1/dues/{kind}/invoices/{invoice_id}/receipt.pdf`: a receipt for what has been received on one instalment.
pub async fn receipt_pdf(
    State(s): State<AppState>,
    auth: Auth,
    Path((kind, invoice_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    auth.require("verticals:read")?;
    let kind = kind_of(&kind)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let row: Option<(String, i64, i64, String, String, String, String, i32, i32, String, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT i.number, i.amount_minor, invoice_settled(i.id), m.name, g.name, a.label, COALESCE(c.name, ''), ai.installment_no, a.instalments, i.id,
                (SELECT max(created_at) FROM invoice_allocations WHERE invoice_id = i.id AND amount_minor > 0)
           FROM dues_assignment_invoices ai JOIN dues_assignments a ON a.id = ai.assignment_id JOIN dues_members m ON m.id = a.member_id AND m.kind = $1
           JOIN dues_groups g ON g.id = m.group_id JOIN invoices i ON i.id = ai.invoice_id LEFT JOIN customers c ON c.id = i.customer_id WHERE i.id = $2",
    )
    .bind(kind)
    .bind(&invoice_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (number, amount, settled, member, group, label, payer, no, of, _, paid_at) =
        row.ok_or_else(|| ApiError::not_found("invoice"))?;
    if settled <= 0 {
        return Err(ApiError::conflict(
            "nothing_paid",
            "nothing has been received on this instalment yet",
        ));
    }
    let merchant: String = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1").bind(&auth.tenant_id).fetch_optional(&mut *tx).await?.unwrap_or_default();
    let note: String =
        sqlx::query_scalar("SELECT receipt_note FROM vertical_settings WHERE kind = $1")
            .bind(kind)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or_default();
    let ist = chrono::FixedOffset::east_opt(19_800).expect("IST");
    let date = paid_at
        .map(|t| t.with_timezone(&ist).date_naive())
        .unwrap_or_else(crate::ist_today);
    let (title, who, grp) = if kind == "school" {
        ("Fee receipt", "Student", "Class")
    } else {
        ("Receipt", "Member", "Block")
    };
    let money = |m: i64| crate::ai::money(m, "INR");
    let mut doc = vec![
        format!("From: {merchant}"),
        format!("Receipt number: RC-{number}"),
        format!("Date: {date}"),
        String::new(),
        format!("Received from: {payer}"),
        format!("{who}: {member}"),
        format!("{grp}: {group}"),
        format!("For: {label}, instalment {no} of {of}"),
        String::new(),
        format!("Amount due on this instalment: {}", money(amount)),
        format!("Amount received to date: {}", money(settled.min(amount))),
        format!("Balance: {}", money((amount - settled).max(0))),
    ];
    if !note.trim().is_empty() {
        doc.push(String::new());
        doc.extend(note.lines().map(str::to_string));
    }
    let pdf = crate::pdf::text_pdf(title, &doc);
    crate::uploads::download_response(
        &format!("RC-{}.pdf", number_part(&number)),
        "application/pdf",
        pdf,
    )
}

#[derive(Deserialize)]
pub struct DuesSettingsIn {
    /// Words printed on every receipt, as typed (a registration line, an address). At most 300 characters.
    receipt_note: String,
}

/// `PUT /v1/dues/{kind}/settings`
pub async fn put_settings(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
    Json(b): Json<DuesSettingsIn>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verticals:write")?;
    let kind = kind_of(&kind)?;
    let note = clean_text(&b.receipt_note, "receipt_note", 300, true, false)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("INSERT INTO vertical_settings (tenant_id, kind, receipt_note) VALUES ($1,$2,$3) ON CONFLICT (tenant_id, kind) DO UPDATE SET receipt_note = $3, updated_at = now()").bind(&auth.tenant_id).bind(kind).bind(&note).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "dues.settings_saved",
        None,
        json!({ "kind": kind }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "kind": kind, "receipt_note": note })))
}

/// `GET /v1/dues/{kind}/settings`
pub async fn get_settings(
    State(s): State<AppState>,
    auth: Auth,
    Path(kind): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verticals:read")?;
    let kind = kind_of(&kind)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let note: String =
        sqlx::query_scalar("SELECT receipt_note FROM vertical_settings WHERE kind = $1")
            .bind(kind)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or_default();
    Ok(Json(json!({ "kind": kind, "receipt_note": note })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_cells_are_quoted_and_cannot_run_as_formulas() {
        assert_eq!(csv_cell("plain"), "plain");
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("=1+1"), "'=1+1");
        assert_eq!(csv_cell("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(csv_cell("-5"), "'-5");
        assert_eq!(rupees(123_450), "1234.50");
        assert_eq!(rupees(5), "0.05");
    }

    #[test]
    fn references_refuse_government_id_lengths_and_odd_characters() {
        for ok in ["12", "A-104", "2026/IV-07", "R_5"] {
            assert!(member_ref_ok(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-1", "a b", "123456789012", &"x".repeat(31), "<b>"] {
            assert!(member_ref_ok(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn csv_headers_accept_the_names_schools_and_societies_use() {
        assert_eq!(header_key("Class"), Some("group"));
        assert_eq!(header_key("Roll No."), Some("ref"));
        assert_eq!(header_key("Flat_No"), Some("ref"));
        assert_eq!(header_key("Parent Email"), Some("payer_email"));
        assert_eq!(header_key("Hobby"), None);
    }
}
