//! REST API for MissionControl.
//!
//! Exposes the entity surface (CRUD where supported, plus task move,
//! validate, index) over HTTP/JSON. Mirrors the MCP tool surface so any
//! client that knows mc semantics can drive it without spawning a process
//! per request.
//!
//! # Concurrency
//!
//! - Reads run unsynchronised. `util::atomic_write` makes each individual
//!   file's write atomic, and reads tolerate seeing a half-built directory
//!   tree (e.g. a customer dir whose markdown file hasn't landed yet) for
//!   the few milliseconds before a concurrent write completes.
//! - Writes acquire `write_lock` (a `Mutex`). One writer at a time
//!   serializes ID allocation (scan-and-increment) and rules out
//!   read-modify-write races between concurrent writers.
//!
//! At the rates this API serves (humans + a small fleet of agents) a single
//! per-process mutex is correct and trivially auditable. If contention ever
//! shows up, the next step is sharding by entity kind, then by repo subtree.
//!
//! Cross-process safety is enforced by an exclusive `flock` on
//! `<repo>/.mc-api.lock` — a second `mc api serve` against the same repo
//! fails fast at startup instead of handing out duplicate IDs.
//!
//! # Auth
//!
//! Static bearer tokens loaded from a YAML file at startup. See
//! [`auth::TokenStore`] for the file format. The middleware injects
//! `auth::AuthedToken` into request extensions; handlers don't need to read
//! it directly today, but it shows up in the trace layer's request span.
//!
//! # Errors
//!
//! Every error response is `application/problem+json` (RFC 7807). See
//! [`error::ProblemJson`] and [`error::problem_from_mc_error`].

pub mod auth;
pub mod error;
pub mod handlers;
pub mod schemas;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::HeaderName;
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::sync::Mutex;
use tower::ServiceBuilder;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{DefaultOnRequest, TraceLayer};
use tracing::Level;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::api::auth::{AuthState, TokenStore};
use crate::config::ResolvedConfig;
use crate::error::McResult;

/// Shared state passed to every handler.
///
/// `write_lock` serializes all mutating operations across all entity kinds:
/// it is acquired around the full read-modify-write of a single request so
/// that ID allocation (a scan of the repo) cannot race with another writer.
/// Reads do not need a corresponding lock — `util::atomic_write` makes each
/// individual file's write atomic, and reads tolerate seeing a half-built
/// directory tree (e.g. a customer dir whose markdown file hasn't landed
/// yet) for the few milliseconds before the write completes.
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<ResolvedConfig>,
    pub write_lock: Arc<Mutex<()>>,
}

/// Configuration for booting the API server.
pub struct ApiServerConfig {
    pub bind: SocketAddr,
    pub tokens: TokenStore,
    pub read_only: bool,
}

/// Owned exclusive flock on the repo. Released when this struct is dropped
/// (i.e. when the server exits). A second `mc api serve` against the same
/// repo will fail to acquire it and exit with a clear error rather than
/// hand out duplicate IDs.
pub struct RepoLock {
    _file: std::fs::File,
}

impl RepoLock {
    pub fn acquire(repo_root: &std::path::Path) -> McResult<Self> {
        use fs2::FileExt;

        let lock_path = repo_root.join(".mc-api.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&lock_path)
            .map_err(crate::error::McError::Io)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(RepoLock { _file: file }),
            Err(_) => Err(crate::error::McError::Other(format!(
                "another `mc api serve` is already running against {} (lock file: {})",
                repo_root.display(),
                lock_path.display()
            ))),
        }
    }
}

/// Adds the `bearer` security scheme to the OpenAPI components so handler
/// `security(("bearer" = []))` annotations resolve.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some("Bearer token (argon2id-hashed in tokens.yml)"))
                    .build(),
            ),
        );
    }
}

/// OpenAPI document — populated by `utoipa::OpenApi` derive on the schemas
/// and handler annotations. Served at `/v1/openapi.json`.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "MissionControl REST API",
        version = "0.2.0",
        description = "HTTP/JSON surface for mc — list, create, and update tasks, customers, projects, meetings, research, sprints, proposals, and contacts.",
        license(name = "MIT")
    ),
    modifiers(&SecurityAddon),
    paths(
        handlers::health::healthz,
        handlers::health::readyz,
        handlers::meta::get_config,
        handlers::meta::get_status,
        handlers::entities::list_entities,
        handlers::entities::get_entity,
        handlers::entities::get_entity_raw,
        handlers::tasks::list_tasks,
        handlers::tasks::move_task,
        handlers::creates::create_customer,
        handlers::creates::create_project,
        handlers::creates::create_meeting,
        handlers::creates::create_research,
        handlers::creates::create_task,
        handlers::creates::create_sprint,
        handlers::creates::create_proposal,
        handlers::creates::create_contact,
        handlers::maintenance::rebuild_index,
        handlers::maintenance::run_validate,
    ),
    components(schemas(
        crate::api::error::ProblemJson,
        crate::api::error::FieldError,
        crate::api::schemas::CreateResult,
        crate::api::schemas::MoveTaskResult,
        crate::api::schemas::IndexResult,
        crate::api::schemas::ValidationReport,
        crate::api::schemas::ValidationIssue,
        crate::api::schemas::StatusResponse,
        crate::api::schemas::KindStatusCounts,
        crate::api::schemas::StatusCount,
        crate::api::schemas::RecentEntry,
        crate::api::schemas::ConfigResponse,
        crate::api::schemas::PrefixView,
        crate::api::schemas::StatusView,
        crate::api::schemas::EntityResponse,
        crate::api::schemas::CreateCustomer,
        crate::api::schemas::CreateProject,
        crate::api::schemas::CreateMeeting,
        crate::api::schemas::CreateResearch,
        crate::api::schemas::CreateTask,
        crate::api::schemas::CreateSprint,
        crate::api::schemas::CreateProposal,
        crate::api::schemas::CreateContact,
        crate::api::schemas::MoveTaskBody,
    )),
    tags(
        (name = "meta", description = "Repository metadata (config, status)"),
        (name = "entities", description = "Generic entity CRUD"),
        (name = "tasks", description = "Task list and status transitions"),
        (name = "maintenance", description = "Index rebuild and validation")
    )
)]
pub struct ApiDoc;

/// Build the axum router. Public so integration tests can drive it without
/// binding a TCP socket.
pub fn build_router(cfg: ResolvedConfig, server_cfg: &ApiServerConfig) -> Router {
    let state = AppState {
        cfg: Arc::new(cfg),
        write_lock: Arc::new(Mutex::new(())),
    };

    let auth_state = AuthState {
        store: server_cfg.tokens.clone(),
        read_only: server_cfg.read_only,
    };

    // Public routes — no auth.
    let public = Router::new()
        .route("/healthz", get(handlers::health::healthz))
        .route("/readyz", get(handlers::health::readyz))
        .route("/v1/openapi.json", get(serve_openapi))
        .route("/v1/docs", get(handlers::health::docs))
        .with_state(state.clone());

    // Authenticated routes.
    let v1 = Router::new()
        .route("/v1/config", get(handlers::meta::get_config))
        .route("/v1/status", get(handlers::meta::get_status))
        .route(
            "/v1/entities/{kind}",
            get(handlers::entities::list_entities),
        )
        .route(
            "/v1/entities/{kind}/{id}",
            get(handlers::entities::get_entity),
        )
        .route(
            "/v1/entities/{kind}/{id}/raw",
            get(handlers::entities::get_entity_raw),
        )
        // GET /v1/tasks lists with the full task filter set; POST creates.
        .route(
            "/v1/tasks",
            get(handlers::tasks::list_tasks).post(handlers::creates::create_task),
        )
        .route("/v1/tasks/{id}/move", post(handlers::tasks::move_task))
        // Create endpoints for the remaining kinds. We use plural-form paths
        // here so they do not collide with the generic GET /v1/entities/{kind}
        // route — listing stays on the generic prefix, creates on /v1/<plural>.
        .route("/v1/customers", post(handlers::creates::create_customer))
        .route("/v1/projects", post(handlers::creates::create_project))
        .route("/v1/meetings", post(handlers::creates::create_meeting))
        .route("/v1/research", post(handlers::creates::create_research))
        .route("/v1/sprints", post(handlers::creates::create_sprint))
        .route("/v1/proposals", post(handlers::creates::create_proposal))
        .route("/v1/contacts", post(handlers::creates::create_contact))
        .route("/v1/index", post(handlers::maintenance::rebuild_index))
        .route("/v1/validate", post(handlers::maintenance::run_validate))
        .layer(middleware::from_fn_with_state(
            auth_state.clone(),
            auth::require_auth,
        ))
        .with_state(state);

    let request_id_header = HeaderName::from_static("x-request-id");

    Router::new().merge(public).merge(v1).layer(
        ServiceBuilder::new()
            .layer(SetRequestIdLayer::new(
                request_id_header.clone(),
                MakeRequestUuid,
            ))
            .layer(PropagateRequestIdLayer::new(request_id_header))
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(make_span)
                    .on_request(DefaultOnRequest::new().level(Level::INFO))
                    .on_response(tower_http::trace::DefaultOnResponse::new().level(Level::INFO)),
            )
            // Cap request bodies at 64 KiB. Even the heaviest entity create
            // (a meeting with all the optional fields) is under 4 KiB; 64
            // KiB leaves plenty of headroom and rules out an OOM via a
            // multi-megabyte JSON body.
            .layer(DefaultBodyLimit::max(64 * 1024))
            // 30 s is generous for any real handler — argon2 verify is
            // ~50 ms cold, and the slowest write (POST /v1/customers)
            // creates a directory tree in milliseconds. A slowloris-style
            // client that fails to send the full body is dropped here
            // before it can hold the write mutex.
            .layer(TimeoutLayer::with_status_code(
                axum::http::StatusCode::REQUEST_TIMEOUT,
                Duration::from_secs(30),
            )),
    )
}

fn make_span(req: &Request) -> tracing::Span {
    tracing::info_span!(
        "http",
        method = %req.method(),
        path = %req.uri().path(),
        request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
    )
}

async fn serve_openapi() -> impl IntoResponse {
    Json(ApiDoc::openapi())
}

/// Bind a TCP listener and serve the API. Used by `mc api serve` after
/// the caller has acquired the repo lock — pass it in so the lifetime is
/// owned by the running server, not by `serve`'s stack frame, and the
/// caller can fail-fast (e.g. before generating a dev token) if the lock
/// is taken.
pub async fn serve_with_lock(
    cfg: ResolvedConfig,
    server_cfg: ApiServerConfig,
    repo_lock: RepoLock,
) -> McResult<()> {
    let bind = server_cfg.bind;
    let read_only = server_cfg.read_only;
    let app = build_router(cfg, &server_cfg);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(crate::error::McError::Io)?;
    print_startup_banner(bind, read_only);
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(crate::error::McError::Io);
    drop(repo_lock); // explicit — release the flock at shutdown.
    result
}

fn print_startup_banner(bind: SocketAddr, read_only: bool) {
    let base = format!("http://{}", bind);
    tracing::info!("mc api serve listening on {}", base);
    tracing::info!("  docs:    {}/v1/docs", base);
    tracing::info!("  spec:    {}/v1/openapi.json", base);
    tracing::info!("  health:  {}/healthz", base);
    if read_only {
        tracing::info!("  mode:    read-only (every non-GET will be rejected with 403)");
    }
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.expect("install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
