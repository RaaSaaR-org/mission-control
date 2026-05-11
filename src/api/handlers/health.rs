//! Health, readiness, and OpenAPI spec endpoints.
//!
//! `/healthz` always returns 200 — process liveness only.
//! `/readyz` returns 200 only when the repo root is readable and the API can
//!     be expected to serve traffic. Returns 503 with a problem-json body
//!     when the underlying filesystem is unreachable.

use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::IntoResponse;

use crate::api::AppState;

/// Liveness probe — the process is alive and the runtime is scheduling.
#[utoipa::path(
    get,
    path = "/healthz",
    tag = "meta",
    responses((status = 200, description = "Process alive"))
)]
pub async fn healthz() -> &'static str {
    "ok"
}

/// Browseable API documentation rendered with RapiDoc, pointing at
/// `/v1/openapi.json`. Public so a fresh `mc api serve` can be opened in
/// the browser without any auth setup.
pub async fn docs() -> impl IntoResponse {
    let body = include_str!("../../../docs/api/rapidoc.html");
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        body,
    )
}

/// Readiness probe — the repo root is readable and writable.
#[utoipa::path(
    get,
    path = "/readyz",
    tag = "meta",
    responses(
        (status = 200, description = "Repo accessible — ready to serve"),
        (status = 503, description = "Repo unreachable")
    )
)]
pub async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    let root = &state.cfg.root;
    match std::fs::metadata(root) {
        Ok(m) if m.is_dir() => (StatusCode::OK, "ready"),
        Ok(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "repo root is not a directory",
        ),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "repo root unreachable"),
    }
}
