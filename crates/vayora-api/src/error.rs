use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    pub fn bad_request(code: &'static str, m: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, m)
    }
    pub fn invalid(code: &'static str, m: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, code, m)
    }
    pub fn not_found(what: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{what} not found"),
        )
    }
    pub fn conflict(code: &'static str, m: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, m)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": { "code": self.code, "message": self.message } })),
        )
            .into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        // Never leak SQL details to clients.
        tracing::error!(error = %e, "database error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error",
        )
    }
}

impl From<vayora_ledger::StoreError> for ApiError {
    fn from(e: vayora_ledger::StoreError) -> Self {
        tracing::error!(error = %e, "ledger error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error",
        )
    }
}

impl From<vayora_upi::UpiError> for ApiError {
    fn from(e: vayora_upi::UpiError) -> Self {
        tracing::error!(error = %e, "upi error");
        Self::new(
            StatusCode::BAD_GATEWAY,
            "upi_unavailable",
            "UPI network unavailable, safe to retry with the same Idempotency-Key",
        )
    }
}
