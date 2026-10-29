use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn post(app: axum::Router, key: Option<&str>, body: &str) -> (StatusCode, String) {
    let mut req = Request::post("/v1/orders").header("content-type", "application/json");
    if let Some(k) = key {
        req = req.header("idempotency-key", k);
    }
    let res = app
        .oneshot(req.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn same_key_returns_same_order() {
    let app = vayora_api::app();
    let (s1, b1) = post(app.clone(), Some("k1"), r#"{"amount_paise":5000}"#).await;
    let (s2, b2) = post(app, Some("k1"), r#"{"amount_paise":5000}"#).await;
    assert_eq!(s1, StatusCode::CREATED);
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(b1, b2);
}

#[tokio::test]
async fn missing_key_or_bad_amount_rejected() {
    let app = vayora_api::app();
    assert_eq!(
        post(app.clone(), None, r#"{"amount_paise":5000}"#).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(app, Some("k"), r#"{"amount_paise":0}"#).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
