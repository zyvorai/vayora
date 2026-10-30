//! Tracing: structured logs always, OpenTelemetry spans when `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
//! Every API request gets a span keyed by its route template (never the raw path, which holds ids)
//! and an `x-request-id` that is echoed back, so a merchant's support ticket can name one request.

use axum::{
    extract::{MatchedPath, Request},
    http::HeaderValue,
    middleware::Next,
    response::Response,
};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use tracing::Instrument;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

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
    let fmt = tracing_subscriber::fmt::layer();
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

/// Wrap each request in a span and give it a request id.
pub async fn request_span(req: Request, next: Next) -> Response {
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
    let span = tracing::info_span!("http", method = %req.method(), %route, request_id = %id, status = tracing::field::Empty);
    let started = std::time::Instant::now();
    let mut res = next.run(req).instrument(span.clone()).await;
    span.record("status", res.status().as_u16());
    tracing::info!(parent: &span, ms = started.elapsed().as_millis() as u64, "request");
    if let Ok(v) = HeaderValue::from_str(&id) {
        res.headers_mut().insert("x-request-id", v);
    }
    res
}
