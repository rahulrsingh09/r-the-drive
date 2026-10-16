use std::sync::{Arc, Mutex};

use axum::{body::Body, middleware, routing::get, Router};
use http::{Request, StatusCode};
use tower::ServiceExt;
use tower_http::trace::TraceLayer;
use tracing::instrument::WithSubscriber;

#[derive(Clone)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn request_logs_exclude_query_path_and_header_capabilities() {
    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NEW)
        .with_writer(move || LogBuffer(writer.clone()))
        .finish();
    async {
        let app = Router::new()
            .route(
                "/v1/invites/{token}",
                get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
            )
            .layer(middleware::from_fn_with_state(
                jsonwebtoken::DecodingKey::from_secret(b"synthetic"),
                super::log_server_errors,
            ))
            .layer(TraceLayer::new_for_http().make_span_with(super::request_span));
        for uri in [
            "/v1/invites/secret-invite?code=secret-oauth&state=secret-state",
            "/unmatched/secret-unknown-path?token=secret-recovery",
        ] {
            let request = Request::builder()
                .uri(uri)
                .header("authorization", "Bearer secret-bearer")
                .header("cookie", "refresh_token=secret-refresh")
                .header("referer", "https://example.invalid/?token=secret-referrer")
                .header("sec-websocket-protocol", "bearer,secret-websocket")
                .body(Body::empty())
                .unwrap();
            let _ = app.clone().oneshot(request).await.unwrap();
        }
    }
    .with_subscriber(subscriber)
    .await;
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(text.contains("/v1/invites/{token}"), "{text}");
    assert!(text.contains("<unmatched>"), "{text}");
    assert!(!text.contains("secret-"), "{text}");
}
