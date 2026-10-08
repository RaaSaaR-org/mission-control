//! JSON endpoints behind the dashboard's interactive features.
//!
//! - `GET  /api/palette`         pages, kinds and entities for the command palette
//! - `GET  /api/version`         a fingerprint of the repo's Markdown files, for live refresh
//! - `GET  /api/preview/{id}`    an entity's hover preview card, as an HTML fragment
//! - `GET  /api/tasks/{id}`      one task
//! - `POST /api/tasks`           create a task (same logic as `mc new task`)
//! - `POST /api/tasks/{id}/move` change a task's status (same logic as `mc task move`)
//! - `PATCH /api/tasks/{id}`     change title, status, priority, owner, sprint, due date, project or customer
//! - `POST /api/entities/{id}/checks`   tick or untick a checklist item
//! - `POST /api/entities/{id}/comments` comment on a task or meeting
//!
//! Write responses carry the updated task, freshly rendered HTML fragments
//! (board card, title block, field rail) so the page can update in place, and
//! the repo version before and after the write, so the page doesn't mistake
//! its own write for an outside change but still notices one that landed
//! just before it. Errors are `{"error": code, "message": text, "field"?}`.

use super::AppState;
use crate::checklist::{self, Target};
use crate::commands::{new as new_cmd, task as task_cmd};
use crate::comments;
use crate::config::ResolvedConfig;
use crate::data::{self, EntityRecord};
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::frontmatter;
use crate::html::{self, Catalog};
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use serde_yaml::Value;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

const MAX_TITLE: usize = 200;
const MAX_OWNER: usize = 80;
use crate::html::MAX_COMMENT;

// ── Errors ──────────────────────────────────────────────────────────────

pub(super) struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    field: Option<&'static str>,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            field: None,
        }
    }

    pub(super) fn forbidden(code: &'static str, message: &str) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }

    fn invalid(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field: Some(field),
            ..Self::new(StatusCode::BAD_REQUEST, "invalid", message)
        }
    }

    fn not_found(id: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("No task with ID {id}."),
        )
    }
}

impl From<McError> for ApiError {
    fn from(e: McError) -> Self {
        match e {
            McError::EntityNotFound(id) => Self::not_found(&id),
            McError::InvalidId(_) => {
                Self::new(StatusCode::BAD_REQUEST, "invalid_id", e.to_string())
            }
            McError::Usage { .. } => {
                let message = match e.hint() {
                    Some(hint) => format!("{e} {hint}"),
                    None => e.to_string(),
                };
                Self::new(StatusCode::BAD_REQUEST, "invalid", message)
            }
            McError::NotFound { .. } => {
                Self::new(StatusCode::NOT_FOUND, "not_found", e.to_string())
            }
            McError::Conflict { .. } => {
                let message = match e.hint() {
                    Some(hint) => format!("{e} {hint}"),
                    None => e.to_string(),
                };
                Self::new(StatusCode::CONFLICT, "conflict", message)
            }
            other => {
                eprintln!("serve: write failed: {other}");
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "write_failed",
                    other.to_string(),
                )
            }
        }
    }
}

impl From<JsonRejection> for ApiError {
    fn from(e: JsonRejection) -> Self {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            return Self::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "too_large",
                "That's too much text for one request.",
            );
        }
        Self::new(StatusCode::BAD_REQUEST, "bad_request", e.body_text())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({"error": self.code, "message": self.message});
        if let Some(f) = self.field {
            body["field"] = json!(f);
        }
        (self.status, no_store(), Json(body)).into_response()
    }
}

type ApiResult = Result<Response, ApiError>;

fn no_store() -> [(header::HeaderName, &'static str); 1] {
    [(header::CACHE_CONTROL, "no-store")]
}

fn ok(status: StatusCode, body: JsonValue) -> ApiResult {
    Ok((status, no_store(), Json(body)).into_response())
}

pub(super) async fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "No such endpoint.")
}

pub(super) async fn method_not_allowed() -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "This endpoint doesn't accept that method.",
    )
}

// ── Reads ───────────────────────────────────────────────────────────────

pub(super) async fn palette(State(state): State<Arc<AppState>>) -> Response {
    let catalog = state.cached_catalog();
    let body = html::palette_json(&state.page(&catalog));
    (
        no_store(),
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

pub(super) async fn version(State(state): State<Arc<AppState>>) -> ApiResult {
    ok(
        StatusCode::OK,
        json!({"version": content_version(&state.cfg), "editable": state.editable}),
    )
}

/// The hover preview card for any entity, as an HTML fragment (`404` with a
/// "not found" card for unknown IDs). Links in it carry the base path.
pub(super) async fn preview(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    // Cards are fetched on every hover: reuse the catalog until files change.
    let catalog = state.cached_catalog();
    let page = state.page(&catalog);
    let (status, card) = match html::preview_card(&page, &id) {
        Some(card) => (StatusCode::OK, card),
        None => (StatusCode::NOT_FOUND, html::preview_not_found(&id)),
    };
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        html::prefix_base_path(&card, &state.base_path),
    )
        .into_response()
}

pub(super) async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult {
    check_task_id(&state.cfg, &id)?;
    task_response(&state, &id, json!({}), StatusCode::OK)
}

/// A fingerprint of every Markdown file's path, size and modification time
/// in the entity directories. It changes whenever a file is added, removed
/// or edited, by mc or anything else.
pub(super) fn content_version(cfg: &ResolvedConfig) -> String {
    let mut dirs = vec![
        &cfg.customers_dir,
        &cfg.projects_dir,
        &cfg.meetings_dir,
        &cfg.research_dir,
        &cfg.tasks_dir,
        &cfg.sprints_dir,
        &cfg.milestones_dir,
        &cfg.proposals_dir,
    ];
    dirs.sort();
    dirs.dedup();
    let mut files: Vec<(std::path::PathBuf, u128, u64)> = Vec::new();
    for dir in dirs.into_iter().filter(|d| d.is_dir()) {
        let walker = WalkDir::new(dir)
            .into_iter()
            .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));
        for entry in walker.filter_map(Result::ok) {
            let path = entry.path();
            if !entry.file_type().is_file() || !data::is_markdown(path) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            files.push((path.to_path_buf(), mtime, meta.len()));
        }
    }
    files.sort();
    files.dedup();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    files.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

// ── Validation ──────────────────────────────────────────────────────────

/// IDs must look exactly like `TASK-012`; anything else (including path
/// separators or `..`) is rejected before touching the filesystem.
fn check_task_id(cfg: &ResolvedConfig, id: &str) -> Result<(), ApiError> {
    let prefix = &cfg.id_prefixes.task;
    let valid = id
        .strip_prefix(prefix.as_str())
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|n| !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()));
    if valid {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_id",
            format!("Task IDs look like {prefix}-001."),
        ))
    }
}

fn check_status(cfg: &ResolvedConfig, status: &str) -> Result<String, ApiError> {
    let valid = EntityKind::Task.statuses(cfg);
    if valid.iter().any(|s| s == status) {
        Ok(status.to_string())
    } else {
        Err(ApiError::invalid(
            "status",
            format!(
                "Unknown status “{status}”. Use one of: {}.",
                valid.join(", ")
            ),
        ))
    }
}

fn check_priority(p: u32) -> Result<u32, ApiError> {
    if html::PRIORITIES.contains(&p) {
        Ok(p)
    } else {
        Err(ApiError::invalid(
            "priority",
            "Priority must be 1 (critical) to 4 (low).",
        ))
    }
}

/// Trimmed single-line text of at most `max` characters.
fn check_text(field: &'static str, s: &str, max: usize) -> Result<String, ApiError> {
    let s = s.trim();
    if s.chars().any(char::is_control) {
        return Err(ApiError::invalid(field, "Use a single line of text."));
    }
    if s.chars().count() > max {
        return Err(ApiError::invalid(
            field,
            format!("Keep it under {max} characters."),
        ));
    }
    Ok(s.to_string())
}

/// Empty clears the date; anything else must be `YYYY-MM-DD`.
fn check_date(s: &str) -> Result<String, ApiError> {
    let s = s.trim();
    // Same strict check as `mc new` (no unpadded `2026-1-5`).
    if s.is_empty() || crate::commands::new::validate_date(s, "due date").is_ok() {
        Ok(s.to_string())
    } else {
        Err(ApiError::invalid("due_date", "Use a date like 2026-10-31."))
    }
}

/// Empty clears the reference; anything else must be an existing entity.
fn check_ref(
    catalog: &Catalog,
    cfg: &ResolvedConfig,
    kind: EntityKind,
    field: &'static str,
    id: &str,
) -> Result<String, ApiError> {
    let id = id.trim();
    if id.is_empty() {
        return Ok(String::new());
    }
    if !cfg.entity_available(&kind) {
        return Err(ApiError::invalid(
            field,
            format!("This repo has no {}.", kind.label_plural()),
        ));
    }
    if catalog.of_kind(kind).any(|r| r.id == id) {
        Ok(id.to_string())
    } else {
        Err(ApiError::invalid(
            field,
            format!("No {} with ID {id}.", kind.label()),
        ))
    }
}

fn find_task(cfg: &ResolvedConfig, id: &str) -> Result<EntityRecord, ApiError> {
    check_task_id(cfg, id)?;
    match data::find_entity_by_id(id, cfg) {
        Ok(rec) if rec.kind == EntityKind::Task => Ok(rec),
        Ok(_) | Err(McError::EntityNotFound(_)) => Err(ApiError::not_found(id)),
        Err(e) => Err(e.into()),
    }
}

/// Any entity by exact ID. The ID must look like `PREFIX-123` before the
/// filesystem is searched.
fn find_entity(cfg: &ResolvedConfig, id: &str) -> Result<EntityRecord, ApiError> {
    let well_formed = id.len() <= 40
        && id.contains('-')
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && EntityKind::from_id(id, cfg).is_ok();
    if !well_formed {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_id",
            "IDs look like TASK-001 or MTG-002.",
        ));
    }
    match data::find_entity_by_id(id, cfg) {
        Ok(rec) => Ok(rec),
        Err(McError::EntityNotFound(_)) => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("No entity with ID {id}."),
        )),
        Err(e) => Err(e.into()),
    }
}

// ── Responses ───────────────────────────────────────────────────────────

fn task_json(rec: &EntityRecord, cfg: &ResolvedConfig) -> JsonValue {
    let fm = &rec.frontmatter;
    let path = rec
        .source_path
        .strip_prefix(&cfg.root)
        .unwrap_or(&rec.source_path);
    json!({
        "id": rec.id,
        "title": html::display_name(rec),
        "status": frontmatter::get_str_or(fm, "status", ""),
        "priority": data::get_number(fm, "priority").unwrap_or(3),
        "owner": frontmatter::get_str_or(fm, "owner", ""),
        "sprint": frontmatter::get_link_str(fm, "sprint").unwrap_or(""),
        "milestone": frontmatter::get_link_str(fm, "milestone").unwrap_or(""),
        "due_date": frontmatter::get_str_or(fm, "due_date", ""),
        "projects": frontmatter::get_link_list(fm, "projects"),
        "customers": frontmatter::get_link_list(fm, "customers"),
        "path": path.display().to_string(),
        "frontmatter": data::yaml_to_json(fm),
    })
}

/// The task as JSON plus freshly rendered fragments, merged into `extra`.
fn task_response(
    state: &AppState,
    id: &str,
    mut extra: JsonValue,
    status: StatusCode,
) -> ApiResult {
    let cfg = &state.cfg;
    let catalog = Catalog::load(cfg);
    let page = state.page(&catalog);
    let found;
    let rec = match catalog
        .records
        .iter()
        .find(|r| r.id == id && r.kind == EntityKind::Task)
    {
        Some(r) => r,
        None => {
            found = find_task(cfg, id)?;
            &found
        }
    };
    let prefix = |h: String| html::prefix_base_path(&h, &state.base_path);
    let (title_block, rail) = html::detail_fragments(&page, rec);
    extra["task"] = task_json(rec, cfg);
    extra["html"] = json!({
        "card": prefix(html::task_card(&page, rec)),
        "title_block": prefix(title_block),
        "rail": prefix(rail),
    });
    extra["version"] = json!(content_version(cfg));
    ok(status, extra)
}

// ── Writes ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MoveBody {
    status: String,
}

pub(super) async fn move_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Result<Json<MoveBody>, JsonRejection>,
) -> ApiResult {
    let Json(body) = body?;
    let cfg = &state.cfg;
    let status = check_status(cfg, body.status.trim())?;
    let _write = state.write_lock.lock().await;
    let prev = content_version(cfg);
    let task = find_task(cfg, &id)?;
    let old = frontmatter::get_str_or(&task.frontmatter, "status", "").to_string();
    if old != status {
        move_status(cfg, &task, &status)?;
    }
    task_response(
        &state,
        &id,
        json!({"old_status": old, "prev_version": prev}),
        StatusCode::OK,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateBody {
    title: Option<String>,
    status: Option<String>,
    priority: Option<u32>,
    owner: Option<String>,
    sprint: Option<String>,
    milestone: Option<String>,
    due_date: Option<String>,
    /// One project ID, or empty for none.
    project: Option<String>,
    /// One customer ID, or empty for none.
    customer: Option<String>,
}

/// A new single project or customer link, or `None` if it's unchanged. A
/// task linking several is edited in its file, not replaced from here.
fn check_single_ref(
    catalog: &Catalog,
    cfg: &ResolvedConfig,
    fm: &Value,
    kind: EntityKind,
    field: &'static str,
    value: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(str::trim) else {
        return Ok(None);
    };
    let current = frontmatter::get_link_list(fm, kind.label_plural());
    let current: Vec<&str> = current
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if current.len() <= 1 && current.first().copied().unwrap_or("") == value {
        return Ok(None);
    }
    if current.len() > 1 {
        return Err(ApiError::invalid(
            field,
            format!(
                "This task links several {}. Edit them in its file.",
                kind.label_plural()
            ),
        ));
    }
    check_ref(catalog, cfg, kind, field, value).map(Some)
}

pub(super) async fn update_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Result<Json<UpdateBody>, JsonRejection>,
) -> ApiResult {
    let Json(body) = body?;
    let cfg = &state.cfg;
    if body.title.is_none()
        && body.status.is_none()
        && body.priority.is_none()
        && body.owner.is_none()
        && body.sprint.is_none()
        && body.milestone.is_none()
        && body.due_date.is_none()
        && body.project.is_none()
        && body.customer.is_none()
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "empty",
            "Nothing to change.",
        ));
    }
    // Checked here first so the form can point at the offending field;
    // `task::update_task` validates again and does the write.
    let status = body
        .status
        .as_deref()
        .map(|s| check_status(cfg, s.trim()))
        .transpose()?;

    let title = body
        .title
        .as_deref()
        .map(|t| check_text("title", t, MAX_TITLE))
        .transpose()?;
    if title.as_deref() == Some("") {
        return Err(ApiError::invalid("title", "Give the task a title."));
    }

    let _write = state.write_lock.lock().await;
    let prev = content_version(cfg);
    let task = find_task(cfg, &id)?;
    let fm = &task.frontmatter;
    let catalog = Catalog::load(cfg);
    let current_sprint = frontmatter::get_link_str(fm, "sprint").unwrap_or("").trim();
    // A one-element list, or an empty one to clear the link.
    let single = |kind, field, value: &Option<String>| {
        check_single_ref(&catalog, cfg, fm, kind, field, value.as_deref())
            .map(|id| id.map(|id| Some(id).filter(|i| !i.is_empty()).into_iter().collect()))
    };
    let changes = task_cmd::TaskUpdate {
        title,
        status,
        priority: body.priority.map(check_priority).transpose()?,
        owner: body
            .owner
            .as_deref()
            .map(|o| check_text("owner", o, MAX_OWNER))
            .transpose()?,
        // The form sends every field; an unchanged sprint may be a legacy
        // title, which is left as it is.
        sprint: match body.sprint.as_deref().map(str::trim) {
            None => None,
            Some(s) if s == current_sprint => None,
            Some(s) => Some(check_ref(&catalog, cfg, EntityKind::Sprint, "sprint", s)?),
        },
        milestone: body
            .milestone
            .as_deref()
            .map(|s| check_ref(&catalog, cfg, EntityKind::Milestone, "milestone", s))
            .transpose()?,
        due_date: body.due_date.as_deref().map(check_date).transpose()?,
        projects: single(EntityKind::Project, "project", &body.project)?,
        customers: single(EntityKind::Customer, "customer", &body.customer)?,
        ..Default::default()
    };
    let old = frontmatter::get_str_or(fm, "status", "").to_string();
    if !changes.is_empty() {
        task_cmd::update_task(cfg, &task.id, &changes)?;
    }
    task_response(
        &state,
        &id,
        json!({"old_status": old, "prev_version": prev}),
        StatusCode::OK,
    )
}

/// Change a task's status with the same logic as `mc task move`.
fn move_status(cfg: &ResolvedConfig, task: &EntityRecord, status: &str) -> McResult<()> {
    task_cmd::move_task_programmatic(cfg, &task.id, status, None).map(drop)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateBody {
    title: String,
    status: Option<String>,
    priority: Option<u32>,
    owner: Option<String>,
    project: Option<String>,
    customer: Option<String>,
    sprint: Option<String>,
    milestone: Option<String>,
    due_date: Option<String>,
}

pub(super) async fn create_task(
    State(state): State<Arc<AppState>>,
    body: Result<Json<CreateBody>, JsonRejection>,
) -> ApiResult {
    let Json(body) = body?;
    let cfg = &state.cfg;
    let title = check_text("title", &body.title, MAX_TITLE)?;
    if title.is_empty() {
        return Err(ApiError::invalid("title", "Give the task a title."));
    }
    let status = body
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| check_status(cfg, s))
        .transpose()?;
    let priority = body.priority.map(check_priority).transpose()?;
    let owner = check_text("owner", body.owner.as_deref().unwrap_or(""), MAX_OWNER)?;
    let due_date = check_date(body.due_date.as_deref().unwrap_or(""))?;

    let _write = state.write_lock.lock().await;
    let prev = content_version(cfg);
    let catalog = Catalog::load(cfg);
    let reference = |kind, field, value: &Option<String>| {
        check_ref(&catalog, cfg, kind, field, value.as_deref().unwrap_or(""))
    };
    let project = reference(EntityKind::Project, "project", &body.project)?;
    let customer = reference(EntityKind::Customer, "customer", &body.customer)?;
    let sprint = reference(EntityKind::Sprint, "sprint", &body.sprint)?;
    let some = |s: &str| (!s.is_empty()).then(|| s.to_string());

    let milestone = reference(EntityKind::Milestone, "milestone", &body.milestone)?;
    let created = new_cmd::create_task(
        cfg,
        &new_cmd::TaskInput {
            title,
            project: some(&project),
            customer: some(&customer),
            owner: some(&owner),
            status,
            priority,
            sprint: some(&sprint),
            milestone: some(&milestone),
            due_date: some(&due_date),
            ..Default::default()
        },
    )?
    .to_json();
    let id = created["id"].as_str().unwrap_or_default().to_string();
    task_response(
        &state,
        &id,
        json!({"href": format!("/entity/{id}"), "prev_version": prev}),
        StatusCode::CREATED,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckBody {
    /// 1-based line of the item in the file.
    line: usize,
    /// The new state; the item must currently be in the other one.
    checked: bool,
    /// The item's text as the page shows it, to detect edits on disk.
    text: Option<String>,
}

/// Tick or untick a checklist item. A stale line, text or state is a 409, so
/// an edit made elsewhere never gets the wrong item toggled.
pub(super) async fn check_item(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Result<Json<CheckBody>, JsonRejection>,
) -> ApiResult {
    let Json(body) = body?;
    let cfg = &state.cfg;
    let _write = state.write_lock.lock().await;
    let prev = content_version(cfg);
    let rec = find_entity(cfg, &id)?;
    let change = checklist::set_checked(
        &rec,
        Target::Line(body.line),
        body.checked,
        Some(!body.checked),
        body.text.as_deref(),
    )?;
    ok(
        StatusCode::OK,
        json!({
            "id": rec.id,
            "item": change.item,
            "changed": change.changed,
            "done": change.done,
            "total": change.total,
            "prev_version": prev,
            "version": content_version(cfg),
        }),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommentBody {
    text: String,
    /// Shown as the comment's author; defaults to the repo's git user.
    author: Option<String>,
}

/// Comment on a task or meeting. Responds with the comment and its rendered
/// list item.
pub(super) async fn add_comment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Result<Json<CommentBody>, JsonRejection>,
) -> ApiResult {
    let Json(body) = body.map_err(|e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError {
                field: Some("text"),
                ..ApiError::new(e.status(), "too_large", too_long_comment())
            }
        } else {
            e.into()
        }
    })?;
    let cfg = &state.cfg;
    if body.text.trim().is_empty() {
        return Err(ApiError::invalid("text", "Write something first."));
    }
    if body.text.chars().count() > MAX_COMMENT {
        return Err(ApiError::invalid("text", too_long_comment()));
    }
    let author = body
        .author
        .as_deref()
        .map(|a| check_text("author", a, MAX_OWNER))
        .transpose()?
        .filter(|a| !a.is_empty());

    let _write = state.write_lock.lock().await;
    let prev = content_version(cfg);
    let rec = find_entity(cfg, &id)?;
    if !comments::is_commentable(rec.kind) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid",
            "Only tasks and meetings take comments.",
        ));
    }
    let added = comments::add(cfg, &rec, &body.text, author.as_deref())?;
    let catalog = Catalog::load(cfg);
    let page = state.page(&catalog);
    let item = html::comment_html(&page, &rec, &added.comment, added.count);
    ok(
        StatusCode::CREATED,
        json!({
            "id": rec.id,
            "comment": added.comment,
            "count": added.count,
            "html": html::prefix_base_path(&item, &state.base_path),
            "prev_version": prev,
            "version": content_version(cfg),
        }),
    )
}

fn too_long_comment() -> String {
    format!("Keep comments under {MAX_COMMENT} characters.")
}
