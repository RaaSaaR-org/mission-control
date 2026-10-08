//! Task-specific endpoints: filtered list, next actionable tasks, field
//! updates and status move.
//!
//! `/v1/tasks` exists alongside `/v1/entities/task` because tasks have a
//! richer filter set (project, customer, priority, sprint, owner) than the
//! generic entity list.

use axum::extract::{Path, Query, State};
use axum::Json;

use crate::api::error::ApiError;
use crate::api::schemas::{
    MoveTaskBody, MoveTaskResult, NextTasksQuery, NextTasksResult, TaskListQuery, UpdateTask,
    UpdateTaskResult,
};
use crate::api::AppState;
use crate::cli::suggest;
use crate::commands::task;
use crate::data::{self, TaskFilter};
use crate::entity::EntityKind;
use crate::error::McError;
use crate::mcp::{
    entity_json, loose_ref, next_tasks_json, relativize_field, resolve_entity, status_filter,
};
use crate::util::parse_comma_list;

#[utoipa::path(
    get,
    path = "/v1/tasks",
    tag = "tasks",
    params(TaskListQuery),
    responses(
        (status = 200, description = "Array of tasks matching the filter set, shaped like GET /v1/entities/task"),
        (status = 401, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn list_tasks(
    State(state): State<AppState>,
    Query(q): Query<TaskListQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let cfg = &state.cfg;
    let status = q
        .status
        .as_deref()
        .map(|s| status_filter(cfg, EntityKind::Task, s));
    let id_ref = |v: &Option<String>, kind| v.as_deref().map(|v| loose_ref(cfg, v, kind));
    let project = id_ref(&q.project, EntityKind::Project);
    let customer = id_ref(&q.customer, EntityKind::Customer);
    let sprint = id_ref(&q.sprint, EntityKind::Sprint);
    let milestone = id_ref(&q.milestone, EntityKind::Milestone);
    let filter = TaskFilter {
        status: status.as_deref(),
        tag: q.tag.as_deref(),
        project: project.as_deref(),
        customer: customer.as_deref(),
        priority: q.priority,
        sprint: sprint.as_deref(),
        owner: q.owner.as_deref(),

        milestone: milestone.as_deref(),
    };

    let tasks = data::collect_tasks_filtered(cfg, &filter)?;
    let json: Vec<serde_json::Value> = tasks.iter().map(|t| entity_json(t, cfg)).collect();

    Ok(Json(serde_json::Value::Array(json)))
}

#[utoipa::path(
    post,
    path = "/v1/tasks/{id}/move",
    tag = "tasks",
    params(("id" = String, Path, description = "Task ID, e.g. TASK-001 (task-1 works too)")),
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
    let cfg = &state.cfg;
    let _write_guard = state.write_lock.lock().await;
    let rec = resolve_entity(cfg, &id, None)?;
    if rec.kind != EntityKind::Task {
        return Err(ApiError::Domain(McError::usage(
            format!("{} is a {}, not a task", rec.id, rec.kind.label()),
            None,
        )));
    }
    // Forgiving like `mc task move`: case, `_` and aliases (`doing`).
    let status = suggest::resolve_status(&body.status, &cfg.statuses.task, EntityKind::Task)
        .map_err(|e| match e.hint() {
            Some(hint) => ApiError::BadRequest(format!("{e} ({hint})")),
            None => ApiError::Domain(e),
        })?;
    let sprint = body
        .sprint
        .as_deref()
        .map(|s| loose_ref(cfg, s, EntityKind::Sprint));
    let mut result = task::move_task_programmatic(cfg, &rec.id, &status, sprint.as_deref())?;
    relativize_field(cfg, &mut result, "path");

    // move_task_programmatic returns serde_json::Value with the same shape
    // as MoveTaskResult — re-deserialize to get a typed response.
    let typed: MoveTaskResult = serde_json::from_value(result)
        .map_err(|e| ApiError::Internal(format!("move_task result decode: {e}")))?;
    Ok(Json(typed))
}

#[utoipa::path(
    get,
    path = "/v1/tasks/next",
    tag = "tasks",
    params(NextTasksQuery),
    responses(
        (status = 200, body = NextTasksResult, description = "Open tasks (todo, backlog) whose dependencies are finished, best first: todo before backlog, then priority, due date and ID"),
        (status = 401, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn next_tasks(
    State(state): State<AppState>,
    Query(q): Query<NextTasksQuery>,
) -> Result<Json<NextTasksResult>, ApiError> {
    let next = next_tasks_json(
        &state.cfg,
        q.project.as_deref(),
        q.customer.as_deref(),
        q.owner.as_deref(),
        q.limit,
    )?;
    let typed: NextTasksResult = serde_json::from_value(next)
        .map_err(|e| ApiError::Internal(format!("next_tasks result decode: {e}")))?;
    Ok(Json(typed))
}

#[utoipa::path(
    patch,
    path = "/v1/tasks/{id}",
    tag = "tasks",
    params(("id" = String, Path, description = "Task ID, e.g. TASK-001 (task-1 works too)")),
    request_body = UpdateTask,
    responses(
        (status = 200, body = UpdateTaskResult),
        (status = 400, body = crate::api::error::ProblemJson, description = "Invalid value, or nothing to change"),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson, description = "Read-only or missing write capability"),
        (status = 404, body = crate::api::error::ProblemJson, description = "Unknown task, or a referenced entity doesn't exist")
    ),
    security(("bearer" = []))
)]
pub async fn update_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTask>,
) -> Result<Json<UpdateTaskResult>, ApiError> {
    let cfg = &state.cfg;
    let _write_guard = state.write_lock.lock().await;
    let rec = resolve_entity(cfg, &id, Some(EntityKind::Task))?;
    let list = |v: Option<String>| v.as_deref().map(parse_comma_list);
    let update = task::TaskUpdate {
        title: body.title,
        status: body.status,
        priority: body.priority,
        owner: body.owner,
        sprint: body.sprint,
        milestone: body.milestone,
        due_date: body.due_date,
        projects: list(body.projects),
        customers: list(body.customers),
        tags: list(body.tags),
        depends_on: list(body.depends_on),
    };
    let updated = task::update_task(cfg, &rec.id, &update)?;
    let typed: UpdateTaskResult = serde_json::from_value(updated.to_json(cfg))
        .map_err(|e| ApiError::Internal(format!("update_task result decode: {e}")))?;
    Ok(Json(typed))
}
