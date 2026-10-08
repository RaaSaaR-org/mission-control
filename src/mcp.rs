//! Model Context Protocol server: exposes the mc CLI surface as MCP tools
//! and read-only resources for AI assistants.
//!
//! Descriptions are written for agents: they state valid values, defaults
//! and return shapes. Status values are configurable per repo, so
//! descriptions point at the `mc://config` resource instead of hard-coding
//! them.

use crate::checklist;
use crate::cli::suggest;
use crate::commands;
use crate::commands::new as new_cmd;
use crate::comments;
use crate::config::{RepoMode, ResolvedConfig};
use crate::data;
use crate::entity::EntityKind;
use crate::error::McError;
use crate::frontmatter;
use crate::util::parse_comma_list;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    Annotated, CallToolResult, Content, ListResourcesResult, PaginatedRequestParams, RawResource,
    ReadResourceRequestParams, ReadResourceResult, ResourceContents, ServerCapabilities,
    ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::path::Path;

/// Fields that may contain wiki-link brackets and should be stripped for JSON output.
const WIKILINK_FIELDS: &[&str] = &[
    "customers",
    "projects",
    "depends_on",
    "sprint",
    "milestone",
    "supersedes",
    "superseded_by",
    "customer",
];

/// Strip wiki-link brackets from known cross-reference fields in a JSON object.
fn strip_wikilinks_in_json(val: &mut JsonValue) {
    if let Some(obj) = val.as_object_mut() {
        for &field in WIKILINK_FIELDS {
            if let Some(v) = obj.get_mut(field) {
                match v {
                    JsonValue::String(s) => {
                        *s = frontmatter::strip_wikilink(s).to_string();
                    }
                    JsonValue::Array(arr) => {
                        for item in arr.iter_mut() {
                            if let JsonValue::String(s) = item {
                                *s = frontmatter::strip_wikilink(s).to_string();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared with the REST API (`api/handlers`), so both surfaces return the
// same entity shapes and resolve IDs and statuses the same way.

/// `path` relative to the repo root, as all JSON output shows paths; absolute
/// only when it lies outside the repo.
pub(crate) fn repo_relative(cfg: &ResolvedConfig, path: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(&cfg.root) {
        return rel.display().to_string();
    }
    // Paths that went through canonicalize (e.g. /private/tmp on macOS).
    if let Some(rel) = cfg
        .root
        .canonicalize()
        .ok()
        .and_then(|root| path.strip_prefix(root).ok().map(Path::to_path_buf))
    {
        return rel.display().to_string();
    }
    path.display().to_string()
}

/// Make the string field `key` of a JSON object repo-relative.
pub(crate) fn relativize_field(cfg: &ResolvedConfig, val: &mut JsonValue, key: &str) {
    if let Some(JsonValue::String(p)) = val.get_mut(key) {
        *p = repo_relative(cfg, Path::new(p.as_str()));
    }
}

/// An entity as JSON: its frontmatter with wiki-link brackets stripped, plus
/// `_kind` and `_source` (the repo-relative file path).
pub(crate) fn entity_json(rec: &data::EntityRecord, cfg: &ResolvedConfig) -> JsonValue {
    let mut val = data::yaml_to_json(&rec.frontmatter);
    strip_wikilinks_in_json(&mut val);
    if let Some(obj) = val.as_object_mut() {
        obj.insert("_kind".into(), JsonValue::String(rec.kind.label().into()));
        obj.insert(
            "_source".into(),
            JsonValue::String(repo_relative(cfg, &rec.source_path)),
        );
    }
    val
}

/// Find an entity from loose input (`task-7`, `TASK-0007` → `TASK-007`), as
/// the CLI does. With `kind`, a bare number is read as that kind and an ID
/// of another kind is not found. Malformed input is a usage error with a
/// suggestion; a well-formed but unknown ID is `EntityNotFound`.
pub(crate) fn resolve_entity(
    cfg: &ResolvedConfig,
    input: &str,
    kind: Option<EntityKind>,
) -> Result<data::EntityRecord, McError> {
    let raw = input.trim();
    let (id, id_kind) = suggest::normalize_id(raw, cfg, kind)?;
    if let Some(k) = kind.filter(|k| *k != id_kind) {
        return Err(McError::EntityNotFound(format!(
            "{id} (not a {})",
            k.label()
        )));
    }
    if !cfg.entity_available(&id_kind) {
        return Err(McError::not_available(id_kind, cfg));
    }
    match data::find_entity_by_id(&id, cfg) {
        // Hand-written IDs may use other padding (TASK-0001); try verbatim.
        Err(McError::EntityNotFound(_)) if raw != id => {
            data::find_entity_by_id(raw, cfg).map_err(|_| McError::EntityNotFound(id))
        }
        other => other,
    }
}

/// The existing entity whose ID is closest to the unknown `id`, as
/// `"TASK-001 (Title)"`.
pub(crate) fn closest_id(cfg: &ResolvedConfig, id: &str) -> Option<String> {
    let kind = EntityKind::from_id(id, cfg).ok()?;
    let existing = data::collect_entities(kind, cfg).ok()?;
    existing
        .iter()
        .map(|e| (suggest::levenshtein(id, &e.id), e))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, e)| (*d, e.id.clone()))
        .map(|(_, e)| {
            let name = frontmatter::get_str(&e.frontmatter, "title")
                .or_else(|| frontmatter::get_str(&e.frontmatter, "name"))
                .unwrap_or("");
            format!("{} ({name})", e.id)
        })
}

/// Canonical form of a loose ID reference (`proj-1` → `PROJ-001`), or the
/// input unchanged when it doesn't parse as one of `kind`.
pub(crate) fn loose_ref(cfg: &ResolvedConfig, input: &str, kind: EntityKind) -> String {
    match suggest::normalize_id(input, cfg, Some(kind)) {
        Ok((id, k)) if k == kind => id,
        _ => input.to_string(),
    }
}

/// A status filter in the configured spelling (`In_Progress`, `wip` →
/// `in-progress`); unknown values pass through and simply match nothing.
pub(crate) fn status_filter(cfg: &ResolvedConfig, kind: EntityKind, input: &str) -> String {
    suggest::match_status(input, kind.statuses(cfg))
        .unwrap_or(input)
        .to_string()
}

/// The actionable task queue as `{tasks, actionable, blocked}`, like `mc
/// task next`: at most `limit` tasks (default 5). Project and customer may
/// be loose IDs. Shared by the MCP `next_tasks` tool and `GET /v1/tasks/next`.
pub(crate) fn next_tasks_json(
    cfg: &ResolvedConfig,
    project: Option<&str>,
    customer: Option<&str>,
    owner: Option<&str>,
    limit: Option<usize>,
) -> Result<JsonValue, McError> {
    let project = project.map(|v| loose_ref(cfg, v, EntityKind::Project));
    let customer = customer.map(|v| loose_ref(cfg, v, EntityKind::Customer));
    let filter = data::TaskFilter {
        status: None,
        tag: None,
        project: project.as_deref(),
        customer: customer.as_deref(),
        priority: None,
        sprint: None,
        owner,

        milestone: None,
    };
    let (queue, blocked) = commands::task::actionable(cfg, &filter)?;
    let tasks: Vec<JsonValue> = queue
        .iter()
        .take(limit.unwrap_or(5).max(1))
        .map(|t| entity_json(t, cfg))
        .collect();
    Ok(serde_json::json!({
        "tasks": tasks,
        "actionable": queue.len(),
        "blocked": blocked,
    }))
}

/// Helper: convert any error into an internal rmcp error.
fn mc_err(e: impl std::fmt::Display) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

/// Map an `McError` to an MCP error: caller mistakes (bad IDs, unknown
/// entities, invalid values) become `invalid_params` so agents can correct
/// the call, a missing non-entity resource (`NotFound`) becomes
/// `resource_not_found`, and everything else is an internal error. The hint,
/// if any, is appended so the agent sees the same guidance as a CLI user.
fn tool_err(e: McError) -> McpError {
    enum Class {
        Params,
        NotFound,
        Internal,
    }
    let class = match &e {
        McError::InvalidId(_)
        | McError::EntityNotFound(_)
        | McError::NotAvailableInMode { .. }
        | McError::Usage { .. }
        | McError::Conflict { .. }
        | McError::ValidationFailed(_) => Class::Params,
        McError::NotFound { .. } => Class::NotFound,
        // Fallback for user errors still signalled via McError::Other.
        McError::Other(msg) => {
            let lower = msg.to_ascii_lowercase();
            if lower.starts_with("invalid ")
                || lower.starts_with("unknown ")
                || lower.contains("cannot be empty")
                || lower.contains("not found")
                || lower.contains(", not a")
            {
                Class::Params
            } else {
                Class::Internal
            }
        }
        _ => Class::Internal,
    };
    // The generic hint names a CLI command; agents should use the tools.
    let hint = match &e {
        McError::EntityNotFound(_) => {
            Some("use list_entities or list_tasks to see valid IDs".to_string())
        }
        _ => e.hint(),
    };
    let message = match hint {
        Some(hint) => format!("{e} ({hint})"),
        None => e.to_string(),
    };
    match class {
        Class::Params => McpError::invalid_params(message, None),
        Class::NotFound => McpError::resource_not_found(message, None),
        Class::Internal => McpError::internal_error(message, None),
    }
}

/// Serialize a value as pretty JSON tool output.
fn json_result(value: &impl serde::Serialize) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(value).map_err(mc_err)?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

// ---------------------------------------------------------------------------
// Parameter structs

/// Comma-separated input to a list (`None` stays empty).
fn csv(v: Option<String>) -> Vec<String> {
    v.as_deref().map(parse_comma_list).unwrap_or_default()
}

/// The kinds usable in this repo, in display order.
fn available_kinds(cfg: &ResolvedConfig) -> Vec<EntityKind> {
    EntityKind::ALL
        .into_iter()
        .filter(|k| cfg.entity_available(k))
        .collect()
}

fn require_available(kind: EntityKind, cfg: &ResolvedConfig) -> Result<(), McpError> {
    if cfg.entity_available(&kind) {
        Ok(())
    } else {
        Err(tool_err(McError::not_available(kind, cfg)))
    }
}

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListEntitiesParams {
    #[schemars(
        description = "Entity kind: customers, contacts, projects, meetings, research, tasks, milestones, sprints, or proposals (singular forms also accepted). Kinds not enabled in this repo are rejected; read mc://config for the list."
    )]
    pub kind: String,
    #[schemars(
        description = "Filter by status, case-insensitive (valid values per kind are in mc://config under statuses)"
    )]
    pub status: Option<String>,
    #[schemars(description = "Filter by tag (case-insensitive exact match)")]
    pub tag: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetEntityParams {
    #[schemars(
        description = "Entity ID with the repo's prefix, e.g. CUST-001, CONT-001, PROJ-001, MTG-001, RES-001, TASK-001, SPR-001, PROP-001 (prefixes are in mc://config). Case and zero-padding are forgiven: task-7 means TASK-007."
    )]
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadEntityFileParams {
    #[schemars(description = "Entity ID, e.g. CUST-001 or TASK-001")]
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateCustomerParams {
    #[schemars(description = "Customer name (required, non-empty)")]
    pub name: String,
    #[schemars(description = "Owner (username or name)")]
    pub owner: Option<String>,
    #[schemars(
        description = "Status (default: first configured customer status, usually 'active'; valid values in mc://config statuses.customer)"
    )]
    pub status: Option<String>,
    #[schemars(description = "Comma-separated tags, e.g. 'enterprise,emea'")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateProjectParams {
    #[schemars(description = "Project name (required, non-empty)")]
    pub name: String,
    #[schemars(description = "Owner (username or name)")]
    pub owner: Option<String>,
    #[schemars(
        description = "Status (default: first configured project status, usually 'active'; valid values in mc://config statuses.project)"
    )]
    pub status: Option<String>,
    #[schemars(description = "Linked customer IDs, comma-separated, e.g. 'CUST-001,CUST-002'")]
    pub customers: Option<String>,
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateMeetingParams {
    #[schemars(description = "Meeting title (required, non-empty)")]
    pub title: String,
    #[schemars(description = "Date YYYY-MM-DD (defaults to today; validated)")]
    pub date: Option<String>,
    #[schemars(description = "Time HH:MM (defaults to 10:00)")]
    pub time: Option<String>,
    #[schemars(description = "Duration e.g. 30m, 1h (defaults to 30m)")]
    pub duration: Option<String>,
    #[schemars(
        description = "Status (default: first configured meeting status, usually 'scheduled'; valid values in mc://config statuses.meeting)"
    )]
    pub status: Option<String>,
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
    #[schemars(description = "Linked customer IDs, comma-separated")]
    pub customers: Option<String>,
    #[schemars(description = "Linked project IDs, comma-separated")]
    pub projects: Option<String>,
    #[schemars(description = "Comma-separated attendee names")]
    pub attendees: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateResearchParams {
    #[schemars(description = "Research title (required, non-empty)")]
    pub title: String,
    #[schemars(description = "Owner (username or name)")]
    pub owner: Option<String>,
    #[schemars(
        description = "Comma-separated agent names; one sub-folder is created per agent (defaults to claude,gemini,chatgpt,perplexity)"
    )]
    pub agents: Option<String>,
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateTaskParams {
    #[schemars(description = "Task title (required, non-empty)")]
    pub title: String,
    #[schemars(
        description = "Scope to a project, e.g. PROJ-001 (must exist; the task file is stored in that project's tasks/ folder)"
    )]
    pub project: Option<String>,
    #[schemars(
        description = "Scope to a customer, e.g. CUST-001 (must exist; ignored when project is set)"
    )]
    pub customer: Option<String>,
    #[schemars(description = "Owner (username or name)")]
    pub owner: Option<String>,
    #[schemars(
        description = "Status (default: first configured task status, usually 'backlog'; valid values in mc://config statuses.task)"
    )]
    pub status: Option<String>,
    #[schemars(description = "Priority 1-4 (1=critical, 2=high, 3=medium, 4=low; defaults to 3)")]
    pub priority: Option<u32>,
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
    #[schemars(description = "Sprint ID to assign, e.g. SPR-001")]
    pub sprint: Option<String>,
    #[schemars(
        description = "Milestone ID or unique title to assign, e.g. MS-001 (must exist; stored as the ID)"
    )]
    pub milestone: Option<String>,
    #[schemars(description = "Comma-separated task IDs this depends on (e.g. TASK-001,TASK-002)")]
    pub depends_on: Option<String>,
    #[schemars(description = "Due date YYYY-MM-DD (validated)")]
    pub due_date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateMilestoneParams {
    #[schemars(description = "Milestone title, e.g. 'AP3: Integration' (required, non-empty)")]
    pub title: String,
    #[schemars(description = "What the milestone delivers (stored in the frontmatter)")]
    pub description: Option<String>,
    #[schemars(description = "Planned start date YYYY-MM-DD (validated)")]
    pub start_date: Option<String>,
    #[schemars(description = "Deadline YYYY-MM-DD (validated; must not be before start_date)")]
    pub due_date: Option<String>,
    #[schemars(description = "Owner (username or name)")]
    pub owner: Option<String>,
    #[schemars(
        description = "Initial status; valid values in mc://config statuses.milestone (default: the first, planned)"
    )]
    pub status: Option<String>,
    #[schemars(description = "Comma-separated project IDs to link (must exist), e.g. PROJ-001")]
    pub projects: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateSprintParams {
    #[schemars(description = "Sprint title, e.g. 2026-W05 (required, non-empty)")]
    pub title: String,
    #[schemars(description = "Owner (username or name)")]
    pub owner: Option<String>,
    #[schemars(
        description = "Status (default: first configured sprint status, usually 'planning'; valid values in mc://config statuses.sprint)"
    )]
    pub status: Option<String>,
    #[schemars(description = "Sprint goal")]
    pub goal: Option<String>,
    #[schemars(description = "Start date YYYY-MM-DD (defaults to today)")]
    pub start_date: Option<String>,
    #[schemars(description = "End date YYYY-MM-DD (must not be before start_date)")]
    pub end_date: Option<String>,
    #[schemars(description = "Linked project IDs, comma-separated")]
    pub projects: Option<String>,
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateProposalParams {
    #[schemars(description = "Proposal title (required, non-empty)")]
    pub title: String,
    #[schemars(description = "Author (username or name)")]
    pub author: Option<String>,
    #[schemars(
        description = "Status (default: first configured proposal status, usually 'draft'; valid values in mc://config statuses.proposal)"
    )]
    pub status: Option<String>,
    #[schemars(
        description = "Proposal type: architecture, feature, or process (defaults to architecture)"
    )]
    pub proposal_type: Option<String>,
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
    #[schemars(description = "ID of the proposal this one supersedes, e.g. PROP-001")]
    pub supersedes: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateContactParams {
    #[schemars(description = "Contact full name (required, non-empty)")]
    pub name: String,
    #[schemars(
        description = "Customer ID the contact belongs to, e.g. CUST-001 (required, must exist)"
    )]
    pub customer: String,
    #[schemars(description = "Role or job title")]
    pub role: Option<String>,
    #[schemars(description = "Email address")]
    pub email: Option<String>,
    #[schemars(description = "Phone number")]
    pub phone: Option<String>,
    #[schemars(
        description = "Status (default: first configured contact status, usually 'active'; valid values in mc://config statuses.contact)"
    )]
    pub status: Option<String>,
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MoveTaskParams {
    #[schemars(description = "Task ID, e.g. TASK-001")]
    pub id: String,
    #[schemars(
        description = "Target status (valid values in mc://config statuses.task, e.g. backlog, todo, in-progress, review, done, cancelled). Case-insensitive; aliases like doing/wip (in-progress) and completed (done) are accepted. Moving to done/cancelled moves the file to done/."
    )]
    pub status: String,
    #[schemars(description = "Sprint ID to assign, e.g. SPR-001")]
    pub sprint: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateTaskParams {
    #[schemars(description = "Task ID, e.g. TASK-001 (task-1 works too)")]
    pub id: String,
    #[schemars(
        description = "New title (single line; also replaces the note's leading '# Title' heading)"
    )]
    pub title: Option<String>,
    #[schemars(
        description = "New status (valid values in mc://config statuses.task; case-insensitive, aliases like doing/wip accepted). Same as move_task: done/cancelled move the file to done/."
    )]
    pub status: Option<String>,
    #[schemars(description = "Priority 1-4 (1=critical, 2=high, 3=medium, 4=low)")]
    pub priority: Option<u32>,
    #[schemars(description = "Owner (username or name); empty string clears it")]
    pub owner: Option<String>,
    #[schemars(
        description = "Sprint ID or title, e.g. SPR-001 (must exist; stored as the ID); empty string clears it"
    )]
    pub sprint: Option<String>,
    #[schemars(
        description = "Milestone ID or unique title, e.g. MS-001 (must exist; stored as the ID); empty string clears it"
    )]
    pub milestone: Option<String>,
    #[schemars(description = "Due date YYYY-MM-DD (validated); empty string clears it")]
    pub due_date: Option<String>,
    #[schemars(
        description = "Comma-separated project IDs that replace the linked projects (must exist); empty string clears them. The task file stays where it is."
    )]
    pub projects: Option<String>,
    #[schemars(
        description = "Comma-separated customer IDs that replace the linked customers (must exist); empty string clears them"
    )]
    pub customers: Option<String>,
    #[schemars(
        description = "Comma-separated tags that replace the task's tags; empty string clears them"
    )]
    pub tags: Option<String>,
    #[schemars(
        description = "Comma-separated task IDs that replace the dependencies (must exist); empty string clears them"
    )]
    pub depends_on: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NextTasksParams {
    #[schemars(description = "Only tasks linked to this project ID, e.g. PROJ-001")]
    pub project: Option<String>,
    #[schemars(description = "Only tasks linked to this customer ID, e.g. CUST-001")]
    pub customer: Option<String>,
    #[schemars(description = "Only tasks of this owner (case-insensitive)")]
    pub owner: Option<String>,
    #[schemars(description = "How many tasks to return (default 5)")]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListChecklistParams {
    #[schemars(description = "Entity ID of any kind, e.g. TASK-001 or MTG-002")]
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CheckItemParams {
    #[schemars(description = "Entity ID of any kind, e.g. TASK-001 or MTG-002")]
    pub id: String,
    #[schemars(
        description = "1-based item number as returned by list_checklist (document order of the '- [ ]' lines, excluding comments)"
    )]
    pub item: usize,
    #[schemars(
        description = "true ticks the item ('- [x]'), false unticks it ('- [ ]'). Defaults to true. Already being in that state is not an error (changed: false)."
    )]
    pub checked: Option<bool>,
    #[schemars(
        description = "Optional safety check: the item's text as list_checklist returned it. If the file now has different text at that number, nothing is changed and the call fails so you can re-list."
    )]
    pub expect_text: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddCommentParams {
    #[schemars(
        description = "Task or meeting ID, e.g. TASK-001 or MTG-002 (other kinds are rejected)"
    )]
    pub id: String,
    #[schemars(
        description = "Comment text in Markdown (required, non-empty). Headings of level 1-3 inside it are demoted to level 4 so they can't break the comments section."
    )]
    pub text: String,
    #[schemars(
        description = "Author name shown in the comment heading, one line, max 80 characters (defaults to the repo's git config user.name)"
    )]
    pub author: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListTasksParams {
    #[schemars(
        description = "Filter by status, case-insensitive (valid values in mc://config statuses.task)"
    )]
    pub status: Option<String>,
    #[schemars(description = "Filter by tag (case-insensitive exact match)")]
    pub tag: Option<String>,
    #[schemars(description = "Filter by linked project ID, e.g. PROJ-001")]
    pub project: Option<String>,
    #[schemars(description = "Filter by linked customer ID, e.g. CUST-001")]
    pub customer: Option<String>,
    #[schemars(description = "Filter by priority 1-4 (1=critical, 2=high, 3=medium, 4=low)")]
    pub priority: Option<u32>,
    #[schemars(description = "Filter by sprint ID, e.g. SPR-001")]
    pub sprint: Option<String>,
    #[schemars(
        description = "Filter by milestone ID or unique title, e.g. MS-001; an empty string lists tasks without a milestone. An unknown milestone is an error."
    )]
    pub milestone: Option<String>,
    #[schemars(description = "Filter by owner (case-insensitive)")]
    pub owner: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrintMeetingParams {
    #[schemars(description = "Meeting ID, e.g. MTG-001")]
    pub id: String,
    #[schemars(
        description = "Output .pdf path relative to the repo root, inside the repo (defaults to {id}.pdf at the repo root)"
    )]
    pub output: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrintResearchParams {
    #[schemars(description = "Research ID, e.g. RES-001")]
    pub id: String,
    #[schemars(
        description = "Output .pdf path relative to the repo root, inside the repo (defaults to {id}-final-report.pdf at the repo root)"
    )]
    pub output: Option<String>,
    #[schemars(
        description = "Only include final/ files whose name contains this text (default: all final/*.md files; falls back to the entity body when final/ is empty)"
    )]
    pub file: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrintFileParams {
    #[schemars(description = "Path to a .md file inside the repo, relative to the repo root")]
    pub path: String,
    #[schemars(
        description = "Output .pdf path relative to the repo root, inside the repo (defaults to <filename>.pdf at the repo root)"
    )]
    pub output: Option<String>,
    #[schemars(
        description = "Cover page template: standard, meeting, research, or sprint (defaults to standard)"
    )]
    pub template: Option<String>,
    #[schemars(
        description = "Override document title (auto-detected from frontmatter title/name, first H1, or filename)"
    )]
    pub title: Option<String>,
}

// ---------------------------------------------------------------------------
// McServer
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct McServer {
    cfg: ResolvedConfig,
    tool_router: ToolRouter<Self>,
}

impl McServer {
    pub fn new(cfg: ResolvedConfig) -> Self {
        Self {
            cfg,
            tool_router: Self::tool_router(),
        }
    }

    /// Look up an entity from loose input (see [`resolve_entity`]); an
    /// unknown ID suggests the closest existing one and the tool to list them.
    fn find(&self, id: &str, kind: Option<EntityKind>) -> Result<data::EntityRecord, McpError> {
        resolve_entity(&self.cfg, id, kind).map_err(|e| match &e {
            McError::EntityNotFound(missing) => {
                let missing_kind = kind.or_else(|| EntityKind::from_id(missing, &self.cfg).ok());
                let tool = match missing_kind {
                    Some(EntityKind::Task) => "list_tasks",
                    _ => "list_entities",
                };
                let hint = match closest_id(&self.cfg, missing) {
                    Some(close) => format!("did you mean {close}? Use {tool} to see valid IDs"),
                    None => format!("use {tool} to see valid IDs"),
                };
                McpError::invalid_params(format!("{e} ({hint})"), None)
            }
            _ => tool_err(e),
        })
    }

    /// Where a print tool writes its PDF: `output` (default `default_name`)
    /// relative to the repo root, never the server's working directory,
    /// which MCP clients set to anything (often `/`).
    fn pdf_output(&self, output: Option<&str>, default_name: &str) -> String {
        let out = output
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .unwrap_or(default_name);
        self.cfg.root.join(out).display().to_string()
    }

    /// Tool result with its `path` made repo-relative.
    fn with_relative_path(&self, mut result: JsonValue) -> Result<CallToolResult, McpError> {
        relativize_field(&self.cfg, &mut result, "path");
        json_result(&result)
    }

    /// JSON for the `mc://config` resource.
    fn config_json(&self) -> JsonValue {
        let cfg = &self.cfg;
        let mut prefixes = serde_json::Map::new();
        let mut statuses = serde_json::Map::new();
        for kind in EntityKind::ALL {
            prefixes.insert(kind.label().into(), kind.prefix(cfg).into());
            statuses.insert(kind.label().into(), kind.statuses(cfg).into());
        }
        let mode = match cfg.mode {
            RepoMode::Standalone => "standalone",
            RepoMode::Embedded => "embedded",
        };
        serde_json::json!({
            "name": cfg.brand.name,
            "mode": mode,
            "available_kinds": available_kinds(cfg)
                .iter()
                .map(|k| k.label_plural())
                .collect::<Vec<_>>(),
            "id_prefixes": prefixes,
            "statuses": statuses,
            "paths": {
                "customers": cfg.customers_dir.display().to_string(),
                "projects": cfg.projects_dir.display().to_string(),
                "meetings": cfg.meetings_dir.display().to_string(),
                "research": cfg.research_dir.display().to_string(),
                "tasks": cfg.tasks_dir.display().to_string(),
                "sprints": cfg.sprints_dir.display().to_string(),
                "milestones": cfg.milestones_dir.display().to_string(),
                "proposals": cfg.proposals_dir.display().to_string(),
            },
        })
    }
}

#[tool_router]
impl McServer {
    #[tool(
        description = "List entities of a given kind with optional status/tag filters. Returns a JSON array of entity objects (frontmatter fields with wiki-link brackets stripped, plus _kind and _source = repo-relative file path), sorted by ID. For tasks, prefer list_tasks which supports richer filters."
    )]
    async fn list_entities(
        &self,
        Parameters(params): Parameters<ListEntitiesParams>,
    ) -> Result<CallToolResult, McpError> {
        let kind = EntityKind::from_str_loose(&params.kind).map_err(tool_err)?;
        require_available(kind, &self.cfg)?;
        let status = params
            .status
            .as_deref()
            .map(|s| status_filter(&self.cfg, kind, s));
        let entities =
            data::collect_filtered(kind, &self.cfg, status.as_deref(), params.tag.as_deref())
                .map_err(tool_err)?;
        let json: Vec<JsonValue> = entities.iter().map(|e| entity_json(e, &self.cfg)).collect();
        json_result(&json)
    }

    #[tool(
        description = "Get an entity by its ID. Returns a JSON object with its frontmatter fields, _source (repo-relative path) and _body_preview (first 500 characters of the markdown body)."
    )]
    async fn get_entity(
        &self,
        Parameters(params): Parameters<GetEntityParams>,
    ) -> Result<CallToolResult, McpError> {
        let rec = self.find(&params.id, None)?;
        let mut json = entity_json(&rec, &self.cfg);
        if let Some(obj) = json.as_object_mut() {
            let preview: String = rec.body.chars().take(500).collect();
            obj.insert("_body_preview".into(), JsonValue::String(preview));
        }
        json_result(&json)
    }

    #[tool(
        description = "Read the full markdown content of an entity file (YAML frontmatter + body) as plain text. Use this instead of get_entity when you need the complete document."
    )]
    async fn read_entity_file(
        &self,
        Parameters(params): Parameters<ReadEntityFileParams>,
    ) -> Result<CallToolResult, McpError> {
        let rec = self.find(&params.id, None)?;
        let content = std::fs::read_to_string(&rec.source_path).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(content)]))
    }

    #[tool(
        description = "Create a new customer with its folder structure (contacts/, contracts/, meetings/, projects/, assets/). Not available in embedded repos. Returns JSON {id, name, path}."
    )]
    async fn create_customer(
        &self,
        Parameters(p): Parameters<CreateCustomerParams>,
    ) -> Result<CallToolResult, McpError> {
        let input = new_cmd::CustomerInput {
            name: p.name,
            owner: p.owner,
            status: p.status,
            tags: csv(p.tags),
        };
        let created = new_cmd::create_customer(&self.cfg, &input).map_err(tool_err)?;
        self.with_relative_path(created.to_json())
    }

    #[tool(
        description = "Create a new project with roadmap.md, backlog.md and specs/, releases/, infra/ folders. Not available in embedded repos. Returns JSON {id, name, path}."
    )]
    async fn create_project(
        &self,
        Parameters(p): Parameters<CreateProjectParams>,
    ) -> Result<CallToolResult, McpError> {
        let input = new_cmd::ProjectInput {
            name: p.name,
            owner: p.owner,
            status: p.status,
            customers: csv(p.customers),
            tags: csv(p.tags),
        };
        let created = new_cmd::create_project(&self.cfg, &input).map_err(tool_err)?;
        self.with_relative_path(created.to_json())
    }

    #[tool(
        description = "Create a new meeting note (file named <date>-<slug>.md). Returns JSON {id, title, path}."
    )]
    async fn create_meeting(
        &self,
        Parameters(p): Parameters<CreateMeetingParams>,
    ) -> Result<CallToolResult, McpError> {
        let input = new_cmd::MeetingInput {
            title: p.title,
            date: p.date,
            time: p.time,
            duration: p.duration,
            status: p.status,
            tags: csv(p.tags),
            customers: csv(p.customers),
            projects: csv(p.projects),
            attendees: csv(p.attendees),
        };
        let created = new_cmd::create_meeting(&self.cfg, &input).map_err(tool_err)?;
        self.with_relative_path(created.to_json())
    }

    #[tool(
        description = "Create a new research topic with one folder per agent plus final/. Status starts as 'draft'. Returns JSON {id, title, path}."
    )]
    async fn create_research(
        &self,
        Parameters(p): Parameters<CreateResearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let input = new_cmd::ResearchInput {
            title: p.title,
            owner: p.owner,
            agents: p.agents.as_deref().map(parse_comma_list),
            tags: csv(p.tags),
        };
        let created = new_cmd::create_research(&self.cfg, &input).map_err(tool_err)?;
        self.with_relative_path(created.to_json())
    }

    #[tool(
        description = "Create a new task (global, or scoped to a project/customer), in its todo/ or done/ folder depending on status. Optionally assign a sprint and a milestone. Returns JSON {id, title, path}."
    )]
    async fn create_task(
        &self,
        Parameters(p): Parameters<CreateTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let input = new_cmd::TaskInput {
            title: p.title,
            project: p.project,
            customer: p.customer,
            owner: p.owner,
            status: p.status,
            priority: p.priority,
            tags: csv(p.tags),
            sprint: p.sprint,
            milestone: p.milestone,
            depends_on: csv(p.depends_on),
            due_date: p.due_date,
        };
        let created = new_cmd::create_task(&self.cfg, &input).map_err(tool_err)?;
        self.with_relative_path(created.to_json())
    }

    #[tool(
        description = "Create a milestone (work package) that groups tasks, in milestones/MS-NNN-<slug>/. Optional description, start_date and due_date (YYYY-MM-DD; due_date not before start_date), owner, status (see mc://config statuses.milestone; default planned) and projects. Returns JSON {id, title, path}. Assign tasks with create_task's or update_task's milestone field; filter with list_tasks' milestone."
    )]
    async fn create_milestone(
        &self,
        Parameters(p): Parameters<CreateMilestoneParams>,
    ) -> Result<CallToolResult, McpError> {
        let input = new_cmd::MilestoneInput {
            title: p.title,
            description: p.description,
            start_date: p.start_date,
            due_date: p.due_date,
            owner: p.owner,
            status: p.status,
            projects: csv(p.projects),
        };
        let created = new_cmd::create_milestone(&self.cfg, &input).map_err(tool_err)?;
        self.with_relative_path(created.to_json())
    }

    #[tool(
        description = "Create a new sprint with planning.md, review.md and retrospective.md. Returns JSON {id, title, path}."
    )]
    async fn create_sprint(
        &self,
        Parameters(p): Parameters<CreateSprintParams>,
    ) -> Result<CallToolResult, McpError> {
        let input = new_cmd::SprintInput {
            title: p.title,
            owner: p.owner,
            status: p.status,
            goal: p.goal,
            start_date: p.start_date,
            end_date: p.end_date,
            projects: csv(p.projects),
            tags: csv(p.tags),
        };
        let created = new_cmd::create_sprint(&self.cfg, &input).map_err(tool_err)?;
        self.with_relative_path(created.to_json())
    }

    #[tool(
        description = "Create a new proposal (architecture/feature/process decision record). Returns JSON {id, title, path}."
    )]
    async fn create_proposal(
        &self,
        Parameters(p): Parameters<CreateProposalParams>,
    ) -> Result<CallToolResult, McpError> {
        let input = new_cmd::ProposalInput {
            title: p.title,
            author: p.author,
            status: p.status,
            proposal_type: p.proposal_type,
            tags: csv(p.tags),
            supersedes: p.supersedes,
        };
        let created = new_cmd::create_proposal(&self.cfg, &input).map_err(tool_err)?;
        self.with_relative_path(created.to_json())
    }

    #[tool(
        description = "Create a new contact under an existing customer. Not available in embedded repos. Returns JSON {id, name, path}."
    )]
    async fn create_contact(
        &self,
        Parameters(p): Parameters<CreateContactParams>,
    ) -> Result<CallToolResult, McpError> {
        let input = new_cmd::ContactInput {
            name: p.name,
            customer: p.customer,
            role: p.role,
            email: p.email,
            phone: p.phone,
            status: p.status,
            tags: csv(p.tags),
        };
        let created = new_cmd::create_contact(&self.cfg, &input).map_err(tool_err)?;
        self.with_relative_path(created.to_json())
    }

    #[tool(
        description = "Move a task to a new status (and optionally assign a sprint). Returns JSON {id, old_status, new_status, path} (path relative to the repo root)."
    )]
    async fn move_task(
        &self,
        Parameters(params): Parameters<MoveTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let rec = self.find(&params.id, None)?;
        if rec.kind != EntityKind::Task {
            return Err(McpError::invalid_params(
                format!(
                    "{} is a {}, not a task (move_task only moves tasks)",
                    rec.id,
                    rec.kind.label()
                ),
                None,
            ));
        }
        // Same forgiving statuses as `mc task move`: case, `_` and aliases
        // such as `doing` → in-progress.
        let status =
            suggest::resolve_status(&params.status, &self.cfg.statuses.task, EntityKind::Task)
                .map_err(tool_err)?;
        let sprint = params
            .sprint
            .as_deref()
            .map(|s| loose_ref(&self.cfg, s, EntityKind::Sprint));
        let result =
            commands::task::move_task_programmatic(&self.cfg, &rec.id, &status, sprint.as_deref())
                .map_err(tool_err)?;
        self.with_relative_path(result)
    }

    #[tool(
        description = "Change a task's fields: title, status, priority, owner, sprint, milestone, due_date, projects, customers, tags, depends_on. Only the fields you pass change; empty strings clear. Values are validated like create_task (references must exist) and nothing is written if any is invalid. Returns JSON {id, changed: [field names that actually changed], old_status, new_status, path} (path relative to the repo root)."
    )]
    async fn update_task(
        &self,
        Parameters(p): Parameters<UpdateTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let rec = self.find(&p.id, Some(EntityKind::Task))?;
        let list = |v: Option<String>| v.as_deref().map(parse_comma_list);
        let update = commands::task::TaskUpdate {
            title: p.title,
            status: p.status,
            priority: p.priority,
            owner: p.owner,
            sprint: p.sprint,
            milestone: p.milestone,
            due_date: p.due_date,
            projects: list(p.projects),
            customers: list(p.customers),
            tags: list(p.tags),
            depends_on: list(p.depends_on),
        };
        let updated = commands::task::update_task(&self.cfg, &rec.id, &update).map_err(tool_err)?;
        json_result(&updated.to_json(&self.cfg))
    }

    #[tool(
        description = "What to work on next: open tasks (status todo or backlog) whose dependencies are all done or cancelled, best first (todo before backlog, then priority, due date, ID). Returns JSON {tasks: [task objects shaped like list_tasks], actionable: total actionable count, blocked: open tasks waiting on unfinished dependencies}."
    )]
    async fn next_tasks(
        &self,
        Parameters(p): Parameters<NextTasksParams>,
    ) -> Result<CallToolResult, McpError> {
        let next = next_tasks_json(
            &self.cfg,
            p.project.as_deref(),
            p.customer.as_deref(),
            p.owner.as_deref(),
            p.limit,
        )
        .map_err(tool_err)?;
        json_result(&next)
    }

    #[tool(
        description = "List the Markdown checklist ('- [ ] item' / '- [x] item' lines) of any entity, in document order. Items in code blocks and in the comments section of tasks/meetings are not included. Returns JSON {id, items: [{index, line, checked, text}], done, total}."
    )]
    async fn list_checklist(
        &self,
        Parameters(p): Parameters<ListChecklistParams>,
    ) -> Result<CallToolResult, McpError> {
        let rec = self.find(&p.id, None)?;
        let content = std::fs::read_to_string(&rec.source_path).map_err(mc_err)?;
        let items = checklist::entity_items(rec.kind, &content);
        let (done, total) = checklist::progress(&items);
        json_result(
            &serde_json::json!({"id": rec.id, "items": items, "done": done, "total": total}),
        )
    }

    #[tool(
        description = "Tick or untick one checklist item of an entity (use list_checklist for item numbers). Only the box character in the file changes; frontmatter and all other text stay byte-for-byte. Returns JSON {id, item: {index, line, checked, text}, changed, done, total}."
    )]
    async fn check_item(
        &self,
        Parameters(p): Parameters<CheckItemParams>,
    ) -> Result<CallToolResult, McpError> {
        let rec = self.find(&p.id, None)?;
        let change = checklist::set_checked(
            &rec,
            checklist::Target::Index(p.item),
            p.checked.unwrap_or(true),
            None,
            p.expect_text.as_deref(),
        )
        .map_err(tool_err)?;
        json_result(&serde_json::json!({
            "id": rec.id,
            "item": change.item,
            "changed": change.changed,
            "done": change.done,
            "total": change.total,
        }))
    }

    #[tool(
        description = "Add a comment to a task or meeting. It is appended to a '## Comments' section at the end of the entity's Markdown file (created if missing) under a heading '### YYYY-MM-DD HH:MM · Author' (local time). Returns JSON {id, comment: {date, time, author, heading, body}, count, path}."
    )]
    async fn add_comment(
        &self,
        Parameters(p): Parameters<AddCommentParams>,
    ) -> Result<CallToolResult, McpError> {
        let rec = self.find(&p.id, None)?;
        let added =
            comments::add(&self.cfg, &rec, &p.text, p.author.as_deref()).map_err(tool_err)?;
        json_result(&added)
    }

    #[tool(
        description = "List tasks across all locations (global, per-project, per-customer) with rich filtering (status, tag, project, customer, priority, sprint, milestone, owner); all filters combine with AND. Returns a JSON array sorted by ID. Use this instead of list_entities for tasks."
    )]
    async fn list_tasks(
        &self,
        Parameters(params): Parameters<ListTasksParams>,
    ) -> Result<CallToolResult, McpError> {
        let cfg = &self.cfg;
        let status = params
            .status
            .as_deref()
            .map(|s| status_filter(cfg, EntityKind::Task, s));
        let id_ref = |v: &Option<String>, kind| v.as_deref().map(|v| loose_ref(cfg, v, kind));
        let project = id_ref(&params.project, EntityKind::Project);
        let customer = id_ref(&params.customer, EntityKind::Customer);
        let sprint = id_ref(&params.sprint, EntityKind::Sprint);
        let milestone = id_ref(&params.milestone, EntityKind::Milestone);
        let filter = data::TaskFilter {
            status: status.as_deref(),
            tag: params.tag.as_deref(),
            project: project.as_deref(),
            customer: customer.as_deref(),
            priority: params.priority,
            sprint: sprint.as_deref(),
            owner: params.owner.as_deref(),

            milestone: milestone.as_deref(),
        };
        let tasks = data::collect_tasks_filtered(cfg, &filter).map_err(tool_err)?;
        let json: Vec<JsonValue> = tasks.iter().map(|e| entity_json(e, &self.cfg)).collect();
        json_result(&json)
    }

    #[tool(
        description = "Export a meeting to a branded PDF (cover page, attendees table, notes). Returns JSON {id, title, path} (path relative to the repo root, where the PDF is written by default)."
    )]
    async fn print_meeting(
        &self,
        Parameters(params): Parameters<PrintMeetingParams>,
    ) -> Result<CallToolResult, McpError> {
        let rec = self.find(&params.id, Some(EntityKind::Meeting))?;
        let output = self.pdf_output(params.output.as_deref(), &format!("{}.pdf", rec.id));
        let result = commands::print::print_meeting_programmatic(&self.cfg, &rec.id, Some(&output))
            .map_err(tool_err)?;
        self.with_relative_path(result)
    }

    #[tool(
        description = "Export a research topic's final report (files in its final/ folder) to a branded PDF. Returns JSON {id, title, path} (path relative to the repo root, where the PDF is written by default)."
    )]
    async fn print_research(
        &self,
        Parameters(params): Parameters<PrintResearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let rec = self.find(&params.id, Some(EntityKind::Research))?;
        let output = self.pdf_output(
            params.output.as_deref(),
            &format!("{}-final-report.pdf", rec.id),
        );
        let result = commands::print::print_research_programmatic(
            &self.cfg,
            &rec.id,
            Some(&output),
            params.file.as_deref(),
        )
        .map_err(tool_err)?;
        self.with_relative_path(result)
    }

    #[tool(
        description = "Generate a branded PDF from any markdown file. Returns JSON {title, path} (path relative to the repo root, where the PDF is written by default)."
    )]
    async fn print_file(
        &self,
        Parameters(params): Parameters<PrintFileParams>,
    ) -> Result<CallToolResult, McpError> {
        use crate::cli::PrintTemplate;
        let template = match params
            .template
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("") | Some("standard") => PrintTemplate::Standard,
            Some("meeting") => PrintTemplate::Meeting,
            Some("research") => PrintTemplate::Research,
            Some("sprint") => PrintTemplate::Sprint,
            Some(other) => {
                return Err(McpError::invalid_params(
                    format!(
                    "Invalid template '{other}' (expected standard, meeting, research, or sprint)"
                ),
                    None,
                ))
            }
        };
        // Relative paths are read from and written to the repo root.
        let input = self.cfg.root.join(&params.path);
        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "document".into());
        let output = self.pdf_output(params.output.as_deref(), &format!("{stem}.pdf"));
        let result = commands::print::print_file_programmatic(
            &self.cfg,
            &input.display().to_string(),
            Some(&output),
            &template,
            params.title.as_deref(),
        )
        .map_err(tool_err)?;
        self.with_relative_path(result)
    }

    #[tool(
        description = "Validate repo structure and frontmatter. Returns the text 'Validation passed: no issues found.' or a JSON array of {path, check, severity, message} issue objects (severity 'error' or 'warning'; warnings such as links to missing entities don't fail validation)."
    )]
    async fn validate_repo(&self) -> Result<CallToolResult, McpError> {
        let issues = commands::validate::validate_programmatic(&self.cfg).map_err(tool_err)?;
        if issues.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "Validation passed: no issues found.",
            )]));
        }
        json_result(&issues)
    }

    #[tool(
        description = "Rebuild the JSON index files in data/ (one per entity kind). Returns a summary string with entity counts."
    )]
    async fn build_index(&self) -> Result<CallToolResult, McpError> {
        let result = commands::index::run_quiet(&self.cfg).map_err(tool_err)?;
        let text = format!(
            "Index built: {} customers, {} projects, {} meetings, {} research, {} tasks, {} sprints, {} proposals, {} contacts",
            result.customers, result.projects, result.meetings, result.research, result.tasks, result.sprints, result.proposals, result.contacts,
        );
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Get a status overview. Returns JSON with 'name' (repo display name), 'counts' (per available entity kind: total and by_status) and 'recent_activity' (the 10 most recently modified entity files: id, name, path)."
    )]
    async fn get_status(&self) -> Result<CallToolResult, McpError> {
        let mut counts = serde_json::Map::new();
        for kind in available_kinds(&self.cfg) {
            let sc = data::count_by_status(kind, &self.cfg).map_err(tool_err)?;
            let by_status: serde_json::Map<String, JsonValue> = sc
                .by_status
                .into_iter()
                .map(|(s, c)| (s, JsonValue::Number(c.into())))
                .collect();
            counts.insert(
                kind.label_plural().to_string(),
                serde_json::json!({
                    "total": sc.total,
                    "by_status": by_status,
                }),
            );
        }

        let recent = data::recent_activity(&self.cfg, 10).map_err(tool_err)?;
        let recent_json: Vec<JsonValue> = recent
            .iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id,
                    "name": f.name,
                    "path": repo_relative(&self.cfg, &f.path),
                })
            })
            .collect();

        json_result(&serde_json::json!({
            "name": self.cfg.brand.name,
            "counts": counts,
            "recent_activity": recent_json,
        }))
    }
}

// ---------------------------------------------------------------------------
// ServerHandler -- provides get_info, list_resources, read_resource
// ---------------------------------------------------------------------------

#[tool_handler]
impl ServerHandler for McServer {
    fn get_info(&self) -> ServerInfo {
        let mode_label = match self.cfg.mode {
            RepoMode::Standalone => "standalone",
            RepoMode::Embedded => "embedded",
        };
        let kinds: Vec<&str> = available_kinds(&self.cfg)
            .iter()
            .map(|k| k.label_plural())
            .collect();
        let instructions = format!(
            "MissionControl repo '{}' ({}) at {}. Manage {} in a git-based knowledge repository.\n\nStart with get_status for an overview. Read the mc://config resource for valid status values and ID prefixes. Use list_tasks (not list_entities) for task queries — it supports richer filters. next_tasks answers \"what should I work on next?\"; update_task changes a task's fields.",
            self.cfg.brand.name,
            mode_label,
            self.cfg.root.display(),
            kinds.join(", "),
        );
        ServerInfo {
            instructions: Some(instructions),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            ..Default::default()
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        fn described_resource(uri: &str, name: &str, desc: &str) -> Annotated<RawResource> {
            let mut r = RawResource::new(uri, name);
            r.description = Some(desc.to_string());
            r.mime_type = Some("application/json".to_string());
            Annotated::new(r, None)
        }

        let mut resources = vec![described_resource(
            "mc://config",
            "config",
            "Repository configuration: name, mode, available entity kinds, valid status values, ID prefixes, and directory paths",
        )];
        for kind in available_kinds(&self.cfg) {
            let plural = kind.label_plural();
            let desc = if kind == EntityKind::Task {
                "All tasks as a JSON array (unfiltered — use the list_tasks tool for filtering)"
                    .to_string()
            } else {
                format!("All {plural} as a JSON array, sorted by ID")
            };
            resources.push(described_resource(
                &format!("mc://entities/{plural}"),
                plural,
                &desc,
            ));
        }

        Ok(ListResourcesResult {
            resources,
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = &request.uri;
        let not_found =
            || McpError::resource_not_found(format!("Unknown resource URI: {}", uri), None);

        let value = if uri == "mc://config" {
            self.config_json()
        } else {
            let plural = uri.strip_prefix("mc://entities/").ok_or_else(not_found)?;
            let kind = available_kinds(&self.cfg)
                .into_iter()
                .find(|k| k.label_plural() == plural)
                .ok_or_else(not_found)?;
            JsonValue::Array(collect_entity_json(kind, &self.cfg)?)
        };
        let text = serde_json::to_string_pretty(&value).map_err(mc_err)?;

        Ok(ReadResourceResult {
            contents: vec![ResourceContents::text(text, uri.clone())],
        })
    }
}

fn collect_entity_json(kind: EntityKind, cfg: &ResolvedConfig) -> Result<Vec<JsonValue>, McpError> {
    let entities = data::collect_entities(kind, cfg).map_err(tool_err)?;
    Ok(entities.iter().map(|e| entity_json(e, cfg)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::init;
    use crate::config;
    use rmcp::model::ErrorCode;
    use tempfile::TempDir;

    fn server(embedded: bool) -> (TempDir, McServer) {
        let tmp = TempDir::new().unwrap();
        init::run(tmp.path(), false, embedded, Some("Ops HQ"), false, true).unwrap();
        let mode = config::detect_mode(tmp.path());
        let cfg = config::load_config(tmp.path(), mode).unwrap();
        (tmp, McServer::new(cfg))
    }

    fn text(result: &CallToolResult) -> String {
        result.content[0]
            .as_text()
            .map(|t| t.text.clone())
            .unwrap_or_default()
    }

    fn json(result: &CallToolResult) -> JsonValue {
        serde_json::from_str(&text(result)).unwrap()
    }

    #[test]
    fn test_tool_surface_and_parameter_docs() {
        let tools = McServer::tool_router().list_all();
        let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "add_comment",
                "build_index",
                "check_item",
                "create_contact",
                "create_customer",
                "create_meeting",
                "create_milestone",
                "create_project",
                "create_proposal",
                "create_research",
                "create_sprint",
                "create_task",
                "get_entity",
                "get_status",
                "list_checklist",
                "list_entities",
                "list_tasks",
                "move_task",
                "next_tasks",
                "print_file",
                "print_meeting",
                "print_research",
                "read_entity_file",
                "update_task",
                "validate_repo",
            ]
        );
        for tool in &tools {
            let desc = tool.description.as_deref().unwrap_or("");
            assert!(desc.len() > 20, "{} needs a description", tool.name);
            if let Some(props) = tool
                .input_schema
                .get("properties")
                .and_then(|p| p.as_object())
            {
                for (param, schema) in props {
                    assert!(
                        schema.get("description").is_some(),
                        "{}.{} has no description",
                        tool.name,
                        param
                    );
                }
            }
        }
        // Proposals work in embedded mode, so the tool must not claim otherwise.
        let proposal = tools.iter().find(|t| t.name == "create_proposal").unwrap();
        assert!(!proposal
            .description
            .as_deref()
            .unwrap()
            .contains("standalone"));
    }

    #[tokio::test]
    async fn test_create_list_get_roundtrip() {
        let (_tmp, srv) = server(false);
        srv.create_customer(Parameters(CreateCustomerParams {
            name: "Acme".into(),
            owner: None,
            status: None,
            tags: Some("a, b".into()),
        }))
        .await
        .unwrap();
        crate::commands::new::create_sprint(
            &srv.cfg,
            &crate::commands::new::SprintInput::new("S1"),
        )
        .unwrap();
        let created = json(
            &srv.create_task(Parameters(CreateTaskParams {
                title: "Call Acme".into(),
                project: None,
                customer: Some("CUST-001".into()),
                owner: None,
                status: None,
                priority: Some(2),
                tags: None,
                sprint: Some("SPR-001".into()),
                depends_on: None,
                due_date: None,

                milestone: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(created["id"], "TASK-001");

        let tasks = json(
            &srv.list_tasks(Parameters(ListTasksParams {
                status: None,
                tag: None,
                project: None,
                customer: Some("cust-001".into()),
                priority: Some(2),
                sprint: Some("SPR-001".into()),
                owner: None,

                milestone: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(tasks.as_array().unwrap().len(), 1);
        // Wiki-links are stripped and _source is repo-relative.
        assert_eq!(tasks[0]["customers"][0], "CUST-001");
        assert_eq!(tasks[0]["sprint"], "SPR-001");
        assert!(!tasks[0]["_source"].as_str().unwrap().starts_with('/'));

        let entity = json(
            &srv.get_entity(Parameters(GetEntityParams {
                id: "CUST-001".into(),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(entity["name"], "Acme");
        assert!(entity["_body_preview"].as_str().unwrap().contains("Acme"));
    }

    #[tokio::test]
    async fn test_user_errors_are_invalid_params() {
        let (_tmp, srv) = server(false);
        let err = srv
            .create_task(Parameters(CreateTaskParams {
                title: "T".into(),
                project: None,
                customer: None,
                owner: None,
                status: Some("bogus".into()),
                priority: None,
                tags: None,
                sprint: None,
                depends_on: None,
                due_date: None,

                milestone: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("Valid statuses"));

        let err = srv
            .get_entity(Parameters(GetEntityParams {
                id: "CUST-999".into(),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);

        let err = srv
            .list_entities(Parameters(ListEntitiesParams {
                kind: "widgets".into(),
                status: None,
                tag: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn test_checklist_and_comment_tools() {
        let (tmp, srv) = server(false);
        let created = json(
            &srv.create_task(Parameters(CreateTaskParams {
                title: "Ship it".into(),
                project: None,
                customer: None,
                owner: None,
                status: None,
                priority: None,
                tags: None,
                sprint: None,
                depends_on: None,
                due_date: None,

                milestone: None,
            }))
            .await
            .unwrap(),
        );
        // `path` is absolute, or relative to the repo root.
        let path = tmp.path().join(created["path"].as_str().unwrap());
        let original = std::fs::read_to_string(&path).unwrap();
        // Replace the template body with a known checklist.
        let (fm, _) = frontmatter::split_frontmatter(&original).unwrap();
        std::fs::write(&path, format!("---\n{fm}\n---\n- [ ] Write\n- [x] Test\n")).unwrap();

        let id = || created["id"].as_str().unwrap().to_string();
        let list = json(
            &srv.list_checklist(Parameters(ListChecklistParams { id: id() }))
                .await
                .unwrap(),
        );
        assert_eq!(
            (list["done"].as_u64(), list["total"].as_u64()),
            (Some(1), Some(2))
        );
        assert_eq!(list["items"][0]["text"], "Write");

        let done = json(
            &srv.check_item(Parameters(CheckItemParams {
                id: id(),
                item: 1,
                checked: None,
                expect_text: Some("Write".into()),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(done["changed"], true);
        assert_eq!(done["done"], 2);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("- [x] Write"));

        let err = srv
            .check_item(Parameters(CheckItemParams {
                id: id(),
                item: 2,
                checked: Some(false),
                expect_text: Some("Something else".into()),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);

        let added = json(
            &srv.add_comment(Parameters(AddCommentParams {
                id: id(),
                text: "Looks good".into(),
                author: Some("Agent".into()),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(added["count"], 1);
        assert_eq!(added["comment"]["author"], "Agent");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("## Comments\n\n### "));
        assert!(content.contains(" · Agent\n\nLooks good\n"));

        let err = srv
            .add_comment(Parameters(AddCommentParams {
                id: id(),
                text: "  ".into(),
                author: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn test_typed_errors_map_to_mcp_codes() {
        let usage = tool_err(McError::usage("bad value", Some("try x".into())));
        assert_eq!(usage.code, ErrorCode::INVALID_PARAMS);
        assert!(usage.message.contains("bad value (try x)"));
        let missing = tool_err(McError::not_found("no such file", None));
        assert_eq!(missing.code, ErrorCode::RESOURCE_NOT_FOUND);
        let io = tool_err(McError::Io(std::io::Error::other("disk")));
        assert_eq!(io.code, ErrorCode::INTERNAL_ERROR);
    }

    #[tokio::test]
    async fn test_embedded_mode_hides_unavailable_kinds() {
        let (_tmp, srv) = server(true);
        let err = srv
            .list_entities(Parameters(ListEntitiesParams {
                kind: "contacts".into(),
                status: None,
                tag: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);

        let status = json(&srv.get_status().await.unwrap());
        assert_eq!(status["name"], "Ops HQ");
        let counts = status["counts"].as_object().unwrap();
        assert!(counts.contains_key("tasks") && counts.contains_key("proposals"));
        assert!(!counts.contains_key("customers") && !counts.contains_key("contacts"));

        let info = srv.get_info();
        let instructions = info.instructions.unwrap();
        assert!(instructions.contains("Ops HQ") && !instructions.contains("contacts"));

        // Proposals are available in embedded repos.
        srv.create_proposal(Parameters(CreateProposalParams {
            title: "Adopt mc".into(),
            author: None,
            status: None,
            proposal_type: None,
            tags: None,
            supersedes: None,
        }))
        .await
        .unwrap();
    }

    fn task_params(title: &str) -> CreateTaskParams {
        CreateTaskParams {
            title: title.into(),
            project: None,
            customer: None,
            owner: None,
            status: None,
            priority: None,
            tags: None,
            sprint: None,
            depends_on: None,
            due_date: None,

            milestone: None,
        }
    }

    fn update_params(id: &str) -> UpdateTaskParams {
        UpdateTaskParams {
            id: id.into(),
            title: None,
            status: None,
            priority: None,
            owner: None,
            sprint: None,
            due_date: None,
            projects: None,
            customers: None,
            tags: None,
            depends_on: None,

            milestone: None,
        }
    }

    #[tokio::test]
    async fn test_update_task_and_next_tasks() {
        let (_tmp, srv) = server(false);
        for title in ["First", "Second", "Third"] {
            srv.create_task(Parameters(task_params(title)))
                .await
                .unwrap();
        }
        crate::commands::new::create_meeting(
            &srv.cfg,
            &crate::commands::new::MeetingInput::new("Sync"),
        )
        .unwrap();

        // TASK-002 waits on TASK-003; TASK-001 is a todo, so it comes first.
        let updated = json(
            &srv.update_task(Parameters(UpdateTaskParams {
                depends_on: Some("task-3".into()),
                priority: Some(1),
                ..update_params("task-2")
            }))
            .await
            .unwrap(),
        );
        assert_eq!(
            updated["changed"],
            serde_json::json!(["priority", "depends_on"])
        );
        assert!(updated["path"].as_str().unwrap().starts_with("tasks/todo/"));
        srv.update_task(Parameters(UpdateTaskParams {
            status: Some("TODO".into()),
            owner: Some("alice".into()),
            ..update_params("TASK-001")
        }))
        .await
        .unwrap();

        let next = |owner: Option<&str>| NextTasksParams {
            project: None,
            customer: None,
            owner: owner.map(Into::into),
            limit: None,
        };
        let queue = json(&srv.next_tasks(Parameters(next(None))).await.unwrap());
        let ids: Vec<&str> = queue["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["TASK-001", "TASK-003"]);
        assert_eq!(queue["actionable"], 2);
        assert_eq!(queue["blocked"], 1);
        let mine = json(
            &srv.next_tasks(Parameters(next(Some("Alice"))))
                .await
                .unwrap(),
        );
        assert_eq!(mine["tasks"].as_array().unwrap().len(), 1);

        // Finishing the dependency unblocks TASK-002.
        srv.update_task(Parameters(UpdateTaskParams {
            status: Some("done".into()),
            ..update_params("TASK-003")
        }))
        .await
        .unwrap();
        let queue = json(&srv.next_tasks(Parameters(next(None))).await.unwrap());
        assert_eq!(queue["blocked"], 0);
        assert_eq!(queue["actionable"], 2);

        // Bad values and other kinds are invalid_params; nothing changes.
        for params in [
            update_params("TASK-001"),
            UpdateTaskParams {
                priority: Some(9),
                ..update_params("TASK-001")
            },
            UpdateTaskParams {
                owner: Some("x".into()),
                ..update_params("MTG-001")
            },
        ] {
            let err = srv.update_task(Parameters(params)).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::INVALID_PARAMS, "{}", err.message);
        }
        // A missing sprint is not found, as in create_task.
        let err = srv
            .update_task(Parameters(UpdateTaskParams {
                sprint: Some("SPR-404".into()),
                ..update_params("TASK-001")
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::RESOURCE_NOT_FOUND, "{}", err.message);
        let err = srv
            .move_task(Parameters(MoveTaskParams {
                id: "MTG-001".into(),
                status: "done".into(),
                sprint: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("not a task"), "{}", err.message);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_concurrent_creates_get_distinct_ids() {
        let (_tmp, srv) = server(false);
        let calls: Vec<_> = (0..8)
            .map(|i| {
                let srv = srv.clone();
                tokio::spawn(async move {
                    json(
                        &srv.create_task(Parameters(task_params(&format!("Race {i}"))))
                            .await
                            .unwrap(),
                    )["id"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
            })
            .collect();
        let mut ids = Vec::new();
        for call in calls {
            ids.push(call.await.unwrap());
        }
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 8, "{ids:?}");
    }

    #[tokio::test]
    async fn test_ids_and_statuses_are_forgiving() {
        let (_tmp, srv) = server(false);
        srv.create_task(Parameters(task_params("Loose")))
            .await
            .unwrap();
        for id in ["TASK-1", "task-001", "task1"] {
            let entity = json(
                &srv.get_entity(Parameters(GetEntityParams { id: id.into() }))
                    .await
                    .unwrap(),
            );
            assert_eq!(entity["id"], "TASK-001", "{id}");
            assert_eq!(entity["_kind"], "task");
        }
        // An unknown ID suggests the closest one and the tool to list them,
        // not a CLI command.
        let err = srv
            .get_entity(Parameters(GetEntityParams {
                id: "TASK-2".into(),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("did you mean TASK-001 (Loose)"),
            "{}",
            err.message
        );
        assert!(err.message.contains("list_tasks") && !err.message.contains("mc list"));

        // `doing` means in-progress (the CLI alias), not "did you mean done".
        let moved = json(
            &srv.move_task(Parameters(MoveTaskParams {
                id: "task-1".into(),
                status: "doing".into(),
                sprint: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(moved["new_status"], "in-progress");
        assert!(moved["path"].as_str().unwrap().starts_with("tasks/todo/"));
        let err = srv
            .move_task(Parameters(MoveTaskParams {
                id: "TASK-001".into(),
                status: "revew".into(),
                sprint: None,
            }))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("did you mean 'review'"),
            "{}",
            err.message
        );

        let tasks = json(
            &srv.list_tasks(Parameters(ListTasksParams {
                status: Some("In_Progress".into()),
                tag: None,
                project: None,
                customer: None,
                priority: None,
                sprint: None,
                owner: None,

                milestone: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(tasks.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_paths_are_repo_relative_and_pdfs_land_in_the_repo() {
        let (tmp, srv) = server(false);
        let created = json(
            &srv.create_task(Parameters(task_params("Rel")))
                .await
                .unwrap(),
        );
        assert!(created["path"].as_str().unwrap().starts_with("tasks/todo/"));
        let status = json(&srv.get_status().await.unwrap());
        for entry in status["recent_activity"].as_array().unwrap() {
            assert!(!entry["path"].as_str().unwrap().starts_with('/'), "{entry}");
        }

        srv.create_meeting(Parameters(CreateMeetingParams {
            title: "Kickoff".into(),
            date: Some("2026-01-05".into()),
            time: None,
            duration: None,
            status: None,
            tags: None,
            customers: None,
            projects: None,
            attendees: None,
        }))
        .await
        .unwrap();
        // The server's working directory doesn't matter: the default output
        // is in the repo root, and the path comes back repo-relative.
        let printed = json(
            &srv.print_meeting(Parameters(PrintMeetingParams {
                id: "mtg-1".into(),
                output: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(printed["path"], "MTG-001.pdf");
        assert!(tmp.path().join("MTG-001.pdf").is_file());

        std::fs::write(tmp.path().join("notes.md"), "# Notes\n\nHello\n").unwrap();
        let printed = json(
            &srv.print_file(Parameters(PrintFileParams {
                path: "notes.md".into(),
                output: Some("notes-out.pdf".into()),
                template: None,
                title: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(printed["path"], "notes-out.pdf");
        assert!(tmp.path().join("notes-out.pdf").is_file());
    }

    #[tokio::test]
    async fn milestone_tool_creates_and_assigns_a_task() {
        let (_tmp, srv) = server(false);
        let created = json(
            &srv.create_milestone(Parameters(CreateMilestoneParams {
                title: "AP3".into(),
                description: Some("Training".into()),
                start_date: Some("2026-09-01".into()),
                due_date: Some("2026-11-30".into()),
                owner: None,
                status: None,
                projects: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(created["id"], "MS-001");
        let mut input = task_params("Train");
        input.milestone = Some("MS-001".into());
        let created = json(&srv.create_task(Parameters(input)).await.unwrap());
        let rec = resolve_entity(
            &srv.cfg,
            created["id"].as_str().unwrap(),
            Some(EntityKind::Task),
        )
        .unwrap();
        assert_eq!(
            frontmatter::get_link_str(&rec.frontmatter, "milestone"),
            Some("MS-001")
        );
    }

    #[tokio::test]
    async fn milestone_can_be_set_filtered_and_cleared_by_update_task() {
        let (_tmp, srv) = server(false);
        crate::commands::new::create_milestone(
            &srv.cfg,
            &crate::commands::new::MilestoneInput::new("Beta"),
        )
        .unwrap();
        for title in ["First", "Second"] {
            srv.create_task(Parameters(task_params(title)))
                .await
                .unwrap();
        }
        let list = |milestone: &str| ListTasksParams {
            status: None,
            tag: None,
            project: None,
            customer: None,
            priority: None,
            sprint: None,
            owner: None,
            milestone: Some(milestone.into()),
        };
        let mut update = update_params("TASK-001");
        update.milestone = Some("beta".into()); // unique title, any case
        let updated = json(&srv.update_task(Parameters(update)).await.unwrap());
        assert_eq!(updated["changed"], serde_json::json!(["milestone"]));
        let tasks = json(&srv.list_tasks(Parameters(list("ms-1"))).await.unwrap());
        assert_eq!(tasks.as_array().unwrap().len(), 1);
        assert_eq!(tasks[0]["milestone"], "MS-001");
        let unassigned = json(&srv.list_tasks(Parameters(list(""))).await.unwrap());
        assert_eq!(unassigned[0]["id"], "TASK-002");
        assert_eq!(unassigned.as_array().unwrap().len(), 1);
        assert!(srv.list_tasks(Parameters(list("MS-404"))).await.is_err());

        let mut clear = update_params("TASK-001");
        clear.milestone = Some(String::new());
        srv.update_task(Parameters(clear)).await.unwrap();
        let tasks = json(&srv.list_tasks(Parameters(list("MS-001"))).await.unwrap());
        assert!(tasks.as_array().unwrap().is_empty());
    }

    #[test]
    fn test_config_resource_json() {
        let (_tmp, srv) = server(false);
        let cfg = srv.config_json();
        assert_eq!(cfg["name"], "Ops HQ");
        assert_eq!(cfg["mode"], "standalone");
        assert_eq!(cfg["id_prefixes"]["task"], "TASK");
        assert_eq!(cfg["statuses"]["task"][0], "backlog");
        assert_eq!(cfg["available_kinds"].as_array().unwrap().len(), 9);
    }
}
