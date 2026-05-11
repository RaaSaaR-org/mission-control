//! POST endpoints for entity creation.
//!
//! Each handler is a thin wrapper over `commands::new::create_*_programmatic`
//! that holds the global write lock for the duration of the call. The lock
//! serializes ID allocation (which is a scan-and-increment) and prevents
//! TOCTOU races between concurrent POSTs.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::Value as JsonValue;

use crate::api::error::ApiError;
use crate::api::schemas::{
    CreateContact, CreateCustomer, CreateMeeting, CreateProject, CreateProposal, CreateResearch,
    CreateResult, CreateSprint, CreateTask,
};
use crate::api::AppState;
use crate::commands::new as new_cmd;

fn into_create_result(v: JsonValue) -> Result<CreateResult, ApiError> {
    serde_json::from_value(v).map_err(|e| ApiError::Internal(format!("create result decode: {e}")))
}

#[utoipa::path(
    post, path = "/v1/customers", tag = "entities",
    request_body = CreateCustomer,
    responses(
        (status = 201, body = CreateResult),
        (status = 400, body = crate::api::error::ProblemJson),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn create_customer(
    State(state): State<AppState>,
    Json(body): Json<CreateCustomer>,
) -> Result<(StatusCode, Json<CreateResult>), ApiError> {
    let _w = state.write_lock.lock().await;
    let v = new_cmd::create_customer_programmatic(
        &state.cfg,
        &body.name,
        body.owner.as_deref(),
        body.status.as_deref(),
        body.tags.as_deref(),
    )?;
    Ok((StatusCode::CREATED, Json(into_create_result(v)?)))
}

#[utoipa::path(
    post, path = "/v1/projects", tag = "entities",
    request_body = CreateProject,
    responses(
        (status = 201, body = CreateResult),
        (status = 400, body = crate::api::error::ProblemJson),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn create_project(
    State(state): State<AppState>,
    Json(body): Json<CreateProject>,
) -> Result<(StatusCode, Json<CreateResult>), ApiError> {
    let _w = state.write_lock.lock().await;
    let v = new_cmd::create_project_programmatic(
        &state.cfg,
        &body.name,
        body.owner.as_deref(),
        body.status.as_deref(),
        body.customers.as_deref(),
        body.tags.as_deref(),
    )?;
    Ok((StatusCode::CREATED, Json(into_create_result(v)?)))
}

#[utoipa::path(
    post, path = "/v1/meetings", tag = "entities",
    request_body = CreateMeeting,
    responses(
        (status = 201, body = CreateResult),
        (status = 400, body = crate::api::error::ProblemJson),
        (status = 401, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn create_meeting(
    State(state): State<AppState>,
    Json(body): Json<CreateMeeting>,
) -> Result<(StatusCode, Json<CreateResult>), ApiError> {
    let _w = state.write_lock.lock().await;
    let v = new_cmd::create_meeting_programmatic(
        &state.cfg,
        &body.title,
        body.date.as_deref(),
        body.time.as_deref(),
        body.duration.as_deref(),
        body.status.as_deref(),
        body.tags.as_deref(),
        body.customers.as_deref(),
        body.projects.as_deref(),
        body.attendees.as_deref(),
    )?;
    Ok((StatusCode::CREATED, Json(into_create_result(v)?)))
}

#[utoipa::path(
    post, path = "/v1/research", tag = "entities",
    request_body = CreateResearch,
    responses(
        (status = 201, body = CreateResult),
        (status = 400, body = crate::api::error::ProblemJson),
        (status = 401, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn create_research(
    State(state): State<AppState>,
    Json(body): Json<CreateResearch>,
) -> Result<(StatusCode, Json<CreateResult>), ApiError> {
    let _w = state.write_lock.lock().await;
    let v = new_cmd::create_research_programmatic(
        &state.cfg,
        &body.title,
        body.owner.as_deref(),
        body.agents.as_deref(),
        body.tags.as_deref(),
    )?;
    Ok((StatusCode::CREATED, Json(into_create_result(v)?)))
}

#[utoipa::path(
    post, path = "/v1/tasks", tag = "entities",
    request_body = CreateTask,
    responses(
        (status = 201, body = CreateResult),
        (status = 400, body = crate::api::error::ProblemJson),
        (status = 401, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<CreateTask>,
) -> Result<(StatusCode, Json<CreateResult>), ApiError> {
    let _w = state.write_lock.lock().await;
    let v = new_cmd::create_task_programmatic(
        &state.cfg,
        &body.title,
        body.project.as_deref(),
        body.customer.as_deref(),
        body.owner.as_deref(),
        body.status.as_deref(),
        body.priority,
        body.tags.as_deref(),
        body.sprint.as_deref(),
        body.depends_on.as_deref(),
        body.due_date.as_deref(),
    )?;
    Ok((StatusCode::CREATED, Json(into_create_result(v)?)))
}

#[utoipa::path(
    post, path = "/v1/sprints", tag = "entities",
    request_body = CreateSprint,
    responses(
        (status = 201, body = CreateResult),
        (status = 400, body = crate::api::error::ProblemJson),
        (status = 401, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn create_sprint(
    State(state): State<AppState>,
    Json(body): Json<CreateSprint>,
) -> Result<(StatusCode, Json<CreateResult>), ApiError> {
    let _w = state.write_lock.lock().await;
    let v = new_cmd::create_sprint_programmatic(
        &state.cfg,
        &body.title,
        body.owner.as_deref(),
        body.status.as_deref(),
        body.goal.as_deref(),
        body.start_date.as_deref(),
        body.end_date.as_deref(),
        body.projects.as_deref(),
        body.tags.as_deref(),
    )?;
    Ok((StatusCode::CREATED, Json(into_create_result(v)?)))
}

#[utoipa::path(
    post, path = "/v1/proposals", tag = "entities",
    request_body = CreateProposal,
    responses(
        (status = 201, body = CreateResult),
        (status = 400, body = crate::api::error::ProblemJson),
        (status = 401, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn create_proposal(
    State(state): State<AppState>,
    Json(body): Json<CreateProposal>,
) -> Result<(StatusCode, Json<CreateResult>), ApiError> {
    let _w = state.write_lock.lock().await;
    let v = new_cmd::create_proposal_programmatic(
        &state.cfg,
        &body.title,
        body.author.as_deref(),
        body.status.as_deref(),
        body.proposal_type.as_deref(),
        body.tags.as_deref(),
        body.supersedes.as_deref(),
    )?;
    Ok((StatusCode::CREATED, Json(into_create_result(v)?)))
}

#[utoipa::path(
    post, path = "/v1/contacts", tag = "entities",
    request_body = CreateContact,
    responses(
        (status = 201, body = CreateResult),
        (status = 400, body = crate::api::error::ProblemJson),
        (status = 401, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn create_contact(
    State(state): State<AppState>,
    Json(body): Json<CreateContact>,
) -> Result<(StatusCode, Json<CreateResult>), ApiError> {
    let _w = state.write_lock.lock().await;
    let v = new_cmd::create_contact_programmatic(
        &state.cfg,
        &body.name,
        &body.customer,
        body.role.as_deref(),
        body.email.as_deref(),
        body.phone.as_deref(),
        body.status.as_deref(),
        body.tags.as_deref(),
    )?;
    Ok((StatusCode::CREATED, Json(into_create_result(v)?)))
}
