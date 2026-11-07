//! Tracing: structured logs always, OpenTelemetry spans when `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
//! Every API request gets a span keyed by its route template (never the raw path, which holds ids)
//! and an `x-request-id` that is echoed back, so a merchant's support ticket can name one request.
//! Authenticated calls are also written to `api_request_log` for the Developers inspector.

use axum::{
    extract::{MatchedPath, Request, State},
    http::HeaderValue,
    middleware::Next,
    response::Response,
};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use tracing::Instrument;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{AppState, reqlog};

/// Flushes buffered spans on drop; keep it alive for the life of the process.
pub struct Guard(Option<SdkTracerProvider>);

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = p.shutdown();
        }
    }
}

pub fn init(service: &'static str) -> Guard {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    // Second-line defence: redact key/PAN/VPA-like fragments that slip into a log line.
    let fmt = tracing_subscriber::fmt::layer()
        .with_writer(crate::redact::RedactingMakeWriter::new(std::io::stderr()));
    let provider = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .ok()
        .filter(|e| !e.is_empty())
        // The exporter's blocking HTTP client must be created outside the async runtime.
        .and_then(|_| {
            match std::thread::spawn(|| SpanExporter::builder().with_http().build())
                .join()
                .expect("exporter thread")
            {
                Ok(exporter) => Some(
                    SdkTracerProvider::builder()
                        .with_batch_exporter(exporter)
                        .with_resource(Resource::builder().with_service_name(service).build())
                        .build(),
                ),
                Err(e) => {
                    eprintln!("OpenTelemetry disabled: {e}");
                    None
                }
            }
        });
    let registry = tracing_subscriber::registry().with(filter).with(fmt);
    match &provider {
        Some(p) => registry
            .with(tracing_opentelemetry::layer().with_tracer(p.tracer(service)))
            .init(),
        None => registry.init(),
    }
    Guard(provider)
}

/// Wrap each request in a span and give it a request id. When Auth runs, also record a row in
/// `api_request_log` (no bodies).
pub async fn request_span(State(s): State<AppState>, mut req: Request, next: Next) -> Response {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_string(), |m| m.as_str().to_string());
    let id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| {
            v.len() <= 64
                && v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        })
        .map_or_else(|| zyvor_core::new_id("req"), str::to_string);
    let method = req.method().to_string();
    let slot = reqlog::Slot::default();
    req.extensions_mut().insert(slot.clone());
    let span = tracing::info_span!("http", method = %method, %route, request_id = %id, status = tracing::field::Empty);
    let started = std::time::Instant::now();
    let mut res = next.run(req).instrument(span.clone()).await;
    let status = res.status().as_u16();
    let ms = started.elapsed().as_millis() as u64;
    span.record("status", status);
    tracing::info!(parent: &span, ms, "request");
    if let Ok(v) = HeaderValue::from_str(&id) {
        res.headers_mut().insert("x-request-id", v);
    }
    // Skip the log endpoint itself and unauthenticated traffic (no Auth → empty slot).
    if route != "/v1/api_requests"
        && let Some(meta) = slot.0.get()
    {
        let pool = s.pool.clone();
        let meta = meta.clone();
        let route = route.clone();
        let id = id.clone();
        tokio::spawn(async move {
            reqlog::record(&pool, &meta, &method, &route, status, &id, ms).await;
        });
    }
    res
}
