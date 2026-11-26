//! Lead pipeline and event endpoints (CRM).
//!
//! `GET /v1/leads` exists alongside `/v1/entities/lead` because leads have a
//! richer filter set (event, warmth, introduce_to, due follow-ups). Writes
//! go through `commands::new` / `commands::lead` under the write lock, like
//! every other endpoint.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;

use crate::api::error::ApiError;
use crate::api::schemas::{
    ConvertLeadBody, ConvertLeadResult, CreateEvent, CreateLead, CreateResult, InteractionResult,
    LeadListQuery, LogInteractionBody, MoveLeadBody, UpdateLead, UpdateLeadResult,
};
use crate::api::AppState;
use crate::commands::lead;
use crate::commands::new as new_cmd;
use crate::entity::EntityKind;
use crate::mcp::{leads_json, resolve_entity, LeadQuery};
use crate::util::parse_comma_list;

type CreateResponse = Result<(StatusCode, Json<CreateResult>), ApiError>;

fn list(v: Option<String>) -> Vec<String> {
    v.as_deref().map(parse_comma_list).unwrap_or_default()
}

/// Re-type a command's JSON result as its response schema.
fn typed<T: serde::de::DeserializeOwned>(what: &str, v: serde_json::Value) -> Result<T, ApiError> {
    serde_json::from_value(v).map_err(|e| ApiError::Internal(format!("{what} result decode: {e}")))
}

#[utoipa::path(
    get,
    path = "/v1/leads",
    tag = "leads",
    params(LeadListQuery),
    responses(
        (status = 200, description = "Array of leads matching the filters, shaped like GET /v1/entities/lead; sorted by ID, or by next_follow_up with follow_up_due=true"),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson, description = "Leads are not enabled in this repo")
    ),
    security(("bearer" = []))
)]
pub async fn list_leads(
    State(state): State<AppState>,
    Query(q): Query<LeadListQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let leads = leads_json(
        &state.cfg,
        &LeadQuery {
            status: q.status.as_deref(),
            event: q.event.as_deref(),
            tag: q.tag.as_deref(),
            owner: q.owner.as_deref(),
            warmth: q.warmth.as_deref(),
            introduce_to: q.introduce_to.as_deref(),
            follow_up_due: q.follow_up_due.unwrap_or(false),
        },
    )?;
    Ok(Json(serde_json::Value::Array(leads)))
}

#[utoipa::path(
    post, path = "/v1/leads", tag = "leads",
    request_body = CreateLead,
    responses(
        (status = 201, body = CreateResult),
        (status = 400, body = crate::api::error::ProblemJson),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson),
        (status = 404, body = crate::api::error::ProblemJson, description = "A referenced event or customer doesn't exist")
    ),
    security(("bearer" = []))
)]
pub async fn create_lead(
    State(state): State<AppState>,
    Json(body): Json<CreateLead>,
) -> CreateResponse {
    let _w = state.write_lock.lock().await;
    let input = new_cmd::LeadInput {
        name: body.name,
        company: body.company,
        role: body.role,
        email: body.email,
        phone: body.phone,
        linkedin: body.linkedin,
        website: body.website,
        source: body.source,
        event: body.event,
        introduce_to: list(body.introduce_to),
        warmth: body.warmth,
        next_follow_up: body.next_follow_up,
        owner: body.owner,
        status: body.status,
        tags: list(body.tags),
        notes: body.notes,
    };
    let created = new_cmd::create_lead(&state.cfg, &input)?;
    Ok((
        StatusCode::CREATED,
        Json(CreateResult::new(created, &state.cfg)),
    ))
}

#[utoipa::path(
    patch,
    path = "/v1/leads/{id}",
    tag = "leads",
    params(("id" = String, Path, description = "Lead ID, e.g. LEAD-001 (lead-1 works too)")),
    request_body = UpdateLead,
    responses(
        (status = 200, body = UpdateLeadResult),
        (status = 400, body = crate::api::error::ProblemJson, description = "Invalid value, or nothing to change"),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson),
        (status = 404, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn update_lead(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateLead>,
) -> Result<Json<UpdateLeadResult>, ApiError> {
    let cfg = &state.cfg;
    let _w = state.write_lock.lock().await;
    let rec = resolve_entity(cfg, &id, Some(EntityKind::Lead))?;
    let opt_list = |v: Option<String>| v.as_deref().map(parse_comma_list);
    let update = lead::LeadUpdate {
        name: body.name,
        company: body.company,
        role: body.role,
        email: body.email,
        phone: body.phone,
        linkedin: body.linkedin,
        website: body.website,
        source: body.source,
        event: body.event,
        introduce_to: opt_list(body.introduce_to),
        warmth: body.warmth,
        next_follow_up: body.next_follow_up,
        last_contact: body.last_contact,
        owner: body.owner,
        status: body.status,
        tags: opt_list(body.tags),
    };
    let updated = lead::update_lead(cfg, &rec.id, &update)?;
    Ok(Json(typed("update_lead", updated.to_json(cfg))?))
}

#[utoipa::path(
    post,
    path = "/v1/leads/{id}/move",
    tag = "leads",
    params(("id" = String, Path, description = "Lead ID, e.g. LEAD-001")),
    request_body = MoveLeadBody,
    responses(
        (status = 200, body = UpdateLeadResult),
        (status = 400, body = crate::api::error::ProblemJson, description = "Invalid status"),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson),
        (status = 404, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn move_lead(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<MoveLeadBody>,
) -> Result<Json<UpdateLeadResult>, ApiError> {
    let cfg = &state.cfg;
    let _w = state.write_lock.lock().await;
    let rec = resolve_entity(cfg, &id, Some(EntityKind::Lead))?;
    let updated = lead::move_lead(cfg, &rec.id, &body.status)?;
    Ok(Json(typed("move_lead", updated.to_json(cfg))?))
}

#[utoipa::path(
    post,
    path = "/v1/leads/{id}/interactions",
    tag = "leads",
    params(("id" = String, Path, description = "Lead ID, e.g. LEAD-001")),
    request_body = LogInteractionBody,
    responses(
        (status = 200, body = InteractionResult),
        (status = 400, body = crate::api::error::ProblemJson, description = "Invalid type, date or empty note"),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson),
        (status = 404, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn log_interaction(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<LogInteractionBody>,
) -> Result<Json<InteractionResult>, ApiError> {
    let cfg = &state.cfg;
    let _w = state.write_lock.lock().await;
    let rec = resolve_entity(cfg, &id, Some(EntityKind::Lead))?;
    let logged = lead::log_interaction(
        cfg,
        &rec.id,
        &lead::Interaction {
            date: body.date,
            kind: body.interaction_type,
            note: body.note,
            next_follow_up: body.next_follow_up,
        },
    )?;
    Ok(Json(typed("log_interaction", logged.to_json(cfg))?))
}

#[utoipa::path(
    post,
    path = "/v1/leads/{id}/convert",
    tag = "leads",
    params(("id" = String, Path, description = "Lead ID, e.g. LEAD-001")),
    request_body = ConvertLeadBody,
    responses(
        (status = 200, body = ConvertLeadResult),
        (status = 400, body = crate::api::error::ProblemJson, description = "No customer to convert into, or both customer and new_customer_name given"),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson),
        (status = 404, body = crate::api::error::ProblemJson),
        (status = 409, body = crate::api::error::ProblemJson, description = "The lead is already converted, or the customer already has a contact for this person (retry with link_existing)")
    ),
    security(("bearer" = []))
)]
pub async fn convert_lead(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ConvertLeadBody>,
) -> Result<Json<ConvertLeadResult>, ApiError> {
    let cfg = &state.cfg;
    let _w = state.write_lock.lock().await;
    let rec = resolve_entity(cfg, &id, Some(EntityKind::Lead))?;
    let converted = lead::convert_lead(
        cfg,
        &rec.id,
        &lead::ConvertInput {
            customer: body.customer,
            new_customer_name: body.new_customer_name,
            link_existing: body.link_existing.unwrap_or(false),
        },
    )?;
    Ok(Json(typed("convert_lead", converted.to_json(cfg))?))
}

#[utoipa::path(
    post, path = "/v1/events", tag = "leads",
    request_body = CreateEvent,
    responses(
        (status = 201, body = CreateResult),
        (status = 400, body = crate::api::error::ProblemJson),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn create_event(
    State(state): State<AppState>,
    Json(body): Json<CreateEvent>,
) -> CreateResponse {
    let _w = state.write_lock.lock().await;
    let input = new_cmd::EventInput {
        title: body.title,
        date: body.date,
        end_date: body.end_date,
        location: body.location,
        event_type: body.event_type,
        url: body.url,
        owner: body.owner,
        status: body.status,
        tags: list(body.tags),
    };
    let created = new_cmd::create_event(&state.cfg, &input)?;
    Ok((
        StatusCode::CREATED,
        Json(CreateResult::new(created, &state.cfg)),
    ))
}
