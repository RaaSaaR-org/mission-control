//! Typed request and response shapes for the REST API.
//!
//! Mirrors `commands::new::create_*_programmatic` and the MCP tool params.
//! All types implement `utoipa::ToSchema` so the generated OpenAPI document
//! describes them precisely.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use utoipa::ToSchema;

/// Generic create response — `{id, name, path}` for entity creates.
///
/// `name` accepts the legacy `title` field on inputs from creators that use
/// `title` (meeting, research, task, sprint, proposal). For clients consuming
/// the API, the response always uses `name`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateResult {
    pub id: String,
    #[serde(alias = "title")]
    pub name: String,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MoveTaskResult {
    pub id: String,
    pub old_status: String,
    pub new_status: String,
    pub path: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IndexResult {
    pub customers: usize,
    pub projects: usize,
    pub meetings: usize,
    pub research: usize,
    pub tasks: usize,
    pub sprints: usize,
    pub proposals: usize,
    pub contacts: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ValidationReport {
    pub ok: bool,
    pub issues: Vec<ValidationIssue>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ValidationIssue {
    pub path: String,
    pub check: String,
    pub message: String,
}

/// `/v1/status` payload.
#[derive(Debug, Serialize, ToSchema)]
pub struct StatusResponse {
    pub counts: Vec<KindStatusCounts>,
    pub recent: Vec<RecentEntry>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct KindStatusCounts {
    pub kind: String,
    pub total: usize,
    pub by_status: Vec<StatusCount>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StatusCount {
    pub status: String,
    pub count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecentEntry {
    pub id: String,
    pub name: String,
    pub modified: String,
}

/// `/v1/config` payload — a flattened view of the parts of `ResolvedConfig`
/// callers actually need (mode, prefixes, valid statuses, configured kinds).
#[derive(Debug, Serialize, ToSchema)]
pub struct ConfigResponse {
    pub mode: String,
    pub prefixes: PrefixView,
    pub statuses: StatusView,
    pub configured_entities: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PrefixView {
    pub customer: String,
    pub project: String,
    pub meeting: String,
    pub research: String,
    pub task: String,
    pub sprint: String,
    pub proposal: String,
    pub contact: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StatusView {
    pub customer: Vec<String>,
    pub project: Vec<String>,
    pub meeting: Vec<String>,
    pub research: Vec<String>,
    pub task: Vec<String>,
    pub sprint: Vec<String>,
    pub proposal: Vec<String>,
    pub contact: Vec<String>,
}

/// A single entity returned by GET endpoints. The `frontmatter` field is
/// untyped because each kind has its own shape; callers should use the typed
/// `kind` and `id` fields and treat `frontmatter` as opaque JSON.
#[derive(Debug, Serialize, ToSchema)]
pub struct EntityResponse {
    pub kind: String,
    pub id: String,
    pub source_path: String,
    pub frontmatter: JsonValue,
    pub body_preview: String,
}

// ───────────────────────── create request bodies ─────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCustomer {
    pub name: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProject {
    pub name: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub customers: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMeeting {
    pub title: String,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub customers: Option<String>,
    #[serde(default)]
    pub projects: Option<String>,
    #[serde(default)]
    pub attendees: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateResearch {
    pub title: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub agents: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTask {
    pub title: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub customer: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<u32>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub sprint: Option<String>,
    #[serde(default)]
    pub depends_on: Option<String>,
    #[serde(default)]
    pub due_date: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSprint {
    pub title: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub projects: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProposal {
    pub title: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, rename = "type")]
    pub proposal_type: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub supersedes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateContact {
    pub name: String,
    pub customer: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
}

// ───────────────────────── query params ─────────────────────────

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct EntityListQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct TaskListQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub customer: Option<String>,
    #[serde(default)]
    pub priority: Option<u32>,
    #[serde(default)]
    pub sprint: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MoveTaskBody {
    pub status: String,
    #[serde(default)]
    pub sprint: Option<String>,
}
