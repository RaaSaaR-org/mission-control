//! `/v1/config` and `/v1/status` — repo-level metadata.

use axum::extract::State;
use axum::Json;

use crate::api::error::ApiError;
use crate::api::schemas::{
    ConfigResponse, KindStatusCounts, PrefixView, RecentEntry, StatusCount, StatusResponse,
    StatusView,
};
use crate::api::AppState;
use crate::config::RepoMode;
use crate::data;
use crate::entity::EntityKind;

#[utoipa::path(
    get,
    path = "/v1/config",
    tag = "meta",
    responses(
        (status = 200, body = ConfigResponse, description = "Repository configuration"),
        (status = 401, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn get_config(State(state): State<AppState>) -> Json<ConfigResponse> {
    let cfg = &state.cfg;
    let mode = match cfg.mode {
        RepoMode::Standalone => "standalone".to_string(),
        RepoMode::Embedded => "embedded".to_string(),
    };
    Json(ConfigResponse {
        mode,
        prefixes: PrefixView {
            customer: cfg.id_prefixes.customer.clone(),
            project: cfg.id_prefixes.project.clone(),
            meeting: cfg.id_prefixes.meeting.clone(),
            research: cfg.id_prefixes.research.clone(),
            task: cfg.id_prefixes.task.clone(),
            sprint: cfg.id_prefixes.sprint.clone(),
            proposal: cfg.id_prefixes.proposal.clone(),
            contact: cfg.id_prefixes.contact.clone(),
        },
        statuses: StatusView {
            customer: cfg.statuses.customer.clone(),
            project: cfg.statuses.project.clone(),
            meeting: cfg.statuses.meeting.clone(),
            research: cfg.statuses.research.clone(),
            task: cfg.statuses.task.clone(),
            sprint: cfg.statuses.sprint.clone(),
            proposal: cfg.statuses.proposal.clone(),
            contact: cfg.statuses.contact.clone(),
        },
        configured_entities: cfg.configured_entities.iter().cloned().collect(),
    })
}

#[utoipa::path(
    get,
    path = "/v1/status",
    tag = "meta",
    responses(
        (status = 200, body = StatusResponse, description = "Counts by status + recent activity"),
        (status = 401, body = crate::api::error::ProblemJson)
    ),
    security(("bearer" = []))
)]
pub async fn get_status(State(state): State<AppState>) -> Result<Json<StatusResponse>, ApiError> {
    let cfg = &state.cfg;
    let kinds = [
        EntityKind::Customer,
        EntityKind::Project,
        EntityKind::Meeting,
        EntityKind::Research,
        EntityKind::Task,
        EntityKind::Sprint,
        EntityKind::Proposal,
        EntityKind::Contact,
    ];

    let mut counts: Vec<KindStatusCounts> = Vec::new();
    for k in kinds {
        if !cfg.entity_available(&k) {
            continue;
        }
        let c = data::count_by_status(k, cfg)?;
        counts.push(KindStatusCounts {
            kind: c.label,
            total: c.total,
            by_status: c
                .by_status
                .into_iter()
                .map(|(status, count)| StatusCount { status, count })
                .collect(),
        });
    }

    let recent_raw = data::recent_activity(cfg, 15)?;
    let recent: Vec<RecentEntry> = recent_raw
        .into_iter()
        .map(|r| {
            let modified = chrono::DateTime::<chrono::Utc>::from(r.modified)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            RecentEntry {
                id: r.id,
                name: r.name,
                modified,
            }
        })
        .collect();

    Ok(Json(StatusResponse { counts, recent }))
}
