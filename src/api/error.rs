//! RFC 7807 problem-json error responses for the REST API.
//!
//! Maps `McError` (and a small set of API-only errors like auth failures)
//! onto `application/problem+json` with stable `type` URIs so callers can
//! switch on machine-readable error categories instead of parsing prose.
//!
//! [`problem_responses`] turns everything else that ends in an error (axum's
//! own rejections of a bad JSON body or query string, unknown routes, wrong
//! methods, oversized bodies, timeouts) into problem+json too, and keeps
//! server filesystem paths out of `detail`.

use crate::api::AppState;
use crate::error::McError;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use std::path::Path;
use utoipa::ToSchema;

/// RFC 7807 problem details document.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProblemJson {
    /// Stable URI identifying the error category.
    #[serde(rename = "type")]
    pub kind: String,
    /// Short, human-readable summary.
    pub title: String,
    /// HTTP status code.
    pub status: u16,
    /// Free-form detail explaining this specific occurrence.
    pub detail: String,
    /// Optional structured field-level errors (used for validation failures).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub errors: Vec<FieldError>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FieldError {
    pub field: String,
    pub code: String,
}

impl ProblemJson {
    pub fn new(kind: &str, title: &str, status: StatusCode, detail: impl Into<String>) -> Self {
        Self {
            kind: format!("https://docs.mc.dev/errors/{kind}"),
            title: title.into(),
            status: status.as_u16(),
            detail: detail.into(),
            errors: Vec::new(),
        }
    }
}

/// API-side errors that do not originate from `McError` (auth, parse failures).
#[derive(Debug)]
pub enum ApiError {
    /// Missing or malformed Authorization header.
    Unauthenticated(&'static str),
    /// Too many requests are already waiting (e.g. new bearer verifications).
    Unavailable(&'static str),
    /// Authenticated but lacks the required capability (e.g. write).
    Forbidden(&'static str),
    /// Request body could not be decoded.
    BadRequest(String),
    /// Unexpected internal error (kept opaque to clients).
    Internal(String),
    /// Wrap an `McError` so it goes through the same axum response path.
    Domain(McError),
}

impl From<McError> for ApiError {
    fn from(e: McError) -> Self {
        ApiError::Domain(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let problem = match self {
            ApiError::Unauthenticated(detail) => ProblemJson::new(
                "unauthenticated",
                "Authentication required",
                StatusCode::UNAUTHORIZED,
                detail,
            ),
            ApiError::Unavailable(detail) => ProblemJson::new(
                "unavailable",
                "Service unavailable",
                StatusCode::SERVICE_UNAVAILABLE,
                detail,
            ),
            ApiError::Forbidden(detail) => {
                ProblemJson::new("forbidden", "Forbidden", StatusCode::FORBIDDEN, detail)
            }
            ApiError::BadRequest(detail) => ProblemJson::new(
                "bad-request",
                "Bad request",
                StatusCode::BAD_REQUEST,
                detail,
            ),
            ApiError::Internal(detail) => {
                tracing::error!(error = %detail, "internal API error");
                ProblemJson::new(
                    "internal",
                    "Internal server error",
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "An unexpected error occurred. See server logs.",
                )
            }
            ApiError::Domain(e) => problem_from_mc_error(&e),
        };

        problem.into_response()
    }
}

impl IntoResponse for ProblemJson {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, Json(self.clone())).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        // Lets `problem_responses` recognise (and scrub) its own errors.
        response.extensions_mut().insert(self);
        response
    }
}

/// Middleware: every error response is problem+json, and its `detail` never
/// shows where the repo lives on the server.
pub async fn problem_responses(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let response = next.run(req).await;
    let status = response.status();
    if !status.is_client_error() && !status.is_server_error() {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let problem = match parts.extensions.remove::<ProblemJson>() {
        Some(problem) => {
            let detail = scrub_paths(&problem.detail, &state.cfg.root);
            if detail == problem.detail {
                return Response::from_parts(parts, body);
            }
            ProblemJson { detail, ..problem }
        }
        None => {
            // An axum rejection or fallback: plain text or an empty body.
            let text = axum::body::to_bytes(body, 16 * 1024)
                .await
                .map(|b| String::from_utf8_lossy(&b).trim().to_string())
                .unwrap_or_default();
            rejection_problem(status, &method, &path, &scrub_paths(&text, &state.cfg.root))
        }
    };
    // Keep headers such as `Allow` on a 405; the body and its type change.
    let mut response = problem.into_response();
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.remove(header::CONTENT_TYPE);
    for (name, value) in parts.headers.iter() {
        response.headers_mut().insert(name.clone(), value.clone());
    }
    response
}

/// Problem document for an error axum produced without `ApiError`.
fn rejection_problem(
    status: StatusCode,
    method: &axum::http::Method,
    path: &str,
    text: &str,
) -> ProblemJson {
    let detail = |fallback: String| {
        if text.is_empty() {
            fallback
        } else {
            text.to_string()
        }
    };
    match status {
        // A JSON body that doesn't parse or doesn't fit the schema (axum says
        // 422 for the latter), or a query string that doesn't.
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => ProblemJson::new(
            "bad-request",
            "Bad request",
            StatusCode::BAD_REQUEST,
            detail("The request could not be read.".into()),
        ),
        StatusCode::NOT_FOUND => ProblemJson::new(
            "not-found",
            "Not found",
            status,
            detail(format!("No endpoint at {path}. See /v1/openapi.json.")),
        ),
        StatusCode::METHOD_NOT_ALLOWED => ProblemJson::new(
            "method-not-allowed",
            "Method not allowed",
            status,
            detail(format!("{method} is not supported on {path}.")),
        ),
        StatusCode::PAYLOAD_TOO_LARGE => ProblemJson::new(
            "payload-too-large",
            "Payload too large",
            status,
            detail("The request body is over the 64 KiB limit.".into()),
        ),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ProblemJson::new(
            "unsupported-media-type",
            "Unsupported media type",
            status,
            detail("Send the body as JSON with Content-Type: application/json.".into()),
        ),
        StatusCode::REQUEST_TIMEOUT => ProblemJson::new(
            "timeout",
            "Request timeout",
            status,
            detail("The request took too long.".into()),
        ),
        StatusCode::SERVICE_UNAVAILABLE => ProblemJson::new(
            "unavailable",
            "Service unavailable",
            status,
            detail("The service is not ready.".into()),
        ),
        _ => ProblemJson::new(
            "http",
            status.canonical_reason().unwrap_or("Error"),
            status,
            detail(status.to_string()),
        ),
    }
}

/// Remove the repo root from paths in an error message, so details name
/// files relative to the repo (`tasks/todo/TASK-001-x.md`).
fn scrub_paths(text: &str, root: &Path) -> String {
    let mut roots: Vec<String> = [Some(root.to_path_buf()), root.canonicalize().ok()]
        .into_iter()
        .flatten()
        .map(|r| format!("{}/", r.display()))
        .filter(|r| r.len() > 2)
        .collect();
    // Longest first: /private/tmp/x/ before /tmp/x/.
    roots.sort_by_key(|r| std::cmp::Reverse(r.len()));
    roots
        .iter()
        .fold(text.to_string(), |out, r| out.replace(r.as_str(), ""))
}

/// Map an `McError` to a `ProblemJson`. Status codes:
/// - 400: invalid input (`Usage`: bad status/priority/date, empty name),
///   invalid id format, frontmatter parse
/// - 403: kind not enabled in this repo (embedded mode or missing from `paths:`)
/// - 404: entity or other requested thing not found (`EntityNotFound`, `NotFound`)
/// - 409: already initialized, or the file changed under a checklist edit
/// - 422: validation failed (set of issues)
/// - 500: io / yaml / json / zip / pdf / other
fn problem_from_mc_error(e: &McError) -> ProblemJson {
    match e {
        McError::InvalidId(_) => ProblemJson::new(
            "invalid-id",
            "Invalid entity ID",
            StatusCode::BAD_REQUEST,
            e.to_string(),
        ),
        McError::EntityNotFound(_) => ProblemJson::new(
            "entity-not-found",
            "Entity not found",
            StatusCode::NOT_FOUND,
            e.to_string(),
        ),
        McError::NotFound { .. } => ProblemJson::new(
            "not-found",
            "Not found",
            StatusCode::NOT_FOUND,
            e.to_string(),
        ),
        McError::Usage { .. } => ProblemJson::new(
            "bad-request",
            "Bad request",
            StatusCode::BAD_REQUEST,
            e.to_string(),
        ),
        McError::Frontmatter { .. } => ProblemJson::new(
            "frontmatter",
            "Invalid frontmatter",
            StatusCode::BAD_REQUEST,
            e.to_string(),
        ),
        McError::ValidationFailed(_) => ProblemJson::new(
            "validation",
            "Validation failed",
            StatusCode::UNPROCESSABLE_ENTITY,
            e.to_string(),
        ),
        McError::NotAvailableInMode { .. } => ProblemJson::new(
            "not-available",
            "Entity kind not enabled in this repo",
            StatusCode::FORBIDDEN,
            e.to_string(),
        ),
        McError::RepoRootNotFound | McError::ConfigNotFound(_) => ProblemJson::new(
            "repo-not-found",
            "Repository not configured",
            StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        ),
        McError::TemplateNotFound(_) => ProblemJson::new(
            "template-not-found",
            "Template missing",
            StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        ),
        McError::Conflict { .. } => ProblemJson::new(
            "conflict",
            "Conflict",
            StatusCode::CONFLICT,
            match e.hint() {
                Some(hint) => format!("{e} {hint}"),
                None => e.to_string(),
            },
        ),
        McError::AlreadyInitialized(_) => ProblemJson::new(
            "already-initialized",
            "Already initialized",
            StatusCode::CONFLICT,
            e.to_string(),
        ),
        // Fallback for user errors still signalled via McError::Other; the core
        // validation paths use the typed `Usage`/`NotFound` variants above.
        McError::Other(msg) if is_user_facing_other(msg) => {
            ProblemJson::new("bad-request", "Bad request", StatusCode::BAD_REQUEST, msg)
        }
        _ => {
            tracing::error!(error = %e, "internal mc error");
            ProblemJson::new(
                "internal",
                "Internal server error",
                StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
            )
        }
    }
}

fn is_user_facing_other(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.starts_with("invalid ")
        || lower.contains("cannot be empty")
        || lower.contains("must be ")
        || lower.contains("does not exist")
        || lower.contains("not found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityKind;

    fn status_of(e: McError) -> u16 {
        problem_from_mc_error(&e).status
    }

    #[test]
    fn typed_errors_map_to_client_statuses() {
        assert_eq!(status_of(McError::usage("Invalid priority 9", None)), 400);
        assert_eq!(status_of(McError::not_found("no such file", None)), 404);
        assert_eq!(status_of(McError::EntityNotFound("TASK-9".into())), 404);
        let not_enabled = McError::NotAvailableInMode {
            kind: EntityKind::Customer,
            embedded: false,
        };
        assert_eq!(status_of(not_enabled), 403);
        assert_eq!(status_of(McError::conflict("item changed", None)), 409);
        assert_eq!(status_of(McError::Io(std::io::Error::other("disk"))), 500);
        let p = problem_from_mc_error(&McError::not_found("gone", None));
        assert!(p.kind.ends_with("/not-found"));
        assert_eq!(p.detail, "gone");
    }
}
