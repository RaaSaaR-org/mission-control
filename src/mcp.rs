use crate::commands;
use crate::config::ResolvedConfig;
use crate::data;
use crate::entity::EntityKind;
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

/// Helper: convert an EntityRecord's frontmatter to JSON, adding _source.
fn entity_to_json(rec: &data::EntityRecord, cfg: &ResolvedConfig) -> JsonValue {
    let mut val = data::yaml_to_json(&rec.frontmatter);
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
    /// Entity kind: customers, projects, meetings, research, or tasks
    #[schemars(description = "Entity kind: customers, projects, meetings, research, or tasks")]
    pub kind: String,
    /// Filter by status (optional)
    #[schemars(description = "Filter by status (e.g. active, draft)")]
    pub status: Option<String>,
    /// Filter by tag (optional)
    #[schemars(description = "Filter by tag")]
    pub tag: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetEntityParams {
    /// Entity ID (e.g. CUST-001, PROJ-002, MTG-003, RES-001, TASK-001)
    #[schemars(description = "Entity ID (e.g. CUST-001, PROJ-002, TASK-001)")]
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadEntityFileParams {
    /// Entity ID whose markdown file to read
    #[schemars(description = "Entity ID whose markdown file to read")]
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateCustomerParams {
    /// Customer name
    #[schemars(description = "Customer name")]
    pub name: String,
    /// Owner (optional)
    #[schemars(description = "Owner")]
    pub owner: Option<String>,
    /// Status (optional, defaults to first configured status)
    #[schemars(description = "Status (defaults to first configured status)")]
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
    #[schemars(description = "Owner")]
    pub owner: Option<String>,
    /// Status (optional)
    #[schemars(description = "Status")]
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
    #[schemars(description = "Status")]
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
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateResearchParams {
    /// Research title
    #[schemars(description = "Research title")]
    pub title: String,
    /// Owner (optional)
    #[schemars(description = "Owner")]
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
    #[schemars(description = "Owner")]
    pub owner: Option<String>,
    /// Status (optional, defaults to first configured status e.g. backlog)
    #[schemars(description = "Status (defaults to first configured status e.g. backlog)")]
    pub status: Option<String>,
    /// Priority 1-4 (1=critical, 2=high, 3=medium, 4=low; defaults to 3)
    #[schemars(description = "Priority 1-4 (1=critical, 2=high, 3=medium, 4=low; defaults to 3)")]
    pub priority: Option<u32>,
    /// Comma-separated tags (optional)
    #[schemars(description = "Comma-separated tags")]
    pub tags: Option<String>,
    /// Sprint name (optional)
    #[schemars(description = "Sprint name")]
    pub sprint: Option<String>,
    /// Comma-separated task IDs this depends on (optional)
    #[schemars(description = "Comma-separated task IDs this depends on")]
    pub depends_on: Option<String>,
    /// Due date YYYY-MM-DD (optional)
    #[schemars(description = "Due date YYYY-MM-DD")]
    pub due_date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateSprintParams {
    /// Sprint title (e.g. "2026-W05")
    #[schemars(description = "Sprint title")]
    pub title: String,
    /// Owner (optional)
    #[schemars(description = "Owner")]
    pub owner: Option<String>,
    /// Status (optional, defaults to planning)
    #[schemars(description = "Status (defaults to planning)")]
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
pub struct MoveTaskParams {
    /// Task ID (e.g. TASK-001)
    #[schemars(description = "Task ID (e.g. TASK-001)")]
    pub id: String,
    /// Target status (e.g. todo, in-progress, done)
    #[schemars(description = "Target status (e.g. todo, in-progress, done)")]
    pub status: String,
    /// Sprint name (optional, updates sprint field)
    #[schemars(description = "Sprint name (optional, updates sprint field)")]
    pub sprint: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListTasksParams {
    /// Filter by status (optional)
    #[schemars(description = "Filter by status (e.g. backlog, todo, in-progress, done)")]
    pub status: Option<String>,
    /// Filter by tag (optional)
    #[schemars(description = "Filter by tag")]
    pub tag: Option<String>,
    /// Filter by project ID (optional, e.g. PROJ-001)
    #[schemars(description = "Filter by project ID (e.g. PROJ-001)")]
    pub project: Option<String>,
    /// Filter by customer ID (optional, e.g. CUST-001)
    #[schemars(description = "Filter by customer ID (e.g. CUST-001)")]
    pub customer: Option<String>,
    /// Filter by priority 1-4 (optional)
    #[schemars(description = "Filter by priority 1-4")]
    pub priority: Option<u32>,
    /// Filter by sprint name (optional)
    #[schemars(description = "Filter by sprint name")]
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
    /// Output file path (optional, defaults to {ID}.pdf)
    #[schemars(description = "Output file path (optional)")]
    pub output: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrintResearchParams {
    /// Research ID (e.g. RES-001)
    #[schemars(description = "Research ID (e.g. RES-001)")]
    pub id: String,
    /// Output file path (optional, defaults to {ID}-final-report.pdf)
    #[schemars(description = "Output file path (optional)")]
    pub output: Option<String>,
    /// Specific file from final/ directory (optional)
    #[schemars(description = "Specific file from final/ (optional)")]
    pub file: Option<String>,
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
    #[tool(description = "List entities of a given kind with optional status/tag filters")]
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

    #[tool(description = "Get detailed information about an entity by its ID")]
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

    #[tool(description = "Read the full markdown file content for an entity")]
    async fn read_entity_file(
        &self,
        Parameters(params): Parameters<ReadEntityFileParams>,
    ) -> Result<CallToolResult, McpError> {
        let rec = data::find_entity_by_id(&params.id, &self.cfg).map_err(mc_err)?;
        let content = std::fs::read_to_string(&rec.source_path).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(content)]))
    }

    #[tool(description = "Create a new customer")]
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

    #[tool(description = "Create a new project")]
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

    #[tool(description = "Create a new meeting")]
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
        )
        .map_err(mc_err)?;
        let text = serde_json::to_string_pretty(&result).map_err(mc_err)?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Create a new research topic")]
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

    #[tool(description = "Create a new task")]
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

    #[tool(description = "Create a new sprint")]
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

    #[tool(description = "Move a task to a new status (and optionally update its sprint)")]
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
        description = "List tasks with rich filtering (project, customer, priority, sprint, owner, status, tag)"
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

    #[tool(description = "Export a meeting to PDF")]
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

    #[tool(description = "Export a research topic to PDF")]
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

    #[tool(description = "Validate the repo structure, naming conventions, and frontmatter")]
    async fn validate_repo(&self) -> Result<CallToolResult, McpError> {
        let issues = commands::validate::validate_programmatic(&self.cfg).map_err(mc_err)?;
        let text = if issues.is_empty() {
            "Validation passed: no issues found.".to_string()
        } else {
            serde_json::to_string_pretty(&issues).map_err(mc_err)?
        };
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Rebuild the JSON index files in data/")]
    async fn build_index(&self) -> Result<CallToolResult, McpError> {
        let result = commands::index::run_quiet(&self.cfg).map_err(mc_err)?;
        let text = format!(
            "Index built: {} customers, {} projects, {} meetings, {} research, {} tasks, {} sprints",
            result.customers, result.projects, result.meetings, result.research, result.tasks, result.sprints,
        );
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Get a status overview with entity counts and recent activity")]
    async fn get_status(&self) -> Result<CallToolResult, McpError> {
        let kinds = [
            EntityKind::Customer,
            EntityKind::Project,
            EntityKind::Meeting,
            EntityKind::Research,
            EntityKind::Task,
            EntityKind::Sprint,
        ];

        let mut counts = serde_json::Map::new();
        for kind in &kinds {
            let sc = data::count_by_status(*kind, &self.cfg).map_err(mc_err)?;
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
        ServerInfo {
            instructions: Some(
                "MissionControl MCP server. Manage customers, projects, meetings, research, and tasks in a git-based knowledge repository."
                    .into(),
            ),
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
        let resources = vec![
            Annotated::new(RawResource::new("mc://config", "config"), None),
            Annotated::new(
                RawResource::new("mc://entities/customers", "customers"),
                None,
            ),
            Annotated::new(RawResource::new("mc://entities/projects", "projects"), None),
            Annotated::new(RawResource::new("mc://entities/meetings", "meetings"), None),
            Annotated::new(RawResource::new("mc://entities/research", "research"), None),
            Annotated::new(RawResource::new("mc://entities/tasks", "tasks"), None),
            Annotated::new(RawResource::new("mc://entities/sprints", "sprints"), None),
        ];

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
                    },
                    "statuses": {
                        "customer": &self.cfg.statuses.customer,
                        "project": &self.cfg.statuses.project,
                        "meeting": &self.cfg.statuses.meeting,
                        "research": &self.cfg.statuses.research,
                        "task": &self.cfg.statuses.task,
                        "sprint": &self.cfg.statuses.sprint,
                    },
                    "paths": {
                        "customers": self.cfg.customers_dir.display().to_string(),
                        "projects": self.cfg.projects_dir.display().to_string(),
                        "meetings": self.cfg.meetings_dir.display().to_string(),
                        "research": self.cfg.research_dir.display().to_string(),
                        "tasks": self.cfg.tasks_dir.display().to_string(),
                        "sprints": self.cfg.sprints_dir.display().to_string(),
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
