//! Checklists and comments inside entity files.
//!
//! - `GET  /v1/entities/{kind}/{id}/checklist`        list `- [ ]` items
//! - `POST /v1/entities/{kind}/{id}/checklist/{item}` tick or untick one
//! - `POST /v1/entities/{kind}/{id}/comments`         comment on a task or meeting
//!
//! Writes go through the same `checklist::set_checked` and `comments::add`
//! as the CLI, MCP and dashboard, under the global write lock.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::api::error::ApiError;
use crate::api::handlers::entities::find_of_kind;
use crate::api::schemas::{
    AddCommentBody, CheckItemBody, CheckResult, ChecklistResponse, CommentResult,
};
use crate::api::AppState;
use crate::checklist::{self, Target};
use crate::comments;

#[utoipa::path(
    get,
    path = "/v1/entities/{kind}/{id}/checklist",
    tag = "entities",
    params(
        ("kind" = String, Path, description = "Entity kind"),
        ("id" = String, Path, description = "Entity ID, e.g. TASK-001")
    ),
    responses(
        (status = 200, body = ChecklistResponse),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 404, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn get_checklist(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Json<ChecklistResponse>, ApiError> {
    let rec = find_of_kind(&state, &kind, &id)?;
    let content = std::fs::read_to_string(&rec.source_path)
        .map_err(|e| ApiError::Internal(format!("read entity file: {e}")))?;
    let items = checklist::entity_items(rec.kind, &content);
    let (done, total) = checklist::progress(&items);
    Ok(Json(ChecklistResponse {
        id: rec.id,
        items: items.into_iter().map(Into::into).collect(),
        done,
        total,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/entities/{kind}/{id}/checklist/{item}",
    tag = "entities",
    params(
        ("kind" = String, Path, description = "Entity kind"),
        ("id" = String, Path, description = "Entity ID, e.g. TASK-001"),
        ("item" = usize, Path, description = "1-based item number from GET .../checklist")
    ),
    request_body = CheckItemBody,
    responses(
        (status = 200, body = CheckResult),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson),
        (status = 404, body = crate::api::error::ProblemJson, description = "No such entity or item"),
        (status = 409, body = crate::api::error::ProblemJson, description = "expect_text no longer matches")
    ),
    security(("bearer" = []))
)]
pub async fn check_item(
    State(state): State<AppState>,
    Path((kind, id, item)): Path<(String, String, usize)>,
    Json(body): Json<CheckItemBody>,
) -> Result<Json<CheckResult>, ApiError> {
    let _write_guard = state.write_lock.lock().await;
    let rec = find_of_kind(&state, &kind, &id)?;
    let change = checklist::set_checked(
        &rec,
        Target::Index(item),
        body.checked.unwrap_or(true),
        None,
        body.expect_text.as_deref(),
    )?;
    Ok(Json(CheckResult {
        id: rec.id,
        item: change.item.into(),
        changed: change.changed,
        done: change.done,
        total: change.total,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/entities/{kind}/{id}/comments",
    tag = "entities",
    params(
        ("kind" = String, Path, description = "task or meeting"),
        ("id" = String, Path, description = "Task or meeting ID")
    ),
    request_body = AddCommentBody,
    responses(
        (status = 201, body = CommentResult),
        (status = 400, body = crate::api::error::ProblemJson, description = "Empty text, bad author or a kind without comments"),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson),
        (status = 404, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn add_comment(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, String)>,
    Json(body): Json<AddCommentBody>,
) -> Result<(StatusCode, Json<CommentResult>), ApiError> {
    let _write_guard = state.write_lock.lock().await;
    let rec = find_of_kind(&state, &kind, &id)?;
    let added = comments::add(&state.cfg, &rec, &body.text, body.author.as_deref())?;
    Ok((StatusCode::CREATED, Json(added.into())))
}
