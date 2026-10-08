//! `/v1/index` and `/v1/validate`.
//!
//! `index` is a write (rebuilds JSON files under `data/`) and takes the write
//! lock. `validate` is read-only and just inspects the tree, so it is served
//! for GET and (for older clients) POST, and read-only tokens may call both.

use axum::extract::State;
use axum::Json;

use crate::api::error::ApiError;
use crate::api::schemas::{
    IndexResult as IndexResp, ValidationIssue as ValidationIssueResp, ValidationReport,
};
use crate::api::AppState;
use crate::commands::{index, validate};

#[utoipa::path(
    post, path = "/v1/index", tag = "maintenance",
    responses(
        (status = 200, body = IndexResp),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn rebuild_index(State(state): State<AppState>) -> Result<Json<IndexResp>, ApiError> {
    let _w = state.write_lock.lock().await;
    let r = index::run_quiet(&state.cfg)?;
    Ok(Json(IndexResp {
        customers: r.customers,
        projects: r.projects,
        meetings: r.meetings,
        research: r.research,
        tasks: r.tasks,
        sprints: r.sprints,
        milestones: r.milestones,
        proposals: r.proposals,
        contacts: r.contacts,
    }))
}

#[utoipa::path(
    method(get, post), path = "/v1/validate", tag = "maintenance",
    responses(
        (status = 200, body = ValidationReport, description = "ok=false when errors are present (warnings alone keep ok=true)"),
        (status = 401, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn run_validate(
    State(state): State<AppState>,
) -> Result<Json<ValidationReport>, ApiError> {
    let issues = validate::validate_programmatic(&state.cfg)?;
    let ok = !issues.iter().any(|i| i.is_error());
    Ok(Json(ValidationReport {
        ok,
        issues: issues
            .into_iter()
            .map(|i| ValidationIssueResp {
                severity: if i.is_error() { "error" } else { "warning" }.into(),
                path: i.path,
                check: i.check,
                message: i.message,
            })
            .collect(),
    }))
}
