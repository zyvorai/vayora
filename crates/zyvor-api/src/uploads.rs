//! Onboarding documents uploaded as files: stored encrypted, read for the facts on them (`docread`), and compared
//! with what the merchant entered (`docfacts`). The comparison is run each time it is shown, from the stored
//! facts, so it follows the profile if the profile changes. Anything a document gets wrong, or fails to show, goes
//! to a person when the business submits; nothing here approves anybody.
//!
//! Aadhaar cards and letters are refused, not stored: the number may not be kept without being licensed for it,
//! and Zyvor has no use for it.

use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use zyvor_core::new_id;

use crate::{
    ApiError, AppState,
    auth::Auth,
    docfacts::{self, Check, Entered, Facts},
    docread::FileKind,
    onboarding, tenant,
};

/// The biggest file accepted, and the most files a business may keep.
pub const MAX_BYTES: usize = 5 * 1024 * 1024;
const MAX_FILES: i64 = 12;

#[derive(Deserialize)]
pub struct UploadIn {
    kind: String,
    filename: String,
    /// The file, standard base64.
    data_base64: String,
}

/// A name that is safe to show and to send back in a header: no path, no control characters, not long.
fn safe_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | ';' | '<' | '>'))
        .take(120)
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        "document".to_string()
    } else {
        cleaned
    }
}

struct Profile {
    legal_name: String,
    pan_last4: String,
    gstin: Option<String>,
}

async fn profile(conn: &mut PgConnection) -> Result<Option<Profile>, ApiError> {
    let row: Option<(String, String, Option<String>)> =
        sqlx::query_as("SELECT legal_name, pan_last4, gstin FROM merchant_profiles")
            .fetch_optional(&mut *conn)
            .await?;
    Ok(row.map(|(legal_name, pan_last4, gstin)| Profile {
        legal_name,
        pan_last4,
        gstin,
    }))
}

async fn checks_for(
    conn: &mut PgConnection,
    kind: &str,
    facts: &Facts,
    p: &Profile,
) -> Result<Vec<Check>, ApiError> {
    let ifscs: Vec<String> = sqlx::query_scalar("SELECT ifsc FROM merchant_bank_accounts")
        .fetch_all(&mut *conn)
        .await?;
    Ok(docfacts::cross_check(
        kind,
        facts,
        &Entered {
            legal_name: &p.legal_name,
            pan_last4: &p.pan_last4,
            gstin: p.gstin.as_deref(),
            bank_ifscs: &ifscs,
        },
    ))
}

type Row = (
    String,
    String,
    String,
    String,
    i32,
    String,
    Value,
    String,
    chrono::DateTime<chrono::Utc>,
);

const COLS: &str =
    "id, kind, filename, content_type, size_bytes, read_method, facts, uploaded_by, created_at";

async fn shown(conn: &mut PgConnection, row: Row, p: Option<&Profile>) -> Result<Value, ApiError> {
    let facts: Facts = serde_json::from_value(row.6.clone()).unwrap_or_default();
    let checks = match p {
        Some(p) => checks_for(conn, &row.1, &facts, p).await?,
        None => Vec::new(),
    };
    Ok(json!({
        "id": row.0, "kind": row.1, "filename": row.2, "content_type": row.3, "size_bytes": row.4,
        "read_method": row.5, "facts": row.6, "checks": checks, "uploaded_by": row.7, "created_at": row.8,
    }))
}

/// Every uploaded document with the comparison against the profile as it stands now.
pub async fn summaries(conn: &mut PgConnection) -> Result<Vec<Value>, ApiError> {
    let p = profile(conn).await?;
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM onboarding_uploads ORDER BY created_at"
    )))
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for r in rows {
        out.push(shown(conn, r, p.as_ref()).await?);
    }
    Ok(out)
}

/// Reasons a person must look at this business's documents before it can be approved: a document that could not be
/// read, or one that disagrees with the profile or does not show what it should.
pub async fn review_reasons(conn: &mut PgConnection) -> Result<Vec<String>, ApiError> {
    let mut reasons = Vec::new();
    for u in summaries(conn).await? {
        let name = u["filename"].as_str().unwrap_or("a document");
        if u["read_method"] == "none" {
            reasons.push(format!(
                "{name}: could not be read automatically, so nothing on it was checked"
            ));
        }
        for c in u["checks"].as_array().into_iter().flatten() {
            if matches!(c["result"].as_str(), Some("mismatch" | "not_found")) {
                reasons.push(format!(
                    "{name}: {} ({})",
                    c["name"].as_str().unwrap_or("check"),
                    c["detail"].as_str().unwrap_or("")
                ));
            }
        }
    }
    Ok(reasons)
}

/// `POST /v1/onboarding/uploads`
pub async fn upload(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<UploadIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("onboarding:write")?;
    if !onboarding::DOC_KINDS.contains(&b.kind.as_str()) {
        return Err(ApiError::invalid(
            "invalid_document",
            format!("kind is one of {:?}", onboarding::DOC_KINDS),
        ));
    }
    if b.data_base64.len() > MAX_BYTES * 4 / 3 + 16 {
        return Err(ApiError::invalid(
            "file_too_large",
            "files are limited to 5 MB",
        ));
    }
    let bytes = STANDARD
        .decode(b.data_base64.trim())
        .map_err(|_| ApiError::invalid("invalid_file", "data_base64 is not valid base64"))?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(ApiError::invalid(
            "file_too_large",
            "files are limited to 5 MB",
        ));
    }
    let Some(kind) = FileKind::sniff(&bytes) else {
        return Err(ApiError::invalid(
            "unsupported_file",
            "send a PDF, PNG or JPEG file",
        ));
    };
    let sha = hex::encode(Sha256::digest(&bytes));

    // room, the profile to compare with, and not the same file twice: decided before the (slow) reading, which
    // holds no database transaction
    {
        let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
        if profile(&mut tx).await?.is_none() {
            return Err(ApiError::conflict(
                "no_profile",
                "save the profile first, so the document can be compared with it",
            ));
        }
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM onboarding_uploads")
            .fetch_one(&mut *tx)
            .await?;
        if n >= MAX_FILES {
            return Err(ApiError::conflict(
                "too_many_files",
                format!("at most {MAX_FILES} files"),
            ));
        }
        let dup: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM onboarding_uploads WHERE sha256 = $1)",
        )
        .bind(&sha)
        .fetch_one(&mut *tx)
        .await?;
        if dup {
            return Err(ApiError::conflict(
                "already_uploaded",
                "this file is already uploaded",
            ));
        }
    }

    let reading = s.doc_reader.read(&bytes, kind).await;
    if docfacts::looks_like_aadhaar(&reading.text) {
        return Err(ApiError::invalid(
            "aadhaar_not_accepted",
            "Aadhaar cards and letters cannot be uploaded. Use the PAN card, or the certificates of the business",
        ));
    }
    let facts = docfacts::extract(&reading.text);

    let id = new_id("oup");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let p = profile(&mut tx)
        .await?
        .ok_or_else(|| ApiError::conflict("no_profile", "save the profile first"))?;
    sqlx::query("INSERT INTO onboarding_uploads (id, tenant_id, kind, filename, content_type, size_bytes, sha256, data_enc, read_method, facts, uploaded_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(&id)
        .bind(&auth.tenant_id)
        .bind(&b.kind)
        .bind(safe_name(&b.filename))
        .bind(kind.mime())
        .bind(bytes.len() as i32)
        .bind(&sha)
        .bind(s.crypto.encrypt(&bytes))
        .bind(reading.method)
        .bind(docfacts::facts_json(&facts))
        .bind(&auth.actor)
        .execute(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => {
                ApiError::conflict("already_uploaded", "this file is already uploaded")
            }
            e => e.into(),
        })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "onboarding.upload_added",
        Some(&id),
        json!({ "kind": b.kind, "read_method": reading.method, "size_bytes": bytes.len() }),
    )
    .await?;
    let row: Row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM onboarding_uploads WHERE id = $1"
    )))
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    let out = shown(&mut tx, row, Some(&p)).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// `GET /v1/onboarding/uploads`
pub async fn list(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("onboarding:write")
        .or_else(|_| auth.require("onboarding:review"))?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let data = summaries(&mut tx).await?;
    Ok(Json(
        json!({ "data": data, "reader_available": s.doc_reader.available() }),
    ))
}

/// `GET /v1/onboarding/uploads/{id}/file`: the stored file, as a download.
pub async fn file(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    auth.require("onboarding:write")
        .or_else(|_| auth.require("onboarding:review"))?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, String, Vec<u8>)> = sqlx::query_as(
        "SELECT filename, content_type, data_enc FROM onboarding_uploads WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (name, mime, enc) = row.ok_or_else(|| ApiError::not_found("document"))?;
    let bytes = s.crypto.decrypt(&enc).ok_or_else(ApiError::internal)?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "onboarding.upload_downloaded",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    download_response(&name, &mime, bytes)
}

/// A stored file as a download: never shown inline, never sniffed, never cached.
pub fn download_response(name: &str, mime: &str, bytes: Vec<u8>) -> Result<Response, ApiError> {
    let mut res = Response::new(Body::from(bytes));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime).map_err(|_| ApiError::internal())?,
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

/// `DELETE /v1/onboarding/uploads/{id}`: only while the profile is still a draft or was sent back.
pub async fn delete(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("onboarding:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let st: Option<String> = sqlx::query_scalar("SELECT status FROM merchant_profiles")
        .fetch_optional(&mut *tx)
        .await?;
    if matches!(st.as_deref(), Some("in_review" | "approved")) {
        return Err(ApiError::conflict(
            "not_editable",
            "documents of a submitted or approved profile are kept as filed",
        ));
    }
    let n = sqlx::query("DELETE FROM onboarding_uploads WHERE id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("document"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "onboarding.upload_deleted",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "deleted": true })))
}

#[cfg(test)]
mod tests {
    use super::safe_name;

    #[test]
    fn a_filename_loses_its_path_and_anything_that_could_break_a_header() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(
            safe_name("C:\\Users\\me\\GST certificate.pdf"),
            "GST certificate.pdf"
        );
        assert_eq!(
            safe_name("a\"b;c\r\nSet-Cookie: x.pdf"),
            "abcSet-Cookie: x.pdf"
        );
        assert_eq!(safe_name("   "), "document");
        assert_eq!(safe_name(&"x".repeat(400)).len(), 120);
    }
}
