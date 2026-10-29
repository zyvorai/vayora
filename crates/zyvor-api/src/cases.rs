//! Reconciliation cases: every unexplained difference becomes a task someone owns.

use serde_json::Value;
use sqlx::PgConnection;

#[allow(clippy::too_many_arguments)]
pub async fn open_case(
    conn: &mut PgConnection,
    tenant_id: &str,
    kind: &str,
    subject_type: &str,
    subject_id: &str,
    expected_minor: Option<i64>,
    actual_minor: Option<i64>,
    explanation: &str,
    detail: Value,
) -> Result<bool, sqlx::Error> {
    let diff = expected_minor.zip(actual_minor).map(|(e, a)| a - e);
    let n = sqlx::query(
        "INSERT INTO recon_cases (id, tenant_id, kind, subject_type, subject_id, expected_minor, actual_minor, diff_minor, explanation, detail)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (tenant_id, kind, subject_id) DO NOTHING",
    )
    .bind(zyvor_core::new_id("rc"))
    .bind(tenant_id)
    .bind(kind)
    .bind(subject_type)
    .bind(subject_id)
    .bind(expected_minor)
    .bind(actual_minor)
    .bind(diff)
    .bind(explanation)
    .bind(detail)
    .execute(conn)
    .await?
    .rows_affected();
    Ok(n == 1)
}
