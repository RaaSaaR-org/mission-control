use crate::config::ResolvedConfig;
use crate::data::{self, ContactFilter, TaskFilter};
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::frontmatter;
use crate::html;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use std::collections::HashMap;
use std::sync::Arc;

struct AppState {
    cfg: ResolvedConfig,
    /// Cached custom CSS content (read once at startup).
    custom_css: String,
}

pub fn run(cfg: &ResolvedConfig, port: u16) -> McResult<()> {
    // Read custom CSS once at startup
    let custom_css = cfg
        .brand
        .custom_css
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();

    let state = Arc::new(AppState {
        cfg: cfg.clone(),
        custom_css,
    });

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
            .route("/contacts", get(handle_contacts))
            .route("/tasks", get(handle_tasks))
            .route("/tasks/board", get(handle_tasks_board))
            .route("/entity/{id}", get(handle_detail))
            .route("/brand/logo", get(handle_brand_logo))
            .route("/brand/fonts/{filename}", get(handle_brand_fonts))
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
        EntityKind::Contact,
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

    Html(html::dashboard_page(
        &counts,
        &recent,
        &cfg.mode,
        &cfg.brand,
        &state.custom_css,
    ))
}

async fn handle_customers(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Customer, &state.cfg, &params, &state.custom_css)
}

async fn handle_projects(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Project, &state.cfg, &params, &state.custom_css)
}

async fn handle_meetings(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Meeting, &state.cfg, &params, &state.custom_css)
}

async fn handle_research(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Research, &state.cfg, &params, &state.custom_css)
}

async fn handle_sprints(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Sprint, &state.cfg, &params, &state.custom_css)
}

async fn handle_proposals(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Proposal, &state.cfg, &params, &state.custom_css)
}

async fn handle_contacts(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    handle_list(EntityKind::Contact, &state.cfg, &params, &state.custom_css)
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
    let priority_filter = params
        .get("priority")
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<u32>().ok());
    let owner_filter = params
        .get("owner")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str());
    let project_filter = params
        .get("project")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str());
    let sprint_filter = params
        .get("sprint")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str());

    let filter = TaskFilter {
        status: status_filter,
        tag: None,
        project: project_filter,
        customer: None,
        priority: priority_filter,
        sprint: sprint_filter,
        owner: owner_filter,
    };

    let tasks = data::collect_tasks_filtered(cfg, &filter).map_err(|e| {
        eprintln!("serve: error loading tasks: {}", e);
        error_response(&e.to_string())
    })?;
    let valid_statuses = EntityKind::Task.statuses(cfg);

    // Collect unique owners, projects, sprints for filter dropdowns
    let all_tasks = data::collect_tasks_filtered(
        cfg,
        &TaskFilter {
            status: None,
            tag: None,
            project: None,
            customer: None,
            priority: None,
            sprint: None,
            owner: None,
        },
    )
    .unwrap_or_default();
    let filter_options = html::TaskFilterOptions::from_tasks(&all_tasks);

    Ok(Html(html::tasks_list_page(
        &tasks,
        status_filter,
        priority_filter,
        owner_filter,
        project_filter,
        sprint_filter,
        valid_statuses,
        &filter_options,
        &cfg.brand,
        &state.custom_css,
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

    Ok(Html(html::board_page(
        &tasks,
        &cfg.brand,
        &state.custom_css,
    )))
}

fn handle_list(
    kind: EntityKind,
    cfg: &ResolvedConfig,
    params: &HashMap<String, String>,
    custom_css: &str,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let status_filter = params
        .get("status")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str());
    let tag_filter = params
        .get("tag")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str());
    let sort_field = params
        .get("sort")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str());
    let sort_dir = params
        .get("dir")
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str())
        .unwrap_or("asc");

    let mut entities =
        data::collect_filtered(kind, cfg, status_filter, tag_filter).map_err(|e| {
            eprintln!("serve: error loading {}: {}", kind.label_plural(), e);
            error_response(&e.to_string())
        })?;

    // Sort entities if requested
    if let Some(field) = sort_field {
        html::sort_entities(&mut entities, field, sort_dir);
    }

    let valid_statuses = kind.statuses(cfg);

    Ok(Html(html::list_page(
        kind.label_plural(),
        &entities,
        status_filter,
        tag_filter,
        valid_statuses,
        sort_field,
        sort_dir,
        &cfg.mode,
        &cfg.brand,
        custom_css,
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
        cfg.id_prefixes.contact.as_str(),
    ];

    // Collect related entities based on entity kind
    let related = collect_related_entities(&entity, cfg);

    Ok(Html(html::detail_page(
        &entity,
        &prefixes,
        &related,
        cfg,
        &state.custom_css,
    )))
}

/// Collect entities related to the given entity.
fn collect_related_entities(
    entity: &data::EntityRecord,
    cfg: &ResolvedConfig,
) -> Vec<html::RelatedSection> {
    let mut sections = Vec::new();

    match entity.kind {
        EntityKind::Customer => {
            // Projects linked to this customer
            if let Ok(projects) = data::collect_entities(EntityKind::Project, cfg) {
                let related: Vec<_> = projects
                    .into_iter()
                    .filter(|p| {
                        let customers = frontmatter::get_link_list(&p.frontmatter, "customers");
                        let customer = frontmatter::get_str_or(&p.frontmatter, "customer", "");
                        let customer = frontmatter::strip_wikilink(customer);
                        customers.iter().any(|c| c.eq_ignore_ascii_case(&entity.id))
                            || customer.eq_ignore_ascii_case(&entity.id)
                    })
                    .collect();
                if !related.is_empty() {
                    sections.push(html::RelatedSection {
                        title: "Projects".to_string(),
                        kind: EntityKind::Project,
                        entities: related,
                    });
                }
            }
            // Contacts for this customer
            let filter = ContactFilter {
                status: None,
                tag: None,
                customer: Some(&entity.id),
            };
            if let Ok(contacts) = data::collect_contacts_filtered(cfg, &filter) {
                if !contacts.is_empty() {
                    sections.push(html::RelatedSection {
                        title: "Contacts".to_string(),
                        kind: EntityKind::Contact,
                        entities: contacts,
                    });
                }
            }
            // Tasks for this customer
            let filter = TaskFilter {
                status: None,
                tag: None,
                project: None,
                customer: Some(&entity.id),
                priority: None,
                sprint: None,
                owner: None,
            };
            if let Ok(tasks) = data::collect_tasks_filtered(cfg, &filter) {
                if !tasks.is_empty() {
                    sections.push(html::RelatedSection {
                        title: "Tasks".to_string(),
                        kind: EntityKind::Task,
                        entities: tasks,
                    });
                }
            }
        }
        EntityKind::Project => {
            // Tasks linked to this project
            let filter = TaskFilter {
                status: None,
                tag: None,
                project: Some(&entity.id),
                customer: None,
                priority: None,
                sprint: None,
                owner: None,
            };
            if let Ok(tasks) = data::collect_tasks_filtered(cfg, &filter) {
                if !tasks.is_empty() {
                    sections.push(html::RelatedSection {
                        title: "Tasks".to_string(),
                        kind: EntityKind::Task,
                        entities: tasks,
                    });
                }
            }
            // Meetings linked to this project
            if let Ok(meetings) = data::collect_entities(EntityKind::Meeting, cfg) {
                let related: Vec<_> = meetings
                    .into_iter()
                    .filter(|m| {
                        let projects = frontmatter::get_link_list(&m.frontmatter, "projects");
                        let project = frontmatter::get_str_or(&m.frontmatter, "project", "");
                        let project = frontmatter::strip_wikilink(project);
                        projects.iter().any(|p| p.eq_ignore_ascii_case(&entity.id))
                            || project.eq_ignore_ascii_case(&entity.id)
                    })
                    .collect();
                if !related.is_empty() {
                    sections.push(html::RelatedSection {
                        title: "Meetings".to_string(),
                        kind: EntityKind::Meeting,
                        entities: related,
                    });
                }
            }
        }
        EntityKind::Sprint => {
            // Tasks in this sprint
            let filter = TaskFilter {
                status: None,
                tag: None,
                project: None,
                customer: None,
                priority: None,
                sprint: Some(&entity.id),
                owner: None,
            };
            if let Ok(tasks) = data::collect_tasks_filtered(cfg, &filter) {
                if !tasks.is_empty() {
                    sections.push(html::RelatedSection {
                        title: "Tasks".to_string(),
                        kind: EntityKind::Task,
                        entities: tasks,
                    });
                }
            }
        }
        _ => {}
    }

    sections
}

fn error_response(message: &str) -> (StatusCode, Html<String>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page(message)),
    )
}

async fn handle_404(State(state): State<Arc<AppState>>, uri: axum::http::Uri) -> Html<String> {
    Html(html::not_found_page(
        uri.path(),
        &state.cfg.brand,
        &state.custom_css,
    ))
}

async fn handle_brand_logo(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let logo_path = state.cfg.brand.logo.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let content = std::fs::read(logo_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let ext = logo_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let content_type = match ext {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    };
    Ok(([(header::CONTENT_TYPE, content_type)], content))
}

async fn handle_brand_fonts(
    State(state): State<Arc<AppState>>,
    Path(filename): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    // Path traversal protection
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return Err(StatusCode::BAD_REQUEST);
    }

    let fonts_dir = state
        .cfg
        .brand
        .fonts_dir
        .as_ref()
        .ok_or(StatusCode::NOT_FOUND)?;
    let file_path = fonts_dir.join(&filename);

    if !file_path.is_file() {
        return Err(StatusCode::NOT_FOUND);
    }

    let content = std::fs::read(&file_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let content_type = if filename.ends_with(".woff2") {
        "font/woff2"
    } else if filename.ends_with(".ttf") {
        "font/ttf"
    } else if filename.ends_with(".woff") {
        "font/woff"
    } else {
        return Err(StatusCode::BAD_REQUEST);
    };
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        content,
    ))
}
