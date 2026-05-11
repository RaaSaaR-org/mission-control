//! Task-specific endpoints: filtered list and status move.
//!
//! `/v1/tasks` exists alongside `/v1/entities/task` because tasks have a
//! richer filter set (project, customer, priority, sprint, owner) than the
//! generic entity list.

use axum::extract::{Path, Query, State};
use axum::Json;

use crate::api::error::ApiError;
use crate::api::schemas::{MoveTaskBody, MoveTaskResult, TaskListQuery};
use crate::api::AppState;
use crate::commands::task;
use crate::data::{self, TaskFilter};

#[utoipa::path(
    get,
    path = "/v1/tasks",
    tag = "tasks",
    params(TaskListQuery),
    responses(
        (status = 200, description = "Array of tasks matching the filter set"),
        (status = 401, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn list_tasks(
    State(state): State<AppState>,
    Query(q): Query<TaskListQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let filter = TaskFilter {
        status: q.status.as_deref(),
        tag: q.tag.as_deref(),
        project: q.project.as_deref(),
        customer: q.customer.as_deref(),
        priority: q.priority,
        sprint: q.sprint.as_deref(),
        owner: q.owner.as_deref(),
    };

    let tasks = data::collect_tasks_filtered(&state.cfg, &filter)?;
    let json: Vec<serde_json::Value> = tasks
        .iter()
        .map(|t| {
            let mut v = data::yaml_to_json(&t.frontmatter);
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "_source".into(),
                    serde_json::Value::String(t.source_path.display().to_string()),
                );
            }
            v
        })
        .collect();

    Ok(Json(serde_json::Value::Array(json)))
}

#[utoipa::path(
    post,
    path = "/v1/tasks/{id}/move",
    tag = "tasks",
    params(("id" = String, Path, description = "Task ID, e.g. TASK-001")),
    request_body = MoveTaskBody,
    responses(
        (status = 200, body = MoveTaskResult),
        (status = 400, body = crate::api::error::ProblemJson, description = "Invalid status"),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson, description = "Read-only or missing write capability"),
        (status = 404, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn move_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<MoveTaskBody>,
) -> Result<Json<MoveTaskResult>, ApiError> {
    let _write_guard = state.write_lock.lock().await;
    let result =
        task::move_task_programmatic(&state.cfg, &id, &body.status, body.sprint.as_deref())?;

    // move_task_programmatic returns serde_json::Value with the same shape
    // as MoveTaskResult — re-deserialize to get a typed response.
    let typed: MoveTaskResult = serde_json::from_value(result)
        .map_err(|e| ApiError::Internal(format!("move_task result decode: {e}")))?;
    Ok(Json(typed))
}
