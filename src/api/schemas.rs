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

impl From<crate::commands::new::Created> for CreateResult {
    fn from(c: crate::commands::new::Created) -> Self {
        CreateResult {
            id: c.id,
            name: c.name,
            path: c.path.display().to_string(),
        }
    }
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
    /// Display name of the repository (`brand.name`, else `site.name`).
    pub name: String,
    pub mode: String,
    pub prefixes: PrefixView,
    pub statuses: StatusView,
    /// Path keys set in config.yml, sorted.
    pub configured_entities: Vec<String>,
    /// Entity kinds (plural labels) usable in this repo, in display order.
    pub available_kinds: Vec<String>,
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
//
// List-valued fields (`tags`, `customers`, `projects`, `attendees`, `agents`,
// `depends_on`) accept either a comma-separated string (`"a,b"`) or a JSON
// array of strings (`["a", "b"]`).

/// Deserialize `"a,b"`, `["a", "b"]` or `null` into an optional comma list.
fn comma_list<'de, D>(de: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum CommaList {
        Text(String),
        Items(Vec<String>),
    }
    Ok(match Option::<CommaList>::deserialize(de)? {
        None => None,
        Some(CommaList::Text(s)) => Some(s),
        Some(CommaList::Items(items)) => Some(items.join(",")),
    })
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCustomer {
    pub name: String,
    #[serde(default)]
    pub owner: Option<String>,
    /// Defaults to the first configured customer status.
    #[serde(default)]
    pub status: Option<String>,
    /// Comma-separated string or array of strings.
    #[serde(default, deserialize_with = "comma_list")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProject {
    pub name: String,
    #[serde(default)]
    pub owner: Option<String>,
    /// Defaults to the first configured project status.
    #[serde(default)]
    pub status: Option<String>,
    /// Customer IDs (e.g. `CUST-001`), comma-separated string or array.
    #[serde(default, deserialize_with = "comma_list")]
    pub customers: Option<String>,
    /// Comma-separated string or array of strings.
    #[serde(default, deserialize_with = "comma_list")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMeeting {
    pub title: String,
    /// `YYYY-MM-DD`, defaults to today.
    #[serde(default)]
    pub date: Option<String>,
    /// `HH:MM`, defaults to `10:00`.
    #[serde(default)]
    pub time: Option<String>,
    /// e.g. `30m`, `1h`; defaults to `30m`.
    #[serde(default)]
    pub duration: Option<String>,
    /// Defaults to the first configured meeting status.
    #[serde(default)]
    pub status: Option<String>,
    /// Comma-separated string or array of strings.
    #[serde(default, deserialize_with = "comma_list")]
    pub tags: Option<String>,
    /// Customer IDs, comma-separated string or array.
    #[serde(default, deserialize_with = "comma_list")]
    pub customers: Option<String>,
    /// Project IDs, comma-separated string or array.
    #[serde(default, deserialize_with = "comma_list")]
    pub projects: Option<String>,
    /// Attendee names, comma-separated string or array.
    #[serde(default, deserialize_with = "comma_list")]
    pub attendees: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateResearch {
    pub title: String,
    #[serde(default)]
    pub owner: Option<String>,
    /// Agent names; defaults to `claude,gemini,chatgpt,perplexity`.
    #[serde(default, deserialize_with = "comma_list")]
    pub agents: Option<String>,
    /// Comma-separated string or array of strings.
    #[serde(default, deserialize_with = "comma_list")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTask {
    pub title: String,
    /// Project ID to scope the task to (must exist).
    #[serde(default)]
    pub project: Option<String>,
    /// Customer ID to scope the task to (must exist; ignored when `project` is set).
    #[serde(default)]
    pub customer: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    /// Defaults to the first configured task status.
    #[serde(default)]
    pub status: Option<String>,
    /// 1 (critical) to 4 (low); defaults to 3.
    #[serde(default)]
    pub priority: Option<u32>,
    /// Comma-separated string or array of strings.
    #[serde(default, deserialize_with = "comma_list")]
    pub tags: Option<String>,
    /// Sprint ID, e.g. `SPR-001`.
    #[serde(default)]
    pub sprint: Option<String>,
    /// Task IDs this task depends on, comma-separated string or array.
    #[serde(default, deserialize_with = "comma_list")]
    pub depends_on: Option<String>,
    /// `YYYY-MM-DD`.
    #[serde(default)]
    pub due_date: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSprint {
    pub title: String,
    #[serde(default)]
    pub owner: Option<String>,
    /// Defaults to the first configured sprint status.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    /// `YYYY-MM-DD`, defaults to today.
    #[serde(default)]
    pub start_date: Option<String>,
    /// `YYYY-MM-DD`, must not be before `start_date`.
    #[serde(default)]
    pub end_date: Option<String>,
    /// Project IDs, comma-separated string or array.
    #[serde(default, deserialize_with = "comma_list")]
    pub projects: Option<String>,
    /// Comma-separated string or array of strings.
    #[serde(default, deserialize_with = "comma_list")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProposal {
    pub title: String,
    #[serde(default)]
    pub author: Option<String>,
    /// Defaults to the first configured proposal status.
    #[serde(default)]
    pub status: Option<String>,
    /// `architecture`, `feature` or `process`; defaults to `architecture`.
    #[serde(default, rename = "type")]
    pub proposal_type: Option<String>,
    /// Comma-separated string or array of strings.
    #[serde(default, deserialize_with = "comma_list")]
    pub tags: Option<String>,
    /// ID of the proposal this one supersedes.
    #[serde(default)]
    pub supersedes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateContact {
    pub name: String,
    /// Customer ID the contact belongs to (must exist).
    pub customer: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    /// Defaults to the first configured contact status.
    #[serde(default)]
    pub status: Option<String>,
    /// Comma-separated string or array of strings.
    #[serde(default, deserialize_with = "comma_list")]
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

/// One Markdown checklist item (`- [ ] text`).
#[derive(Debug, Serialize, ToSchema)]
pub struct CheckItemView {
    /// 1-based position in the entity's checklist.
    pub index: usize,
    /// 1-based line in the file.
    pub line: usize,
    pub checked: bool,
    pub text: String,
}

impl From<crate::checklist::CheckItem> for CheckItemView {
    fn from(i: crate::checklist::CheckItem) -> Self {
        CheckItemView {
            index: i.index,
            line: i.line,
            checked: i.checked,
            text: i.text,
        }
    }
}

/// `GET /v1/entities/{kind}/{id}/checklist` payload.
#[derive(Debug, Serialize, ToSchema)]
pub struct ChecklistResponse {
    pub id: String,
    pub items: Vec<CheckItemView>,
    pub done: usize,
    pub total: usize,
}

/// Tick or untick a checklist item.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckItemBody {
    /// New state; defaults to true (ticked).
    #[serde(default)]
    pub checked: Option<bool>,
    /// The item's text as last read; a mismatch is a 409 and nothing changes.
    #[serde(default)]
    pub expect_text: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CheckResult {
    pub id: String,
    pub item: CheckItemView,
    /// False when the item was already in the requested state.
    pub changed: bool,
    pub done: usize,
    pub total: usize,
}

/// Add a comment to a task or meeting.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddCommentBody {
    /// Markdown text (required, non-empty).
    pub text: String,
    /// Author shown in the heading; defaults to the repo's git user.name.
    #[serde(default)]
    pub author: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommentView {
    /// `YYYY-MM-DD`
    pub date: Option<String>,
    /// `HH:MM`
    pub time: Option<String>,
    pub author: Option<String>,
    /// The heading as written.
    pub heading: String,
    /// Markdown body.
    pub body: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommentResult {
    pub id: String,
    pub comment: CommentView,
    /// Comments on the entity after this one was added.
    pub count: usize,
    pub path: String,
}

impl From<crate::comments::Added> for CommentResult {
    fn from(a: crate::comments::Added) -> Self {
        let c = a.comment;
        CommentResult {
            id: a.id,
            comment: CommentView {
                date: c.date,
                time: c.time,
                author: c.author,
                heading: c.heading,
                body: c.body,
            },
            count: a.count,
            path: a.path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_fields_accept_string_array_or_null() {
        let a: CreateTask =
            serde_json::from_value(serde_json::json!({"title": "t", "tags": "a,b"})).unwrap();
        assert_eq!(a.tags.as_deref(), Some("a,b"));
        let b: CreateTask =
            serde_json::from_value(serde_json::json!({"title": "t", "tags": ["a", "b"]})).unwrap();
        assert_eq!(b.tags.as_deref(), Some("a,b"));
        let c: CreateTask =
            serde_json::from_value(serde_json::json!({"title": "t", "tags": null})).unwrap();
        assert_eq!(c.tags, None);
        let d: CreateTask = serde_json::from_value(serde_json::json!({"title": "t"})).unwrap();
        assert_eq!(d.tags, None);
        assert!(
            serde_json::from_value::<CreateTask>(serde_json::json!({"title": "t", "tags": 5}))
                .is_err()
        );
    }
}
