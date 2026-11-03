//! An email logo uploaded to Zyvor instead of hosted on the business's own site. The image is kept with the
//! business's settings and served from `GET /logo/{tenant}/{version}` (public, like the logo in the email it is
//! shown in); uploading sets `email_logo_url` to that address. Needs `ZYVOR_PUBLIC_API_URL`. PNG, JPEG, GIF and WebP up
//! to 200 KB, identified by their first bytes (never by what the upload claims); SVG is refused because it can carry
//! script.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use crate::{ApiError, AppState, auth::Auth, tenant};

const MAX_BYTES: usize = 200 * 1024;

/// The image type from the file's first bytes.
pub fn sniff(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// `POST /v1/settings/logo` (admin): the body is the image itself.
pub async fn upload(
    State(s): State<AppState>,
    auth: Auth,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let Some(base) = &s.public_api_url else {
        return Err(ApiError::conflict(
            "logo_upload_unavailable",
            "uploading a logo needs ZYVOR_PUBLIC_API_URL on the server; give an https address in email_logo_url instead",
        ));
    };
    if body.is_empty() || body.len() > MAX_BYTES {
        return Err(ApiError::invalid(
            "invalid_logo",
            "the logo must be an image of up to 200 KB",
        ));
    }
    let Some(kind) = sniff(&body) else {
        return Err(ApiError::invalid(
            "invalid_logo",
            "the logo must be a PNG, JPEG, GIF or WebP image",
        ));
    };
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let version: i32 = sqlx::query_scalar(
        "INSERT INTO tenant_settings (tenant_id, email_logo_data, email_logo_type, email_logo_version) VALUES ($1, $2, $3, 1)
         ON CONFLICT (tenant_id) DO UPDATE SET email_logo_data = $2, email_logo_type = $3,
             email_logo_version = tenant_settings.email_logo_version + 1
         RETURNING email_logo_version",
    )
    .bind(&auth.tenant_id)
    .bind(body.as_ref())
    .bind(kind)
    .fetch_one(&mut *tx)
    .await?;
    let url = format!("{base}/logo/{}/{version}", auth.tenant_id);
    sqlx::query("UPDATE tenant_settings SET email_logo_url = $2 WHERE tenant_id = $1")
        .bind(&auth.tenant_id)
        .bind(&url)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "settings.email_logo_uploaded",
        None,
        json!({ "bytes": body.len(), "type": kind }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "email_logo_url": url })))
}

/// `DELETE /v1/settings/logo` (admin): drop the uploaded image and the address that points at it.
pub async fn remove(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("UPDATE tenant_settings SET email_logo_data = NULL, email_logo_type = NULL, email_logo_url = NULL WHERE tenant_id = $1")
        .bind(&auth.tenant_id)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "settings.email_logo_removed",
        None,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "email_logo_url": null })))
}

/// `GET /logo/{tenant}/{version}`: the image, for mail programs. Only the current version exists.
pub async fn serve(
    State(s): State<AppState>,
    Path((tenant_id, version)): Path<(String, i32)>,
) -> Response {
    let found: Result<Option<(Vec<u8>, String)>, sqlx::Error> = async {
        let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
        sqlx::query_as(
            "SELECT email_logo_data, email_logo_type FROM tenant_settings WHERE tenant_id = $1 AND email_logo_version = $2 AND email_logo_data IS NOT NULL",
        )
        .bind(&tenant_id)
        .bind(version)
        .fetch_optional(&mut *tx)
        .await
    }
    .await;
    match found {
        Ok(Some((data, kind))) => {
            let mut h = HeaderMap::new();
            h.insert(header::CONTENT_TYPE, kind.parse().expect("a known type"));
            h.insert(
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable"
                    .parse()
                    .expect("static"),
            );
            h.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                "nosniff".parse().expect("static"),
            );
            (h, data).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::sniff;

    #[test]
    fn images_are_recognised_by_their_bytes_and_svg_is_not() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0]), Some("image/jpeg"));
        assert_eq!(sniff(b"GIF89a.."), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), None);
        assert_eq!(sniff(b"<?xml version='1.0'?><svg/>"), None);
        assert_eq!(sniff(b""), None);
    }
}
