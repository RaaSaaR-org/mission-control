use crate::commands;
use crate::config::{RepoMode, ResolvedConfig};
use crate::data;
use crate::entity::EntityKind;
use crate::frontmatter;
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

/// Fields that may contain wiki-link brackets and should be stripped for JSON output.
const WIKILINK_FIELDS: &[&str] = &[
    "customers",
    "projects",
    "depends_on",
    "sprint",
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

/// Helper: convert an EntityRecord's frontmatter to JSON, adding _source.
fn entity_to_json(rec: &data::EntityRecord, cfg: &ResolvedConfig) -> JsonValue {
    let mut val = data::yaml_to_json(&rec.frontmatter);
    strip_wikilinks_in_json(&mut val);
    if let Some(obj) = val.as_object_mut() {
        let rel = rec
            .source_path
            .strip_prefix(&cfg.root)
            .unwrap_or(&rec.source_path)
            .to_string_lossy()
            .to_string();
        obj.insert("_source".into(), JsonValue::String(rel));
    }
    val
}

/// Helper: convert our McError into rmcp McpError.
fn mc_err(e: impl std::fmt::Display) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListEntitiesParams {
    /// Entity kind: customers, contacts, projects, meetings, research, tasks, sprints, or proposals
    #[schemars(
        description = "Entity kind: customers, contacts, projects, meetings, research, tasks, sprints, or proposals"
    )]
    pub kind: String,
    /// Filter by status (optional)
    #[schemars(
        description = "Filter by status (valid values depend on entity kind — read mc://config)"
    )]
    pub status: Option<String>,
    /// Filter by tag (optional)
    #[schemars(description = "Filter by tag (exact match)")]
    pub tag: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetEntityParams {
    /// Entity ID (e.g. CUST-001, CONT-001, PROJ-001, MTG-001, RES-001, TASK-001, SPR-001, PROP-001)
    #[schemars(
        description = "Entity ID (e.g. CUST-001, CONT-001, PROJ-001, MTG-001, RES-001, TASK-001, SPR-001, PROP-001)"
    )]
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadEntityFileParams {
    /// Entity ID (e.g. CUST-001, TASK-001)
    #[schemars(description = "Entity ID (e.g. CUST-001, TASK-001)")]
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateCustomerParams {
    /// Customer name
    #[schemars(description = "Customer name")]
    pub name: String,
    /// Owner (optional)
    #[schemars(description = "Owner (username or name)")]
    pub owner: Option<String>,
    /// Status (optional, defaults to active)
    #[schemars(description = "Status (default: active; values: active, inactive, prospect)")]
    pub status: Option<String>,
    /// Comma-separated tags (optional)
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateProjectParams {
    /// Project name
    #[schemars(description = "Project name")]
    pub name: String,
    /// Owner (optional)
    #[schemars(description = "Owner (username or name)")]
    pub owner: Option<String>,
    /// Status (optional)
    #[schemars(description = "Status (default: active; values: active, on-hold, completed)")]
    pub status: Option<String>,
    /// Linked customer IDs, comma-separated (optional)
    #[schemars(description = "Linked customer IDs, comma-separated")]
    pub customers: Option<String>,
    /// Comma-separated tags (optional)
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateMeetingParams {
    /// Meeting title
    #[schemars(description = "Meeting title")]
    pub title: String,
    /// Date YYYY-MM-DD (optional, defaults to today)
    #[schemars(description = "Date YYYY-MM-DD (defaults to today)")]
    pub date: Option<String>,
    /// Time HH:MM (optional, defaults to 10:00)
    #[schemars(description = "Time HH:MM (defaults to 10:00)")]
    pub time: Option<String>,
    /// Duration e.g. 30m, 1h (optional, defaults to 30m)
    #[schemars(description = "Duration e.g. 30m, 1h (defaults to 30m)")]
    pub duration: Option<String>,
    /// Status (optional)
    #[schemars(description = "Status (default: scheduled; values: scheduled, completed)")]
    pub status: Option<String>,
    /// Comma-separated tags (optional)
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
    /// Linked customer IDs, comma-separated (optional)
    #[schemars(description = "Linked customer IDs, comma-separated")]
    pub customers: Option<String>,
    /// Linked project IDs, comma-separated (optional)
    #[schemars(description = "Linked project IDs, comma-separated")]
    pub projects: Option<String>,
    /// Comma-separated attendee names (optional)
    #[schemars(description = "Comma-separated attendee names")]
    pub attendees: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateResearchParams {
    /// Research title
    #[schemars(description = "Research title")]
    pub title: String,
    /// Owner (optional)
    #[schemars(description = "Owner (username or name)")]
    pub owner: Option<String>,
    /// Comma-separated agent names (optional, defaults to claude,gemini,chatgpt,perplexity)
    #[schemars(
        description = "Comma-separated agent names (defaults to claude,gemini,chatgpt,perplexity)"
    )]
    pub agents: Option<String>,
    /// Comma-separated tags (optional)
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateTaskParams {
    /// Task title
    #[schemars(description = "Task title")]
    pub title: String,
    /// Scope to a project (e.g. PROJ-001)
    #[schemars(description = "Scope to a project (e.g. PROJ-001)")]
    pub project: Option<String>,
    /// Scope to a customer (e.g. CUST-001)
    #[schemars(description = "Scope to a customer (e.g. CUST-001)")]
    pub customer: Option<String>,
    /// Owner (optional)
    #[schemars(description = "Owner (username or name)")]
    pub owner: Option<String>,
    /// Status (optional, defaults to backlog)
    #[schemars(
        description = "Status (default: backlog; values: backlog, todo, in-progress, review, done, cancelled)"
    )]
    pub status: Option<String>,
    /// Priority 1-4 (1=critical, 2=high, 3=medium, 4=low; defaults to 3)
    #[schemars(description = "Priority 1-4 (1=critical, 2=high, 3=medium, 4=low; defaults to 3)")]
    pub priority: Option<u32>,
    /// Comma-separated tags (optional)
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
    /// Sprint label (optional)
    #[schemars(description = "Sprint label (e.g. 2026-W05)")]
    pub sprint: Option<String>,
    /// Comma-separated task IDs this depends on (optional)
    #[schemars(description = "Comma-separated task IDs this depends on (e.g. TASK-001,TASK-002)")]
    pub depends_on: Option<String>,
    /// Due date YYYY-MM-DD (optional)
    #[schemars(description = "Due date YYYY-MM-DD")]
    pub due_date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateSprintParams {
    /// Sprint title (e.g. "2026-W05")
    #[schemars(description = "Sprint title (e.g. 2026-W05)")]
    pub title: String,
    /// Owner (optional)
    #[schemars(description = "Owner (username or name)")]
    pub owner: Option<String>,
    /// Status (optional, defaults to planning)
    #[schemars(
        description = "Status (default: planning; values: planning, active, review, completed, cancelled)"
    )]
    pub status: Option<String>,
    /// Sprint goal (optional)
    #[schemars(description = "Sprint goal")]
    pub goal: Option<String>,
    /// Start date YYYY-MM-DD (optional, defaults to today)
    #[schemars(description = "Start date YYYY-MM-DD (defaults to today)")]
    pub start_date: Option<String>,
    /// End date YYYY-MM-DD (optional)
    #[schemars(description = "End date YYYY-MM-DD")]
    pub end_date: Option<String>,
    /// Linked project IDs, comma-separated (optional)
    #[schemars(description = "Linked project IDs, comma-separated")]
    pub projects: Option<String>,
    /// Comma-separated tags (optional)
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateProposalParams {
    /// Proposal title
    #[schemars(description = "Proposal title")]
    pub title: String,
    /// Author (optional)
    #[schemars(description = "Author (username or name)")]
    pub author: Option<String>,
    /// Status (optional, defaults to draft)
    #[schemars(
        description = "Status (default: draft; values: draft, proposed, accepted, rejected, superseded, withdrawn)"
    )]
    pub status: Option<String>,
    /// Proposal type: architecture, feature, or process (optional, defaults to architecture)
    #[schemars(
        description = "Proposal type: architecture, feature, or process (defaults to architecture)"
    )]
    pub proposal_type: Option<String>,
    /// Comma-separated tags (optional)
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
    /// ID of proposal this supersedes (optional, e.g. PROP-001)
    #[schemars(description = "ID of proposal this supersedes (e.g. PROP-001)")]
    pub supersedes: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateContactParams {
    /// Contact full name
    #[schemars(description = "Contact full name")]
    pub name: String,
    /// Customer ID (required, e.g. CUST-001)
    #[schemars(description = "Customer ID (required, e.g. CUST-001)")]
    pub customer: String,
    /// Role / job title (optional)
    #[schemars(description = "Role or job title")]
    pub role: Option<String>,
    /// Email address (optional)
    #[schemars(description = "Email address")]
    pub email: Option<String>,
    /// Phone number (optional)
    #[schemars(description = "Phone number")]
    pub phone: Option<String>,
    /// Status (optional, defaults to active)
    #[schemars(description = "Status (default: active; values: active, inactive)")]
    pub status: Option<String>,
    /// Comma-separated tags (optional)
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MoveTaskParams {
    /// Task ID (e.g. TASK-001)
    #[schemars(description = "Task ID (e.g. TASK-001)")]
    pub id: String,
    /// Target status
    #[schemars(
        description = "Target status (values: backlog, todo, in-progress, review, done, cancelled)"
    )]
    pub status: String,
    /// Sprint label to assign (optional)
    #[schemars(description = "Sprint label to assign (e.g. 2026-W05)")]
    pub sprint: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListTasksParams {
    /// Filter by status (optional)
    #[schemars(
        description = "Filter by status (values: backlog, todo, in-progress, review, done, cancelled)"
    )]
    pub status: Option<String>,
    /// Filter by tag (optional)
    #[schemars(description = "Filter by tag (exact match)")]
    pub tag: Option<String>,
    /// Filter by project ID (optional, e.g. PROJ-001)
    #[schemars(description = "Filter by project ID (e.g. PROJ-001)")]
    pub project: Option<String>,
    /// Filter by customer ID (optional, e.g. CUST-001)
    #[schemars(description = "Filter by customer ID (e.g. CUST-001)")]
    pub customer: Option<String>,
    /// Filter by priority 1-4 (optional)
    #[schemars(description = "Filter by priority 1-4 (1=critical, 2=high, 3=medium, 4=low)")]
    pub priority: Option<u32>,
    /// Filter by sprint label (optional)
    #[schemars(description = "Filter by sprint label (e.g. 2026-W05)")]
    pub sprint: Option<String>,
    /// Filter by owner (optional)
    #[schemars(description = "Filter by owner")]
    pub owner: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrintMeetingParams {
    /// Meeting ID (e.g. MTG-001)
    #[schemars(description = "Meeting ID (e.g. MTG-001)")]
    pub id: String,
    /// Output file path (optional)
    #[schemars(description = "Output file path (defaults to {id}.pdf)")]
    pub output: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrintResearchParams {
    /// Research ID (e.g. RES-001)
    #[schemars(description = "Research ID (e.g. RES-001)")]
    pub id: String,
    /// Output file path (optional)
    #[schemars(description = "Output file path (defaults to {id}-final-report.pdf)")]
    pub output: Option<String>,
    /// Filename from the research final/ directory (optional)
    #[schemars(description = "Filename from the research final/ directory (optional)")]
    pub file: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrintFileParams {
    /// Path to markdown file
    #[schemars(description = "Path to markdown file (relative to repo root)")]
    pub path: String,
    /// Output PDF path (optional)
    #[schemars(description = "Output PDF path (defaults to <filename>.pdf)")]
    pub output: Option<String>,
    /// Cover page template: standard, meeting, research, sprint (optional, defaults to standard)
    #[schemars(
        description = "Cover page template: standard, meeting, research, sprint (defaults to standard)"
    )]
    pub template: Option<String>,
    /// Override document title (optional)
    #[schemars(description = "Override document title (auto-detected from first H1 or filename)")]
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
}

#[tool_router]
impl McServer {
    #[tool(
        description = "List entities of a given kind with optional status/tag filters. Returns JSON array of entity objects. For tasks, prefer list_tasks which supports richer filters."
    )]
    async fn list_entities(
        &self,
        Parameters(params): Parameters<ListEntitiesParams>,
    ) -> Result<CallToolResult, McpError> {
        let kind = EntityKind::from_str_loose(&params.kind).map_err(mc_err)?;
        let entities = data::collect_filtered(
            kind,
            &self.cfg,
            params.status.as_deref(),
            params.tag.as_deref(),
        )
        .map_err(mc_err)?;

        let json: Vec<JsonValue> = entities
            .iter()
            .map(|e| entity_to_json(e, &self.cfg))
            .collect();
        let text = serde_json::to_string_pretty(&json).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Get detailed information about an entity by its ID. Returns JSON object with frontmatter fields and _body_preview (first 500 chars of markdown body)."
    )]
    async fn get_entity(
        &self,
        Parameters(params): Parameters<GetEntityParams>,
    ) -> Result<CallToolResult, McpError> {
        let rec = data::find_entity_by_id(&params.id, &self.cfg).map_err(mc_err)?;
        let mut json = entity_to_json(&rec, &self.cfg);

        if let Some(obj) = json.as_object_mut() {
            let preview: String = rec.body.chars().take(500).collect();
            obj.insert("_body_preview".into(), JsonValue::String(preview));
        }

        let text = serde_json::to_string_pretty(&json).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Read the full markdown content of an entity file (YAML frontmatter + body). Use this instead of get_entity when you need the complete document text."
    )]
    async fn read_entity_file(
        &self,
        Parameters(params): Parameters<ReadEntityFileParams>,
    ) -> Result<CallToolResult, McpError> {
        let rec = data::find_entity_by_id(&params.id, &self.cfg).map_err(mc_err)?;
        let content = std::fs::read_to_string(&rec.source_path).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(content)]))
    }

    #[tool(
        description = "Create a new customer (standalone mode only). Returns JSON with id, name, and path."
    )]
    async fn create_customer(
        &self,
        Parameters(params): Parameters<CreateCustomerParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = commands::new::create_customer_programmatic(
            &self.cfg,
            &params.name,
            params.owner.as_deref(),
            params.status.as_deref(),
            params.tags.as_deref(),
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Create a new project (standalone mode only). Returns JSON with id, name, and path."
    )]
    async fn create_project(
        &self,
        Parameters(params): Parameters<CreateProjectParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = commands::new::create_project_programmatic(
            &self.cfg,
            &params.name,
            params.owner.as_deref(),
            params.status.as_deref(),
            params.customers.as_deref(),
            params.tags.as_deref(),
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Create a new meeting. Returns JSON with id, title, and path.")]
    async fn create_meeting(
        &self,
        Parameters(params): Parameters<CreateMeetingParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = commands::new::create_meeting_programmatic(
            &self.cfg,
            &params.title,
            params.date.as_deref(),
            params.time.as_deref(),
            params.duration.as_deref(),
            params.status.as_deref(),
            params.tags.as_deref(),
            params.customers.as_deref(),
            params.projects.as_deref(),
            params.attendees.as_deref(),
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Create a new research topic. Status is always 'draft'. Returns JSON with id, title, and path."
    )]
    async fn create_research(
        &self,
        Parameters(params): Parameters<CreateResearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = commands::new::create_research_programmatic(
            &self.cfg,
            &params.title,
            params.owner.as_deref(),
            params.agents.as_deref(),
            params.tags.as_deref(),
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Create a new task. Returns JSON with id, title, and path.")]
    async fn create_task(
        &self,
        Parameters(params): Parameters<CreateTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = commands::new::create_task_programmatic(
            &self.cfg,
            &params.title,
            params.project.as_deref(),
            params.customer.as_deref(),
            params.owner.as_deref(),
            params.status.as_deref(),
            params.priority,
            params.tags.as_deref(),
            params.sprint.as_deref(),
            params.depends_on.as_deref(),
            params.due_date.as_deref(),
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Create a new sprint. Returns JSON with id, title, and path.")]
    async fn create_sprint(
        &self,
        Parameters(params): Parameters<CreateSprintParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = commands::new::create_sprint_programmatic(
            &self.cfg,
            &params.title,
            params.owner.as_deref(),
            params.status.as_deref(),
            params.goal.as_deref(),
            params.start_date.as_deref(),
            params.end_date.as_deref(),
            params.projects.as_deref(),
            params.tags.as_deref(),
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Create a new proposal (standalone mode only). Returns JSON with id, title, and path."
    )]
    async fn create_proposal(
        &self,
        Parameters(params): Parameters<CreateProposalParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = commands::new::create_proposal_programmatic(
            &self.cfg,
            &params.title,
            params.author.as_deref(),
            params.status.as_deref(),
            params.proposal_type.as_deref(),
            params.tags.as_deref(),
            params.supersedes.as_deref(),
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Create a new contact under a customer (standalone mode only)")]
    async fn create_contact(
        &self,
        Parameters(params): Parameters<CreateContactParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = commands::new::create_contact_programmatic(
            &self.cfg,
            &params.name,
            &params.customer,
            params.role.as_deref(),
            params.email.as_deref(),
            params.phone.as_deref(),
            params.status.as_deref(),
            params.tags.as_deref(),
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Move a task to a new status (and optionally assign a sprint). Returns updated task as JSON."
    )]
    async fn move_task(
        &self,
        Parameters(params): Parameters<MoveTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = commands::task::move_task_programmatic(
            &self.cfg,
            &params.id,
            &params.status,
            params.sprint.as_deref(),
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "List tasks with rich filtering. Returns JSON array. Use this instead of list_entities for tasks."
    )]
    async fn list_tasks(
        &self,
        Parameters(params): Parameters<ListTasksParams>,
    ) -> Result<CallToolResult, McpError> {
        let filter = data::TaskFilter {
            status: params.status.as_deref(),
            tag: params.tag.as_deref(),
            project: params.project.as_deref(),
            customer: params.customer.as_deref(),
            priority: params.priority,
            sprint: params.sprint.as_deref(),
            owner: params.owner.as_deref(),
        };
        let tasks = data::collect_tasks_filtered(&self.cfg, &filter).map_err(mc_err)?;
        let json: Vec<JsonValue> = tasks.iter().map(|e| entity_to_json(e, &self.cfg)).collect();
        let text = serde_json::to_string_pretty(&json).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Export a meeting to PDF. Returns JSON with the output file path.")]
    async fn print_meeting(
        &self,
        Parameters(params): Parameters<PrintMeetingParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = commands::print::print_meeting_programmatic(
            &self.cfg,
            &params.id,
            params.output.as_deref(),
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Export a research final report to PDF. Returns JSON with the output file path."
    )]
    async fn print_research(
        &self,
        Parameters(params): Parameters<PrintResearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = commands::print::print_research_programmatic(
            &self.cfg,
            &params.id,
            params.output.as_deref(),
            params.file.as_deref(),
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Generate a branded PDF from any markdown file. Returns JSON with the output file path."
    )]
    async fn print_file(
        &self,
        Parameters(params): Parameters<PrintFileParams>,
    ) -> Result<CallToolResult, McpError> {
        use crate::cli::PrintTemplate;
        let template = match params.template.as_deref() {
            Some("meeting") => PrintTemplate::Meeting,
            Some("research") => PrintTemplate::Research,
            Some("sprint") => PrintTemplate::Sprint,
            _ => PrintTemplate::Standard,
        };
        let result = commands::print::print_file_programmatic(
            &self.cfg,
            &params.path,
            params.output.as_deref(),
            &template,
            params.title.as_deref(),
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Validate repo structure and frontmatter. Returns 'Validation passed: no issues found.' or a JSON array of issue objects."
    )]
    async fn validate_repo(&self) -> Result<CallToolResult, McpError> {
        let issues = commands::validate::validate_programmatic(&self.cfg).map_err(mc_err)?;
        let text = if issues.is_empty() {
            "Validation passed: no issues found.".to_string()
        } else {
            serde_json::to_string_pretty(&issues).map_err(mc_err)?
        };
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Rebuild JSON index files in data/ for the web dashboard. Returns a summary string with entity counts."
    )]
    async fn build_index(&self) -> Result<CallToolResult, McpError> {
        let result = commands::index::run_quiet(&self.cfg).map_err(mc_err)?;
        let text = format!(
            "Index built: {} customers, {} projects, {} meetings, {} research, {} tasks, {} sprints, {} proposals, {} contacts",
            result.customers, result.projects, result.meetings, result.research, result.tasks, result.sprints, result.proposals, result.contacts,
        );
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Get a status overview. Returns JSON with 'counts' (per-entity-kind totals and by_status breakdowns) and 'recent_activity' (last 10 modified entities)."
    )]
    async fn get_status(&self) -> Result<CallToolResult, McpError> {
        let all_kinds = [
            EntityKind::Customer,
            EntityKind::Project,
            EntityKind::Meeting,
            EntityKind::Research,
            EntityKind::Task,
            EntityKind::Sprint,
            EntityKind::Proposal,
            EntityKind::Contact,
        ];

        let mut counts = serde_json::Map::new();
        let kinds: Vec<_> = all_kinds
            .iter()
            .filter(|k| self.cfg.entity_available(k))
            .collect();
        for kind in &kinds {
            let sc = data::count_by_status(**kind, &self.cfg).map_err(mc_err)?;
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

        let recent = data::recent_activity(&self.cfg, 10).map_err(mc_err)?;
        let recent_json: Vec<JsonValue> = recent
            .iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id,
                    "name": f.name,
                    "path": f.path.display().to_string(),
                })
            })
            .collect();

        let result = serde_json::json!({
            "counts": counts,
            "recent_activity": recent_json,
        });

        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
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
        let entities = match self.cfg.mode {
            RepoMode::Standalone => {
                "customers, projects, contacts, meetings, research, sprints, proposals, and tasks"
            }
            RepoMode::Embedded => "meetings, research, sprints, proposals, and tasks",
        };
        let instructions = format!(
            "MissionControl ({}) at {}. Manage {} in a git-based knowledge repository.\n\nStart with get_status for an overview. Read the mc://config resource for valid status values and ID prefixes. Use list_tasks (not list_entities) for task queries — it supports richer filters.",
            mode_label,
            self.cfg.root.display(),
            entities,
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
            "Repository configuration: valid status values, ID prefixes, and directory paths for each entity kind",
        )];

        if self.cfg.mode == RepoMode::Standalone {
            resources.push(described_resource(
                "mc://entities/customers",
                "customers",
                "All customers as a JSON array",
            ));
            resources.push(described_resource(
                "mc://entities/projects",
                "projects",
                "All projects as a JSON array",
            ));
        }

        resources.push(described_resource(
            "mc://entities/meetings",
            "meetings",
            "All meetings as a JSON array",
        ));
        resources.push(described_resource(
            "mc://entities/research",
            "research",
            "All research topics as a JSON array",
        ));
        resources.push(described_resource(
            "mc://entities/tasks",
            "tasks",
            "All tasks as a JSON array (unfiltered — use list_tasks tool for filtering)",
        ));
        resources.push(described_resource(
            "mc://entities/sprints",
            "sprints",
            "All sprints as a JSON array",
        ));
        resources.push(described_resource(
            "mc://entities/proposals",
            "proposals",
            "All proposals as a JSON array",
        ));
        resources.push(described_resource(
            "mc://entities/contacts",
            "contacts",
            "All contacts as a JSON array",
        ));

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

        let text = match uri.as_str() {
            "mc://config" => {
                let config_json = serde_json::json!({
                    "id_prefixes": {
                        "customer": &self.cfg.id_prefixes.customer,
                        "project": &self.cfg.id_prefixes.project,
                        "meeting": &self.cfg.id_prefixes.meeting,
                        "research": &self.cfg.id_prefixes.research,
                        "task": &self.cfg.id_prefixes.task,
                        "sprint": &self.cfg.id_prefixes.sprint,
                        "proposal": &self.cfg.id_prefixes.proposal,
                        "contact": &self.cfg.id_prefixes.contact,
                    },
                    "statuses": {
                        "customer": &self.cfg.statuses.customer,
                        "project": &self.cfg.statuses.project,
                        "meeting": &self.cfg.statuses.meeting,
                        "research": &self.cfg.statuses.research,
                        "task": &self.cfg.statuses.task,
                        "sprint": &self.cfg.statuses.sprint,
                        "proposal": &self.cfg.statuses.proposal,
                        "contact": &self.cfg.statuses.contact,
                    },
                    "paths": {
                        "customers": self.cfg.customers_dir.display().to_string(),
                        "projects": self.cfg.projects_dir.display().to_string(),
                        "meetings": self.cfg.meetings_dir.display().to_string(),
                        "research": self.cfg.research_dir.display().to_string(),
                        "tasks": self.cfg.tasks_dir.display().to_string(),
                        "sprints": self.cfg.sprints_dir.display().to_string(),
                        "proposals": self.cfg.proposals_dir.display().to_string(),
                    },
                });
                serde_json::to_string_pretty(&config_json).map_err(mc_err)?
            }
            "mc://entities/customers" => {
                let entities = collect_entity_json(EntityKind::Customer, &self.cfg)?;
                serde_json::to_string_pretty(&entities).map_err(mc_err)?
            }
            "mc://entities/projects" => {
                let entities = collect_entity_json(EntityKind::Project, &self.cfg)?;
                serde_json::to_string_pretty(&entities).map_err(mc_err)?
            }
            "mc://entities/meetings" => {
                let entities = collect_entity_json(EntityKind::Meeting, &self.cfg)?;
                serde_json::to_string_pretty(&entities).map_err(mc_err)?
            }
            "mc://entities/research" => {
                let entities = collect_entity_json(EntityKind::Research, &self.cfg)?;
                serde_json::to_string_pretty(&entities).map_err(mc_err)?
            }
            "mc://entities/tasks" => {
                let entities = collect_entity_json(EntityKind::Task, &self.cfg)?;
                serde_json::to_string_pretty(&entities).map_err(mc_err)?
            }
            "mc://entities/sprints" => {
                let entities = collect_entity_json(EntityKind::Sprint, &self.cfg)?;
                serde_json::to_string_pretty(&entities).map_err(mc_err)?
            }
            "mc://entities/proposals" => {
                let entities = collect_entity_json(EntityKind::Proposal, &self.cfg)?;
                serde_json::to_string_pretty(&entities).map_err(mc_err)?
            }
            "mc://entities/contacts" => {
                let entities = collect_entity_json(EntityKind::Contact, &self.cfg)?;
                serde_json::to_string_pretty(&entities).map_err(mc_err)?
            }
            _ => {
                return Err(McpError::resource_not_found(
                    format!("Unknown resource URI: {}", uri),
                    None,
                ));
            }
        };

        Ok(ReadResourceResult {
            contents: vec![ResourceContents::text(text, uri.clone())],
        })
    }
}

fn collect_entity_json(kind: EntityKind, cfg: &ResolvedConfig) -> Result<Vec<JsonValue>, McpError> {
    let entities = data::collect_entities(kind, cfg).map_err(mc_err)?;
    Ok(entities.iter().map(|e| entity_to_json(e, cfg)).collect())
}
