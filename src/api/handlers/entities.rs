//! Read endpoints over arbitrary entity kinds.
//!
//! - `GET /v1/entities/{kind}` — list, with optional `status` and `tag` filters.
//! - `GET /v1/entities/{kind}/{id}` — single entity, parsed.
//! - `GET /v1/entities/{kind}/{id}/raw` — single entity as raw markdown.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;

use crate::api::error::ApiError;
use crate::api::schemas::{EntityListQuery, EntityResponse};
use crate::api::AppState;
use crate::data;
use crate::entity::EntityKind;
use crate::error::McError;

/// Resolve the path segment to an `EntityKind`, or 400.
fn parse_kind(s: &str) -> Result<EntityKind, ApiError> {
    EntityKind::from_str_loose(s)
        .map_err(|_| ApiError::BadRequest(format!("unknown entity kind: {s}")))
}

#[utoipa::path(
    get,
    path = "/v1/entities/{kind}",
    tag = "entities",
    params(
        ("kind" = String, Path, description = "Entity kind: customer, project, meeting, research, task, sprint, proposal, contact"),
        EntityListQuery
    ),
    responses(
        (status = 200, description = "Array of entities (frontmatter as JSON, free-form per kind)"),
        (status = 400, body = crate::api::error::ProblemJson, description = "Unknown entity kind"),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 403, body = crate::api::error::ProblemJson, description = "Kind not available in this repo mode")
    ),
    security(("bearer" = []))
)]
pub async fn list_entities(
    State(state): State<AppState>,
    Path(kind): Path<String>,
    Query(query): Query<EntityListQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let kind = parse_kind(&kind)?;
    if !state.cfg.entity_available(&kind) {
        return Err(ApiError::Domain(McError::NotAvailableInMode {
            kind: kind.label().into(),
        }));
    }

    let entries = data::collect_filtered(
        kind,
        &state.cfg,
        query.status.as_deref(),
        query.tag.as_deref(),
    )?;

    let json: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            let mut v = data::yaml_to_json(&e.frontmatter);
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "_kind".into(),
                    serde_json::Value::String(e.kind.label().into()),
                );
                obj.insert(
                    "_source".into(),
                    serde_json::Value::String(e.source_path.display().to_string()),
                );
            }
            v
        })
        .collect();

    Ok(Json(serde_json::Value::Array(json)))
}

#[utoipa::path(
    get,
    path = "/v1/entities/{kind}/{id}",
    tag = "entities",
    params(
        ("kind" = String, Path, description = "Entity kind"),
        ("id" = String, Path, description = "Entity ID, e.g. CUST-001")
    ),
    responses(
        (status = 200, body = EntityResponse),
        (status = 400, body = crate::api::error::ProblemJson),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 404, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn get_entity(
    State(state): State<AppState>,
    Path((kind_str, id)): Path<(String, String)>,
) -> Result<Json<EntityResponse>, ApiError> {
    let kind = parse_kind(&kind_str)?;
    let record = data::find_entity_by_id(&id, &state.cfg)?;
    if record.kind != kind {
        return Err(ApiError::Domain(McError::EntityNotFound(format!(
            "{id} (not a {})",
            kind.label()
        ))));
    }

    let body_preview = preview(&record.body, 500);
    Ok(Json(EntityResponse {
        kind: record.kind.label().into(),
        id: record.id.clone(),
        source_path: record.source_path.display().to_string(),
        frontmatter: data::yaml_to_json(&record.frontmatter),
        body_preview,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/entities/{kind}/{id}/raw",
    tag = "entities",
    params(
        ("kind" = String, Path),
        ("id" = String, Path)
    ),
    responses(
        (status = 200, description = "Raw markdown file (text/markdown)", content_type = "text/markdown"),
        (status = 401, body = crate::api::error::ProblemJson),
        (status = 404, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn get_entity_raw(
    State(state): State<AppState>,
    Path((kind_str, id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let kind = parse_kind(&kind_str)?;
    let record = data::find_entity_by_id(&id, &state.cfg)?;
    if record.kind != kind {
        return Err(ApiError::Domain(McError::EntityNotFound(format!(
            "{id} (not a {})",
            kind.label()
        ))));
    }

    let raw = std::fs::read_to_string(&record.source_path)
        .map_err(|e| ApiError::Internal(format!("read entity file: {e}")))?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/markdown; charset=utf-8"),
    );
    Ok((StatusCode::OK, headers, raw))
}

fn preview(body: &str, max: usize) -> String {
    if body.len() <= max {
        body.to_string()
    } else {
        // Truncate on a char boundary, append ellipsis.
        let mut end = max;
        while end > 0 && !body.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &body[..end])
    }
}
