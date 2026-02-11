use crate::config::ResolvedConfig;
use crate::data::{self, TaskFilter};
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::html;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use std::collections::HashMap;
use std::sync::Arc;

struct AppState {
    cfg: ResolvedConfig,
}

pub fn run(cfg: &ResolvedConfig, port: u16) -> McResult<()> {
    let state = Arc::new(AppState { cfg: cfg.clone() });

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let app = Router::new()
            .route("/", get(handle_dashboard))
            .route("/customers", get(handle_customers))
            .route("/projects", get(handle_projects))
            .route("/meetings", get(handle_meetings))
            .route("/research", get(handle_research))
            .route("/sprints", get(handle_sprints))
            .route("/proposals", get(handle_proposals))
            .route("/tasks", get(handle_tasks))
            .route("/tasks/board", get(handle_tasks_board))
            .route("/entity/{id}", get(handle_detail))
            .fallback(handle_404)
            .with_state(state);

        let addr = format!("127.0.0.1:{}", port);
        println!("MissionControl web dashboard: http://{}", addr);
        println!("Press Ctrl+C to stop.");

        let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                McError::Other(format!(
                    "Port {} is already in use. Try a different port with: mc serve --port <PORT>",
                    port
                ))
            } else {
                McError::Io(e)
            }
        })?;
        axum::serve(listener, app).await.map_err(McError::Io)?;
        Ok(())
    })
}

async fn handle_dashboard(State(state): State<Arc<AppState>>) -> Html<String> {
    let cfg = &state.cfg;

    let kinds = [
        EntityKind::Customer,
        EntityKind::Project,
        EntityKind::Meeting,
        EntityKind::Research,
        EntityKind::Task,
        EntityKind::Sprint,
        EntityKind::Proposal,
    ];

    let counts: Vec<data::StatusCounts> = kinds
        .iter()
        .filter_map(|k| data::count_by_status(*k, cfg).ok())
        .collect();

    let recent = match data::recent_activity(cfg, 10) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("serve: error loading recent activity: {}", e);
            Vec::new()
        }
    };

    Html(html::dashboard_page(&counts, &recent))
}

async fn handle_customers(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Customer, &state.cfg, &params)
}

async fn handle_projects(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Project, &state.cfg, &params)
}

async fn handle_meetings(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Meeting, &state.cfg, &params)
}

async fn handle_research(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Research, &state.cfg, &params)
}

async fn handle_sprints(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Sprint, &state.cfg, &params)
}

async fn handle_proposals(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Proposal, &state.cfg, &params)
}

async fn handle_tasks(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let cfg = &state.cfg;
    let status_filter = params
        .get("status")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str());

    let filter = TaskFilter {
        status: status_filter,
        tag: None,
        project: None,
        customer: None,
        priority: None,
        sprint: None,
        owner: None,
    };

    let tasks = data::collect_tasks_filtered(cfg, &filter).map_err(|e| {
        eprintln!("serve: error loading tasks: {}", e);
        error_response(&e.to_string())
    })?;
    let valid_statuses = EntityKind::Task.statuses(cfg);

    Ok(Html(html::tasks_list_page(
        &tasks,
        status_filter,
        valid_statuses,
    )))
}

async fn handle_tasks_board(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let cfg = &state.cfg;
    let project = params
        .get("project")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str());
    let customer = params
        .get("customer")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str());
    let sprint = params
        .get("sprint")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str());

    let filter = TaskFilter {
        status: None,
        tag: None,
        project,
        customer,
        priority: None,
        sprint,
        owner: None,
    };

    let tasks = data::collect_tasks_filtered(cfg, &filter).map_err(|e| {
        eprintln!("serve: error loading tasks: {}", e);
        error_response(&e.to_string())
    })?;

    Ok(Html(html::board_page(&tasks)))
}

fn handle_list(
    kind: EntityKind,
    cfg: &ResolvedConfig,
    params: &HashMap<String, String>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let status_filter = params
        .get("status")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str());
    let tag_filter = params
        .get("tag")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str());

    let entities = data::collect_filtered(kind, cfg, status_filter, tag_filter).map_err(|e| {
        eprintln!("serve: error loading {}: {}", kind.label_plural(), e);
        error_response(&e.to_string())
    })?;

    let valid_statuses = kind.statuses(cfg);

    Ok(Html(html::list_page(
        kind.label_plural(),
        &entities,
        status_filter,
        tag_filter,
        valid_statuses,
    )))
}

async fn handle_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Html<String>, StatusCode> {
    let cfg = &state.cfg;

    let entity = data::find_entity_by_id(&id, cfg).map_err(|_| StatusCode::NOT_FOUND)?;

    let prefixes = vec![
        cfg.id_prefixes.customer.as_str(),
        cfg.id_prefixes.project.as_str(),
        cfg.id_prefixes.meeting.as_str(),
        cfg.id_prefixes.research.as_str(),
        cfg.id_prefixes.task.as_str(),
        cfg.id_prefixes.sprint.as_str(),
        cfg.id_prefixes.proposal.as_str(),
    ];

    Ok(Html(html::detail_page(&entity, &prefixes)))
}

fn error_response(message: &str) -> (StatusCode, Html<String>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page(message)),
    )
}

async fn handle_404(uri: axum::http::Uri) -> Html<String> {
    Html(html::not_found_page(uri.path()))
}
