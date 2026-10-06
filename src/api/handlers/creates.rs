//! POST endpoints for entity creation.
//!
//! Each handler maps its request body onto the matching `commands::new`
//! input struct and calls `create_*` while holding the global write lock. The
//! lock serializes ID allocation (which is a scan-and-increment) and prevents
//! TOCTOU races between concurrent POSTs.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::api::error::ApiError;
use crate::api::schemas::{
    CreateContact, CreateCustomer, CreateMeeting, CreateProject, CreateProposal, CreateResearch,
    CreateResult, CreateSprint, CreateTask,
};
use crate::api::AppState;
use crate::commands::new as new_cmd;
use crate::util::parse_comma_list;

type CreateResponse = Result<(StatusCode, Json<CreateResult>), ApiError>;

fn list(v: Option<String>) -> Vec<String> {
    v.as_deref().map(parse_comma_list).unwrap_or_default()
}

fn created(c: new_cmd::Created) -> CreateResponse {
    Ok((StatusCode::CREATED, Json(c.into())))
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
) -> CreateResponse {
    let _w = state.write_lock.lock().await;
    let input = new_cmd::CustomerInput {
        name: body.name,
        owner: body.owner,
        status: body.status,
        tags: list(body.tags),
    };
    created(new_cmd::create_customer(&state.cfg, &input)?)
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
) -> CreateResponse {
    let _w = state.write_lock.lock().await;
    let input = new_cmd::ProjectInput {
        name: body.name,
        owner: body.owner,
        status: body.status,
        customers: list(body.customers),
        tags: list(body.tags),
    };
    created(new_cmd::create_project(&state.cfg, &input)?)
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
) -> CreateResponse {
    let _w = state.write_lock.lock().await;
    let input = new_cmd::MeetingInput {
        title: body.title,
        date: body.date,
        time: body.time,
        duration: body.duration,
        status: body.status,
        tags: list(body.tags),
        customers: list(body.customers),
        projects: list(body.projects),
        attendees: list(body.attendees),
    };
    created(new_cmd::create_meeting(&state.cfg, &input)?)
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
) -> CreateResponse {
    let _w = state.write_lock.lock().await;
    let input = new_cmd::ResearchInput {
        title: body.title,
        owner: body.owner,
        agents: body.agents.as_deref().map(parse_comma_list),
        tags: list(body.tags),
    };
    created(new_cmd::create_research(&state.cfg, &input)?)
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
) -> CreateResponse {
    let _w = state.write_lock.lock().await;
    let input = new_cmd::TaskInput {
        title: body.title,
        project: body.project,
        customer: body.customer,
        owner: body.owner,
        status: body.status,
        priority: body.priority,
        tags: list(body.tags),
        sprint: body.sprint,
        depends_on: list(body.depends_on),
        due_date: body.due_date,
    };
    created(new_cmd::create_task(&state.cfg, &input)?)
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
) -> CreateResponse {
    let _w = state.write_lock.lock().await;
    let input = new_cmd::SprintInput {
        title: body.title,
        owner: body.owner,
        status: body.status,
        goal: body.goal,
        start_date: body.start_date,
        end_date: body.end_date,
        projects: list(body.projects),
        tags: list(body.tags),
    };
    created(new_cmd::create_sprint(&state.cfg, &input)?)
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
) -> CreateResponse {
    let _w = state.write_lock.lock().await;
    let input = new_cmd::ProposalInput {
        title: body.title,
        author: body.author,
        status: body.status,
        proposal_type: body.proposal_type,
        tags: list(body.tags),
        supersedes: body.supersedes,
    };
    created(new_cmd::create_proposal(&state.cfg, &input)?)
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
) -> CreateResponse {
    let _w = state.write_lock.lock().await;
    let input = new_cmd::ContactInput {
        name: body.name,
        customer: body.customer,
        role: body.role,
        email: body.email,
        phone: body.phone,
        status: body.status,
        tags: list(body.tags),
    };
    created(new_cmd::create_contact(&state.cfg, &input)?)
}
