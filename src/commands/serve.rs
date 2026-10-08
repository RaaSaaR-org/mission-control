//! `mc serve`: the web dashboard.
//!
//! Pages are server-rendered HTML. The JSON endpoints under `/api/` back the
//! interactive features (command palette, live refresh, task edits,
//! checklists, comments); see
//! `serve/api.rs`. Every request passes `serve/guard.rs` (Host check,
//! security headers, write protection).

mod api;
mod guard;

use crate::config::{RepoMode, ResolvedConfig};
use crate::data::{self, TaskFilter};
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::html::{self, Catalog, ListQuery, Page, TaskQuery};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{middleware, Router};
use std::collections::HashMap;
use std::sync::Arc;

/// Largest JSON body the write endpoints accept.
const MAX_BODY_BYTES: usize = 32 * 1024;
/// Largest comment body: [`html::MAX_COMMENT`] characters of up to four
/// UTF-8 bytes each, plus JSON escaping and the author.
const MAX_COMMENT_BODY_BYTES: usize = 128 * 1024;

/// How the dashboard is served.
#[derive(Debug, Clone, Default)]
pub struct ServeOptions {
    /// Path prefix when served behind a reverse proxy (e.g. `/hq`).
    pub base_path: String,
    /// Never accept edits.
    pub read_only: bool,
    /// Accept edits even behind a reverse proxy (`base_path` set).
    pub allow_edits: bool,
}

impl ServeOptions {
    /// Edits are on for local use and off behind a proxy unless allowed.
    pub fn editable(&self) -> bool {
        !self.read_only && (self.base_path.trim_end_matches('/').is_empty() || self.allow_edits)
    }
}

struct AppState {
    cfg: ResolvedConfig,
    /// Cached custom CSS content (read once at startup, relative URLs rewritten).
    custom_css: String,
    /// Base path prefix for reverse proxy deployments (e.g. "/hq").
    base_path: String,
    /// Whether write endpoints and the edit UI are enabled.
    editable: bool,
    /// Serialises writes so ID allocation and file moves don't race.
    write_lock: tokio::sync::Mutex<()>,
    /// The catalog for hover previews and the palette, with the repo version
    /// it was loaded at.
    catalog_cache: std::sync::Mutex<Option<(String, Arc<Catalog>)>>,
}

impl AppState {
    fn page<'a>(&'a self, catalog: &'a Catalog) -> Page<'a> {
        Page::new(&self.cfg, catalog, &self.custom_css).with_editable(self.editable)
    }

    /// Render a page with a freshly loaded catalog and apply the base path.
    fn render(&self, f: impl FnOnce(&Page) -> String) -> Html<String> {
        self.render_with(&Catalog::load(&self.cfg), f)
    }

    /// Render a page with an already loaded catalog and apply the base path.
    fn render_with(&self, catalog: &Catalog, f: impl FnOnce(&Page) -> String) -> Html<String> {
        Html(html::prefix_base_path(
            &f(&self.page(catalog)),
            &self.base_path,
        ))
    }

    /// The catalog, reloaded only when a Markdown file changed since the last
    /// call (checked with a cheap metadata walk). For frequent reads such as
    /// hover previews; pages load their own.
    fn cached_catalog(&self) -> Arc<Catalog> {
        let version = api::content_version(&self.cfg);
        let lock = || self.catalog_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((v, catalog)) = lock().as_ref() {
            if *v == version {
                return catalog.clone();
            }
        }
        let catalog = Arc::new(Catalog::load(&self.cfg));
        *lock() = Some((version, catalog.clone()));
        catalog
    }
}

type Params = Query<HashMap<String, String>>;

pub fn run(cfg: &ResolvedConfig, port: u16, opts: &ServeOptions) -> McResult<()> {
    let base_path = opts.base_path.trim_end_matches('/').to_string();
    let app = router(cfg, opts);

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let addr = format!("127.0.0.1:{}", port);
        println!("MissionControl web dashboard: http://{}{}", addr, base_path);
        if opts.editable() {
            println!("Editing is on: tasks can be moved, edited and created from the browser.");
        } else if opts.read_only {
            println!("Read-only: editing is off.");
        } else {
            println!("Read-only behind --base-path. Pass --allow-edits to enable editing.");
        }
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

/// Build the dashboard router with default options for `base_path`: editable
/// locally, read-only behind a proxy.
pub fn build_router(cfg: &ResolvedConfig, base_path: &str) -> Router {
    router(
        cfg,
        &ServeOptions {
            base_path: base_path.to_string(),
            ..Default::default()
        },
    )
}

/// Build the dashboard router, nested under `opts.base_path` when it's
/// non-empty (e.g. `/hq` behind a reverse proxy).
pub fn router(cfg: &ResolvedConfig, opts: &ServeOptions) -> Router {
    // Read custom CSS once at startup. Relative url() references are rewritten
    // to /brand/asset/ so they resolve against the stylesheet's directory.
    let custom_css = cfg
        .brand
        .custom_css
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|css| html::rewrite_css_urls(&css))
        .unwrap_or_default();

    // Normalize base_path: strip trailing slash, keep leading slash
    let base_path = opts.base_path.trim_end_matches('/').to_string();

    let state = Arc::new(AppState {
        cfg: cfg.clone(),
        custom_css,
        base_path: base_path.clone(),
        editable: opts.editable(),
        write_lock: tokio::sync::Mutex::new(()),
        catalog_cache: std::sync::Mutex::new(None),
    });

    let api = Router::new()
        .route("/palette", get(api::palette))
        .route("/version", get(api::version))
        .route("/preview/{id}", get(api::preview))
        .route("/tasks", post(api::create_task))
        .route("/tasks/{id}", get(api::get_task).patch(api::update_task))
        .route("/tasks/{id}/move", post(api::move_task))
        .route("/entities/{id}/checks", post(api::check_item))
        .route(
            "/entities/{id}/comments",
            post(api::add_comment).layer(DefaultBodyLimit::max(MAX_COMMENT_BODY_BYTES)),
        )
        .fallback(api::not_found)
        .method_not_allowed_fallback(api::method_not_allowed)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES));

    let list =
        |kind: EntityKind| get(move |s: State<Arc<AppState>>, q: Params| handle_list(kind, s, q));
    let routes = Router::new()
        .route("/", get(handle_dashboard))
        .route("/customers", list(EntityKind::Customer))
        .route("/projects", list(EntityKind::Project))
        .route("/meetings", list(EntityKind::Meeting))
        .route("/meetings/calendar", get(handle_calendar))
        .route("/research", list(EntityKind::Research))
        .route("/sprints", list(EntityKind::Sprint))
        .route("/milestones", get(handle_milestones))
        .route("/proposals", list(EntityKind::Proposal))
        .route("/contacts", list(EntityKind::Contact))
        .route("/tasks", get(handle_tasks_board))
        .route("/tasks/list", get(handle_tasks))
        .route("/entity/{id}", get(handle_detail))
        .route("/files/{*path}", get(handle_file))
        .route("/search", get(handle_search))
        .route("/index.json", get(handle_index_json))
        .route("/assets/archivo.woff2", get(handle_font))
        .route("/assets/app.css", get(handle_app_css))
        .route("/assets/app.js", get(handle_app_js))
        .route("/brand/logo", get(handle_brand_logo))
        .route("/brand/fonts/{filename}", get(handle_brand_fonts))
        .route("/brand/asset/{*path}", get(handle_brand_asset))
        .nest("/api", api)
        .fallback(handle_404)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            guard::protect,
        ))
        .with_state(state);

    if base_path.is_empty() {
        return routes;
    }
    // Axum nest doesn't match trailing slash on the base path itself.
    // Add explicit redirect: /base/path/ -> /base/path
    let bp = base_path.clone();
    Router::new().nest(&base_path, routes).route(
        &format!("{}/", base_path),
        get(move || async move { axum::response::Redirect::permanent(&bp) }),
    )
}

/// Get a non-empty query parameter.
fn param<'a>(params: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    params.get(key).map(|s| s.trim()).filter(|s| !s.is_empty())
}

async fn handle_dashboard(State(state): State<Arc<AppState>>) -> Html<String> {
    let recent = data::recent_activity(&state.cfg, 12).unwrap_or_else(|e| {
        eprintln!("serve: error loading recent activity: {}", e);
        Vec::new()
    });
    state.render(|page| html::dashboard_page(page, &recent))
}

/// The 404 page for a kind the repo doesn't enable, saying how to enable it.
fn not_available(state: &AppState, kind: EntityKind, path: &str) -> (StatusCode, Html<String>) {
    let err = McError::not_available(kind, &state.cfg);
    let hint = err.hint().unwrap_or_default();
    (
        StatusCode::NOT_FOUND,
        state.render(|page| html::not_available_page(page, path, &err.to_string(), &hint)),
    )
}

async fn handle_list(
    kind: EntityKind,
    State(state): State<Arc<AppState>>,
    Query(params): Params,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let cfg = &state.cfg;
    if !cfg.entity_available(&kind) {
        return Err(not_available(
            &state,
            kind,
            &format!("/{}", kind.label_plural()),
        ));
    }

    let status = param(&params, "status");
    let tag = param(&params, "tag");
    // Meetings are most useful newest first.
    let (sort, dir) = match (param(&params, "sort"), kind) {
        (Some(s), _) => (Some(s), param(&params, "dir").unwrap_or("asc")),
        (None, EntityKind::Meeting) => (Some("date"), "desc"),
        (None, _) => (None, "asc"),
    };

    // One catalog per request: the list is filtered from it in memory.
    let catalog = Catalog::load(cfg);
    let filter = TaskFilter {
        status,
        tag,
        ..TaskFilter::all()
    };
    let mut entities: Vec<data::EntityRecord> = catalog
        .of_kind(kind)
        .filter(|e| filter.matches(&e.frontmatter))
        .cloned()
        .collect();
    if let Some(field) = sort {
        html::sort_entities(&mut entities, field, dir);
    }

    let query = ListQuery {
        status,
        tag,
        sort,
        dir,
    };
    Ok(state.render_with(&catalog, |page| {
        html::list_page(page, kind, &entities, &query)
    }))
}

/// The meeting calendar for `?month=YYYY-MM` (this month by default);
/// `?overlays=1` adds sprints and task deadlines.
async fn handle_calendar(
    State(state): State<Arc<AppState>>,
    Query(params): Params,
) -> (StatusCode, Html<String>) {
    if !state.cfg.entity_available(&EntityKind::Meeting) {
        return not_available(&state, EntityKind::Meeting, "/meetings/calendar");
    }
    let query = html::CalendarQuery {
        month: param(&params, "month"),
        overlays: matches!(param(&params, "overlays"), Some("1" | "on" | "true")),
    };
    (
        StatusCode::OK,
        state.render(|page| html::calendar_page(page, &query)),
    )
}

fn task_query(params: &HashMap<String, String>) -> TaskQuery<'_> {
    TaskQuery {
        status: param(params, "status"),
        priority: param(params, "priority").and_then(|s| s.parse().ok()),
        owner: param(params, "owner"),
        project: param(params, "project"),
        customer: param(params, "customer"),
        sprint: param(params, "sprint"),
        milestone: param(params, "milestone"),
        sort: param(params, "sort"),
        dir: param(params, "dir").unwrap_or("asc"),
    }
}

/// The tasks matching `q`, and the filter options, from one catalog.
fn load_tasks(
    catalog: &Catalog,
    q: &TaskQuery,
) -> (Vec<data::EntityRecord>, html::TaskFilterOptions) {
    let filter = TaskFilter {
        status: q.status,
        tag: None,
        project: q.project,
        customer: q.customer,
        priority: q.priority,
        sprint: q.sprint,
        owner: q.owner,

        milestone: q.milestone,
    };
    let tasks = catalog
        .of_kind(EntityKind::Task)
        .filter(|t| filter.matches(&t.frontmatter))
        .cloned()
        .collect();
    // Dropdown options come from all tasks so filters can be switched freely.
    let options = html::TaskFilterOptions::from_records(catalog.of_kind(EntityKind::Task));
    (tasks, options)
}

async fn handle_tasks(State(state): State<Arc<AppState>>, Query(params): Params) -> Html<String> {
    let query = task_query(&params);
    let catalog = Catalog::load(&state.cfg);
    let (mut tasks, options) = load_tasks(&catalog, &query);
    if let Some(field) = query.sort {
        html::sort_entities(&mut tasks, field, query.dir);
    }
    state.render_with(&catalog, |page| {
        html::tasks_list_page(page, &tasks, &query, &options)
    })
}

async fn handle_tasks_board(
    State(state): State<Arc<AppState>>,
    Query(params): Params,
) -> Html<String> {
    // The board groups by status and orders lanes itself, so status,
    // priority and sort parameters don't apply.
    let query = TaskQuery {
        status: None,
        priority: None,
        sort: None,
        ..task_query(&params)
    };
    let catalog = Catalog::load(&state.cfg);
    let (tasks, options) = load_tasks(&catalog, &query);
    state.render_with(&catalog, |page| {
        html::board_page(page, &tasks, &query, &options)
    })
}

async fn handle_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> axum::response::Response {
    let catalog = Catalog::load(&state.cfg);
    let page = state.page(&catalog);
    let html = match catalog.records.iter().find(|r| r.id == id) {
        Some(entity) => html::detail_page(&page, entity),
        None => match data::find_entity_by_id(&id, &state.cfg) {
            Ok(entity) => html::detail_page(&page, &entity),
            Err(_) => {
                // IDs typed the way the CLI takes them (`task-37`) go to
                // the entity they mean.
                let loose = crate::cli::suggest::normalize_id(&id, &state.cfg, None)
                    .ok()
                    .map(|(canonical, _)| canonical)
                    .filter(|canonical| *canonical != id && catalog.name(canonical).is_some());
                if let Some(canonical) = loose {
                    let href = format!("{}/entity/{}", state.base_path, canonical);
                    return axum::response::Redirect::permanent(&href).into_response();
                }
                let page_html = html::not_found_page(&page, &format!("/entity/{id}"));
                return (
                    StatusCode::NOT_FOUND,
                    Html(html::prefix_base_path(&page_html, &state.base_path)),
                )
                    .into_response();
            }
        },
    };
    Html(html::prefix_base_path(&html, &state.base_path)).into_response()
}

/// Repo files linked from notes: Markdown is rendered as a page, images and
/// PDFs are served as they are. Hidden paths, other file types and anything
/// outside the repo are not served.
async fn handle_file(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> axum::response::Response {
    let cfg = &state.cfg;
    let not_found = || {
        (
            StatusCode::NOT_FOUND,
            state.render(|page| html::not_found_page(page, &format!("/files/{path}"))),
        )
            .into_response()
    };
    let rel = std::path::Path::new(&path);
    let visible = |rel: &std::path::Path| {
        rel.components().enumerate().all(|(i, c)| match c {
            std::path::Component::Normal(s) => {
                let s = s.to_string_lossy();
                !s.starts_with('.') || (i == 0 && s == ".mc" && cfg.mode == RepoMode::Embedded)
            }
            _ => false,
        })
    };
    let file = cfg.root.join(rel);
    let inside = cfg.root.canonicalize().ok().zip(file.canonicalize().ok());
    // The resolved path must be visible too: a symlink must not lead into
    // .git/ or another hidden folder.
    let resolved_ok = |(root, f): (std::path::PathBuf, std::path::PathBuf)| {
        f.is_file() && f.strip_prefix(&root).is_ok_and(visible)
    };
    if !visible(rel) || !inside.is_some_and(resolved_ok) {
        return not_found();
    }
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "md" {
        let Ok(content) = std::fs::read_to_string(&file) else {
            return not_found();
        };
        let catalog = Catalog::load(cfg);
        if let Some(id) = catalog.id_for_path(&file) {
            let href = format!("{}/entity/{}", state.base_path, id);
            return axum::response::Redirect::to(&href).into_response();
        }
        let page = state.page(&catalog);
        let html = html::file_page(&page, &file, &content);
        return Html(html::prefix_base_path(&html, &state.base_path)).into_response();
    }
    let content_type = match ext.as_str() {
        "pdf" => "application/pdf",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => {
            content_type_for(&ext).unwrap_or("application/octet-stream")
        }
        _ => return not_found(),
    };
    match std::fs::read(&file) {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, "no-cache"),
                // Files are shown, never run: no scripts even in SVGs.
                (
                    header::CONTENT_SECURITY_POLICY,
                    "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; sandbox",
                ),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => not_found(),
    }
}

async fn handle_search(State(state): State<Arc<AppState>>, Query(params): Params) -> Html<String> {
    let q = params.get("q").map(String::as_str).unwrap_or("");
    state.render(|page| html::search_page(page, q))
}

/// Every entity's ID, title, kind, status and date, for client-side jump-to.
async fn handle_index_json(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let catalog = state.cached_catalog();
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        html::index_json(&catalog),
    )
}

async fn handle_font() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "font/woff2"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        html::ARCHIVO_WOFF2,
    )
}

/// The bundled stylesheet and script. Pages link them with a content hash
/// (`?v=…`), so they can be cached for good.
async fn handle_app_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        html::APP_CSS,
    )
}

async fn handle_app_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        html::APP_JS,
    )
}

async fn handle_404(
    State(state): State<Arc<AppState>>,
    uri: axum::http::Uri,
) -> (StatusCode, Html<String>) {
    (
        StatusCode::NOT_FOUND,
        state.render(|page| html::not_found_page(page, uri.path())),
    )
}

async fn handle_brand_logo(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let logo_path = state.cfg.brand.logo.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let content = std::fs::read(logo_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let ext = logo_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let content_type = content_type_for(ext).unwrap_or("application/octet-stream");
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

    let ext = filename.rsplit('.').next().unwrap_or("");
    let content_type = match ext {
        "woff2" | "woff" | "ttf" | "otf" => content_type_for(ext),
        _ => None,
    }
    .ok_or(StatusCode::BAD_REQUEST)?;
    let content = std::fs::read(&file_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        content,
    ))
}

/// Serve files referenced by the custom stylesheet (fonts, images), relative
/// to the stylesheet's directory.
async fn handle_brand_asset(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let css_path = state
        .cfg
        .brand
        .custom_css
        .as_ref()
        .ok_or(StatusCode::NOT_FOUND)?;
    let base = css_path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .ok_or(StatusCode::NOT_FOUND)?;
    let file_path = base
        .join(&path)
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;
    // Path traversal protection: the resolved file must stay inside the CSS directory.
    if !file_path.starts_with(&base) || !file_path.is_file() {
        return Err(StatusCode::NOT_FOUND);
    }
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let content_type = content_type_for(&ext).ok_or(StatusCode::NOT_FOUND)?;
    let content = std::fs::read(&file_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        content,
    ))
}

/// Content type for static brand files. Returns `None` for unsupported types.
fn content_type_for(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "css" => "text/css",
        _ => return None,
    })
}

async fn handle_milestones(
    State(state): State<Arc<AppState>>,
    Query(params): Params,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    if !state.cfg.entity_available(&EntityKind::Milestone) {
        return Err(not_available(&state, EntityKind::Milestone, "/milestones"));
    }
    Ok(state.render(|page| html::milestones_page(page, param(&params, "project"))))
}
