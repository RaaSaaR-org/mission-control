//! RFC 7807 problem-json error responses for the REST API.
//!
//! Maps `McError` (and a small set of API-only errors like auth failures)
//! onto `application/problem+json` with stable `type` URIs so callers can
//! switch on machine-readable error categories instead of parsing prose.

use crate::error::McError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
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

    #[allow(dead_code)]
    pub fn with_field_errors(mut self, errors: Vec<FieldError>) -> Self {
        self.errors = errors;
        self
    }
}

/// API-side errors that do not originate from `McError` (auth, parse failures).
#[derive(Debug)]
pub enum ApiError {
    /// Missing or malformed Authorization header.
    Unauthenticated(&'static str),
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

        let status =
            StatusCode::from_u16(problem.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, Json(problem)).into_response();
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

/// Map an `McError` to a `ProblemJson`. Status codes:
/// - 400: invalid id format, status not in config, name empty, frontmatter parse
/// - 403: kind not available in current repo mode
/// - 404: entity not found, repo root not found, template missing
/// - 409: already initialized
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
            "Entity kind not available in this repo mode",
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
        McError::AlreadyInitialized(_) => ProblemJson::new(
            "already-initialized",
            "Already initialized",
            StatusCode::CONFLICT,
            e.to_string(),
        ),
        // Some create_*_programmatic helpers signal user errors via McError::Other
        // (e.g. "name cannot be empty", "Invalid task status 'frob'"). Without a
        // dedicated variant we can't tell those apart from internal errors, so we
        // map by message prefix — short and pragmatic, and a future PR can split
        // out a typed `BadRequest` variant.
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
