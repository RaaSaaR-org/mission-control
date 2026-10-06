//! `mc new <kind>`: entity creation.
//!
//! Every kind has an input struct (`CustomerInput`, `TaskInput`, ...) and a
//! `create_*` function that validates it, renders the template and writes the
//! files. The interactive CLI (`run`), the MCP server and the REST API all go
//! through those functions; the `create_*_programmatic` wrappers keep the
//! older string-based signatures working.

use crate::cli::ui;
use crate::cli::NewEntity;
use crate::config::ResolvedConfig;
use crate::entity::{self, EntityKind};
use crate::error::{McError, McResult};
use crate::frontmatter;
use crate::template;
use crate::util;
use colored::*;
use serde_json::Value as JsonValue;
use serde_yaml::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Default AI agents for research topics.
pub const DEFAULT_RESEARCH_AGENTS: [&str; 4] = ["claude", "gemini", "chatgpt", "perplexity"];
/// Proposal types offered by the interactive prompt.
pub const PROPOSAL_TYPES: [&str; 3] = ["architecture", "feature", "process"];
/// Task priority used when none is given (3 = medium).
pub const DEFAULT_PRIORITY: u32 = 3;

// ---------------------------------------------------------------------------
// Inputs and results
// ---------------------------------------------------------------------------

/// Fields for a new customer. `None` / empty values fall back to defaults.
#[derive(Debug, Clone, Default)]
pub struct CustomerInput {
    pub name: String,
    pub owner: Option<String>,
    pub status: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectInput {
    pub name: String,
    pub owner: Option<String>,
    pub status: Option<String>,
    /// Customer IDs (plain or `[[wiki-linked]]`).
    pub customers: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MeetingInput {
    pub title: String,
    /// `YYYY-MM-DD`, defaults to today.
    pub date: Option<String>,
    /// `HH:MM`, defaults to `10:00`.
    pub time: Option<String>,
    /// e.g. `30m`, defaults to `30m`.
    pub duration: Option<String>,
    pub status: Option<String>,
    pub tags: Vec<String>,
    pub customers: Vec<String>,
    pub projects: Vec<String>,
    pub attendees: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ResearchInput {
    pub title: String,
    pub owner: Option<String>,
    /// `None` uses [`DEFAULT_RESEARCH_AGENTS`]; `Some(vec![])` means no agents.
    pub agents: Option<Vec<String>>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TaskInput {
    pub title: String,
    /// Scope the task to a project (stored under `projects/<PROJ>/tasks/`).
    pub project: Option<String>,
    /// Scope the task to a customer (used when no project is given).
    pub customer: Option<String>,
    pub owner: Option<String>,
    pub status: Option<String>,
    /// 1 (critical) to 4 (low), defaults to [`DEFAULT_PRIORITY`].
    pub priority: Option<u32>,
    pub tags: Vec<String>,
    /// Sprint ID, e.g. `SPR-001`.
    pub sprint: Option<String>,
    pub depends_on: Vec<String>,
    /// `YYYY-MM-DD`.
    pub due_date: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SprintInput {
    pub title: String,
    pub owner: Option<String>,
    pub status: Option<String>,
    pub goal: Option<String>,
    /// `YYYY-MM-DD`, defaults to today.
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub projects: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ProposalInput {
    pub title: String,
    pub author: Option<String>,
    pub status: Option<String>,
    /// Defaults to `architecture`.
    pub proposal_type: Option<String>,
    pub tags: Vec<String>,
    /// ID of the proposal this one supersedes.
    pub supersedes: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ContactInput {
    pub name: String,
    /// Customer ID the contact belongs to (required, must exist).
    pub customer: String,
    pub role: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub status: Option<String>,
    pub tags: Vec<String>,
}

macro_rules! titled_input {
    ($($ty:ident . $field:ident),* $(,)?) => {$(
        impl $ty {
            pub fn new(text: impl Into<String>) -> Self {
                Self { $field: text.into(), ..Default::default() }
            }
        }
    )*};
}
titled_input!(
    CustomerInput.name,
    ProjectInput.name,
    MeetingInput.title,
    ResearchInput.title,
    TaskInput.title,
    SprintInput.title,
    ProposalInput.title,
);

impl ContactInput {
    pub fn new(name: impl Into<String>, customer: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            customer: customer.into(),
            ..Default::default()
        }
    }
}

/// A freshly created entity.
#[derive(Debug, Clone)]
pub struct Created {
    pub kind: EntityKind,
    pub id: String,
    /// The entity's name or title.
    pub name: String,
    /// The entity directory (customers, projects, research, sprints) or file.
    pub path: PathBuf,
}

impl Created {
    /// `{"id", "name"|"title", "path"}` -- customers, projects and contacts use
    /// `name`, everything else `title` (the shape MCP and API clients expect).
    pub fn to_json(&self) -> JsonValue {
        let name_key = match self.kind {
            EntityKind::Customer | EntityKind::Project | EntityKind::Contact => "name",
            _ => "title",
        };
        serde_json::json!({
            "id": self.id,
            name_key: self.name,
            "path": self.path.display().to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Validation and normalisation helpers
// ---------------------------------------------------------------------------

fn check_mode(kind: EntityKind, cfg: &ResolvedConfig) -> McResult<()> {
    if !cfg.entity_available(&kind) {
        return Err(McError::not_available(kind, cfg));
    }
    Ok(())
}

fn validate_status(status: &str, kind: EntityKind, cfg: &ResolvedConfig) -> McResult<()> {
    let valid = kind.statuses(cfg);
    if !valid.iter().any(|s| s == status) {
        return Err(McError::usage(
            format!(
                "Invalid {} status '{}'. Valid statuses: {}",
                kind.label(),
                status,
                valid.join(", ")
            ),
            None,
        ));
    }
    Ok(())
}

fn validate_name_not_empty(name: &str, kind: EntityKind) -> McResult<()> {
    if name.trim().is_empty() {
        return Err(McError::usage(
            format!("{} name/title cannot be empty", kind.label()),
            None,
        ));
    }
    Ok(())
}

/// Parse a strict `YYYY-MM-DD` date. chrono alone also accepts unpadded
/// forms like `2026-1-5`, which would sort and name files inconsistently.
fn validate_date(value: &str, field: &str) -> McResult<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .filter(|d| d.format("%Y-%m-%d").to_string() == value)
        .ok_or_else(|| {
            McError::usage(
                format!("Invalid {field} '{value}' (expected YYYY-MM-DD)"),
                None,
            )
        })
}

fn validate_priority(priority: u32) -> McResult<()> {
    if (1..=4).contains(&priority) {
        Ok(())
    } else {
        Err(McError::usage(
            format!(
                "Invalid priority {priority} (expected 1-4: 1=critical, 2=high, 3=medium, 4=low)"
            ),
            None,
        ))
    }
}

/// Trimmed value, or `None` when missing or blank.
fn present(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// The given status (validated) or the kind's first configured status.
fn resolve_status(
    status: &Option<String>,
    kind: EntityKind,
    cfg: &ResolvedConfig,
) -> McResult<String> {
    let status = match present(status) {
        Some(s) => s.to_string(),
        None => kind.statuses(cfg).first().cloned().unwrap_or_default(),
    };
    validate_status(&status, kind, cfg)?;
    Ok(status)
}

/// Cross-reference IDs without wiki-link brackets or blanks.
fn clean_refs(refs: &[String]) -> Vec<String> {
    refs.iter()
        .map(|r| frontmatter::strip_wikilink(r.trim()).trim().to_string())
        .filter(|r| !r.is_empty())
        .collect()
}

fn clean_ref(r: &Option<String>) -> Option<String> {
    present(r)
        .map(|r| frontmatter::strip_wikilink(r).trim().to_string())
        .filter(|r| !r.is_empty())
}

/// Slug for file and directory names; never empty (`untitled` fallback).
fn slug_for(name: &str) -> String {
    let slug = util::slugify(name);
    if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug
    }
}

/// `dir/stem.md`, or `dir/stem-2.md`, `-3`, ... if that file already exists.
fn unique_md_path(dir: &Path, stem: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.md"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.md"));
        n += 1;
    }
    path
}

/// Find an entity directory (`<base>/<ID>-<slug>`) by ID.
fn find_entity_dir(base: &Path, id: &str) -> McResult<PathBuf> {
    let id = frontmatter::strip_wikilink(id.trim());
    if base.is_dir() && !id.is_empty() {
        let dir_prefix = format!("{}-", id);
        let mut entries: Vec<_> = fs::read_dir(base)?.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if entry.file_type()?.is_dir()
                && entry.file_name().to_string_lossy().starts_with(&dir_prefix)
            {
                return Ok(entry.path());
            }
        }
    }
    Err(McError::EntityNotFound(id.to_string()))
}

/// Find a project directory by its ID (e.g. "PROJ-001").
fn find_project_dir(cfg: &ResolvedConfig, proj_id: &str) -> McResult<PathBuf> {
    find_entity_dir(&cfg.projects_dir, proj_id)
}

/// Find a customer directory by its ID (e.g. "CUST-001").
pub fn find_customer_dir(cfg: &ResolvedConfig, cust_id: &str) -> McResult<PathBuf> {
    find_entity_dir(&cfg.customers_dir, cust_id)
}

/// Create a directory with a .gitkeep file so git tracks it.
fn mkdir_with_gitkeep(path: &Path) -> McResult<()> {
    fs::create_dir_all(path)?;
    fs::write(path.join(".gitkeep"), "")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Frontmatter building and rendering
// ---------------------------------------------------------------------------

fn s(v: impl Into<String>) -> Value {
    Value::String(v.into())
}

fn list(items: &[String]) -> Value {
    Value::Sequence(items.iter().map(|i| s(i.as_str())).collect())
}

fn links(items: &[String]) -> Value {
    Value::Sequence(
        items
            .iter()
            .map(|i| s(frontmatter::wrap_wikilink(i)))
            .collect(),
    )
}

/// Frontmatter fields set on every entity: `id` and `aliases: [id]`.
fn id_fields(id: &str) -> Vec<(String, Value)> {
    vec![
        ("id".into(), s(id)),
        ("aliases".into(), Value::Sequence(vec![s(id)])),
    ]
}

/// Load `templates/<name>.md`, overlay `fields`, fill `{{ key }}` placeholders.
fn render(
    cfg: &ResolvedConfig,
    template_name: &str,
    fields: &[(String, Value)],
    placeholders: &[(&str, &str)],
) -> McResult<String> {
    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, template_name)?;
    let (fm, body) = template::render_template_ordered(tmpl_fm, &tmpl_body, fields, placeholders);
    Ok(frontmatter::serialize_document(&fm, &body))
}

/// Write `<dir>/<id>.md` inside a fresh `<base>/<id>-<slug>/` directory.
fn write_entity_dir(base: &Path, id: &str, slug: &str, doc: &str) -> McResult<PathBuf> {
    let dir = base.join(format!("{id}-{slug}"));
    fs::create_dir_all(&dir)?;
    util::atomic_write(&dir.join(format!("{id}.md")), doc.as_bytes())?;
    Ok(dir)
}

// ---------------------------------------------------------------------------
// Creation (no prompts, no printing)
// ---------------------------------------------------------------------------

pub fn create_customer(cfg: &ResolvedConfig, input: &CustomerInput) -> McResult<Created> {
    let kind = EntityKind::Customer;
    check_mode(kind, cfg)?;
    validate_name_not_empty(&input.name, kind)?;
    let name = input.name.trim();
    let status = resolve_status(&input.status, kind, cfg)?;
    let id = entity::next_id(kind, cfg)?.to_string();
    let slug = slug_for(name);
    let today = util::today_str();

    let mut fields = id_fields(&id);
    fields.extend([
        ("name".into(), s(name)),
        ("slug".into(), s(slug.as_str())),
        ("status".into(), s(status)),
        ("owner".into(), s(present(&input.owner).unwrap_or(""))),
        ("tags".into(), list(&input.tags)),
        ("projects".into(), Value::Sequence(vec![])),
        ("contracts".into(), Value::Sequence(vec![])),
        ("notes".into(), s("")),
        ("created".into(), s(today.as_str())),
        ("updated".into(), s(today)),
    ]);
    let doc = render(cfg, "customer", &fields, &[("name", name), ("id", &id)])?;

    let dir = write_entity_dir(&cfg.customers_dir, &id, &slug, &doc)?;
    for sub in ["contacts", "contracts", "meetings", "projects", "assets"] {
        mkdir_with_gitkeep(&dir.join(sub))?;
    }
    Ok(Created {
        kind,
        id,
        name: name.to_string(),
        path: dir,
    })
}

pub fn create_project(cfg: &ResolvedConfig, input: &ProjectInput) -> McResult<Created> {
    let kind = EntityKind::Project;
    check_mode(kind, cfg)?;
    validate_name_not_empty(&input.name, kind)?;
    let name = input.name.trim();
    let status = resolve_status(&input.status, kind, cfg)?;
    let id = entity::next_id(kind, cfg)?.to_string();
    let slug = slug_for(name);
    let today = util::today_str();

    let mut fields = id_fields(&id);
    fields.extend([
        ("name".into(), s(name)),
        ("slug".into(), s(slug.as_str())),
        ("status".into(), s(status)),
        ("owner".into(), s(present(&input.owner).unwrap_or(""))),
        ("customers".into(), links(&clean_refs(&input.customers))),
        ("tags".into(), list(&input.tags)),
        ("start_date".into(), s(today.as_str())),
        ("target_date".into(), s("")),
        ("created".into(), s(today.as_str())),
        ("updated".into(), s(today)),
    ]);
    let doc = render(cfg, "project", &fields, &[("name", name), ("id", &id)])?;

    let dir = write_entity_dir(&cfg.projects_dir, &id, &slug, &doc)?;
    fs::write(dir.join("roadmap.md"), format!("# {} -- Roadmap\n", name))?;
    fs::write(dir.join("backlog.md"), format!("# {} -- Backlog\n", name))?;
    for sub in ["specs", "releases", "infra"] {
        mkdir_with_gitkeep(&dir.join(sub))?;
    }
    Ok(Created {
        kind,
        id,
        name: name.to_string(),
        path: dir,
    })
}

pub fn create_meeting(cfg: &ResolvedConfig, input: &MeetingInput) -> McResult<Created> {
    let kind = EntityKind::Meeting;
    validate_name_not_empty(&input.title, kind)?;
    let title = input.title.trim();
    let date = match present(&input.date) {
        Some(d) => {
            // The date becomes part of the file name -- never trust it unchecked.
            validate_date(d, "meeting date")?;
            d.to_string()
        }
        None => util::today_str(),
    };
    let status = resolve_status(&input.status, kind, cfg)?;
    let id = entity::next_id(kind, cfg)?.to_string();

    let mut fields = id_fields(&id);
    fields.extend([
        ("title".into(), s(title)),
        ("date".into(), s(date.as_str())),
        ("time".into(), s(present(&input.time).unwrap_or("10:00"))),
        (
            "duration".into(),
            s(present(&input.duration).unwrap_or("30m")),
        ),
        ("tags".into(), list(&input.tags)),
        ("customers".into(), links(&clean_refs(&input.customers))),
        ("projects".into(), links(&clean_refs(&input.projects))),
        ("attendees".into(), list(&input.attendees)),
        ("status".into(), s(status)),
    ]);
    let doc = render(cfg, "meeting", &fields, &[("title", title), ("id", &id)])?;

    fs::create_dir_all(&cfg.meetings_dir)?;
    // Two meetings with the same date and title must not overwrite each other.
    let path = unique_md_path(&cfg.meetings_dir, &format!("{}-{}", date, slug_for(title)));
    util::atomic_write(&path, doc.as_bytes())?;
    Ok(Created {
        kind,
        id,
        name: title.to_string(),
        path,
    })
}

pub fn create_research(cfg: &ResolvedConfig, input: &ResearchInput) -> McResult<Created> {
    let kind = EntityKind::Research;
    validate_name_not_empty(&input.title, kind)?;
    let title = input.title.trim();
    let agents: Vec<String> = match &input.agents {
        Some(a) => a
            .iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect(),
        None => DEFAULT_RESEARCH_AGENTS
            .iter()
            .map(|a| a.to_string())
            .collect(),
    };
    let id = entity::next_id(kind, cfg)?.to_string();
    let slug = slug_for(title);
    let today = util::today_str();
    let status = if kind.statuses(cfg).iter().any(|s| s == "draft") {
        "draft".to_string()
    } else {
        resolve_status(&None, kind, cfg)?
    };

    let mut fields = id_fields(&id);
    fields.extend([
        ("title".into(), s(title)),
        ("slug".into(), s(slug.as_str())),
        ("status".into(), s(status)),
        ("owner".into(), s(present(&input.owner).unwrap_or(""))),
        ("customers".into(), Value::Sequence(vec![])),
        ("projects".into(), Value::Sequence(vec![])),
        ("tags".into(), list(&input.tags)),
        ("created".into(), s(today.as_str())),
        ("updated".into(), s(today)),
        ("agents".into(), list(&agents)),
        ("summary".into(), s("")),
    ]);
    let doc = render(cfg, "research", &fields, &[("title", title), ("id", &id)])?;

    let dir = write_entity_dir(&cfg.research_dir, &id, &slug, &doc)?;
    // Agent names come from user input: slugify them so they can't escape the
    // research directory (e.g. "../../etc").
    let mut agent_dirs: Vec<String> = agents.iter().map(|a| util::slugify(a)).collect();
    agent_dirs.retain(|d| !d.is_empty() && d != "final");
    agent_dirs.sort();
    agent_dirs.dedup();
    for agent_dir in &agent_dirs {
        mkdir_with_gitkeep(&dir.join(agent_dir))?;
    }
    mkdir_with_gitkeep(&dir.join("final"))?;
    Ok(Created {
        kind,
        id,
        name: title.to_string(),
        path: dir,
    })
}

pub fn create_task(cfg: &ResolvedConfig, input: &TaskInput) -> McResult<Created> {
    let kind = EntityKind::Task;
    validate_name_not_empty(&input.title, kind)?;
    let title = input.title.trim();
    let status = resolve_status(&input.status, kind, cfg)?;
    let priority = input.priority.unwrap_or(DEFAULT_PRIORITY);
    validate_priority(priority)?;
    let due_date = present(&input.due_date).unwrap_or("");
    if !due_date.is_empty() {
        validate_date(due_date, "due date")?;
    }
    let project = clean_ref(&input.project);
    let customer = clean_ref(&input.customer);

    // Resolve the scope first so an unknown project/customer fails before an
    // ID is allocated.
    let tasks_base = if let Some(proj_id) = &project {
        find_project_dir(cfg, proj_id)?.join("tasks")
    } else if let Some(cust_id) = &customer {
        find_customer_dir(cfg, cust_id)?.join("tasks")
    } else {
        cfg.tasks_dir.clone()
    };

    let id = entity::next_id(kind, cfg)?.to_string();
    let slug = slug_for(title);
    let today = util::today_str();
    let projects: Vec<String> = project.into_iter().collect();
    let customers: Vec<String> = customer.into_iter().collect();
    let sprint = clean_ref(&input.sprint).unwrap_or_default();

    let mut fields = id_fields(&id);
    fields.extend([
        ("title".into(), s(title)),
        ("slug".into(), s(slug.as_str())),
        ("status".into(), s(status.as_str())),
        (
            "priority".into(),
            Value::Number(serde_yaml::Number::from(u64::from(priority))),
        ),
        ("owner".into(), s(present(&input.owner).unwrap_or(""))),
        ("projects".into(), links(&projects)),
        ("customers".into(), links(&customers)),
        ("tags".into(), list(&input.tags)),
        ("sprint".into(), s(frontmatter::wrap_wikilink(&sprint))),
        ("depends_on".into(), links(&clean_refs(&input.depends_on))),
        ("due_date".into(), s(due_date)),
        ("created".into(), s(today.as_str())),
        ("updated".into(), s(today)),
    ]);
    let doc = render(cfg, "task", &fields, &[("title", title), ("id", &id)])?;

    // Same placement as `mc task move`: active statuses in todo/, others in done/.
    let todo_dir = tasks_base.join("todo");
    let done_dir = tasks_base.join("done");
    fs::create_dir_all(&todo_dir)?;
    if !done_dir.exists() {
        mkdir_with_gitkeep(&done_dir)?;
    }
    let path = tasks_base
        .join(entity::task_status_folder(&status))
        .join(format!("{}-{}.md", id, slug));
    util::atomic_write(&path, doc.as_bytes())?;
    Ok(Created {
        kind,
        id,
        name: title.to_string(),
        path,
    })
}

pub fn create_sprint(cfg: &ResolvedConfig, input: &SprintInput) -> McResult<Created> {
    let kind = EntityKind::Sprint;
    validate_name_not_empty(&input.title, kind)?;
    let title = input.title.trim();
    let status = resolve_status(&input.status, kind, cfg)?;
    let today = util::today_str();
    let start_date = present(&input.start_date).unwrap_or(&today).to_string();
    let start = validate_date(&start_date, "start date")?;
    let end_date = present(&input.end_date).unwrap_or("");
    if !end_date.is_empty() {
        let end = validate_date(end_date, "end date")?;
        if end < start {
            return Err(McError::usage(
                format!(
                    "Invalid end date '{end_date}': it is before the start date '{start_date}'"
                ),
                None,
            ));
        }
    }
    let id = entity::next_id(kind, cfg)?.to_string();
    let slug = slug_for(title);

    let mut fields = id_fields(&id);
    fields.extend([
        ("title".into(), s(title)),
        ("status".into(), s(status)),
        ("goal".into(), s(present(&input.goal).unwrap_or(""))),
        ("start_date".into(), s(start_date)),
        ("end_date".into(), s(end_date)),
        ("owner".into(), s(present(&input.owner).unwrap_or(""))),
        ("projects".into(), links(&clean_refs(&input.projects))),
        ("tags".into(), list(&input.tags)),
        ("created".into(), s(today.as_str())),
        ("updated".into(), s(today)),
    ]);
    let doc = render(cfg, "sprint", &fields, &[("title", title), ("id", &id)])?;

    let dir = write_entity_dir(&cfg.sprints_dir, &id, &slug, &doc)?;
    let ceremonies = [
        (
            "planning.md",
            "Sprint Planning",
            "## Capacity\n\n## Selected Items\n\n## Notes\n",
        ),
        (
            "review.md",
            "Sprint Review",
            "## Demo Outcomes\n\n## Feedback\n\n## Notes\n",
        ),
        (
            "retrospective.md",
            "Retrospective",
            "## What Went Well\n\n## What Could Improve\n\n## Action Items\n",
        ),
    ];
    for (file, heading, sections) in ceremonies {
        fs::write(
            dir.join(file),
            format!("# {} -- {}\n\n{}", title, heading, sections),
        )?;
    }
    Ok(Created {
        kind,
        id,
        name: title.to_string(),
        path: dir,
    })
}

pub fn create_proposal(cfg: &ResolvedConfig, input: &ProposalInput) -> McResult<Created> {
    let kind = EntityKind::Proposal;
    check_mode(kind, cfg)?;
    validate_name_not_empty(&input.title, kind)?;
    let title = input.title.trim();
    let status = resolve_status(&input.status, kind, cfg)?;
    let id = entity::next_id(kind, cfg)?.to_string();
    let slug = slug_for(title);
    let today = util::today_str();
    let supersedes = clean_ref(&input.supersedes).unwrap_or_default();

    let mut fields = id_fields(&id);
    fields.extend([
        ("title".into(), s(title)),
        ("status".into(), s(status)),
        (
            "type".into(),
            s(present(&input.proposal_type).unwrap_or(PROPOSAL_TYPES[0])),
        ),
        ("author".into(), s(present(&input.author).unwrap_or(""))),
        (
            "supersedes".into(),
            s(frontmatter::wrap_wikilink(&supersedes)),
        ),
        ("superseded_by".into(), s("")),
        ("tags".into(), list(&input.tags)),
        ("created".into(), s(today.as_str())),
        ("updated".into(), s(today)),
    ]);
    let doc = render(cfg, "proposal", &fields, &[("title", title), ("id", &id)])?;

    fs::create_dir_all(&cfg.proposals_dir)?;
    let path = cfg.proposals_dir.join(format!("{}-{}.md", id, slug));
    util::atomic_write(&path, doc.as_bytes())?;
    Ok(Created {
        kind,
        id,
        name: title.to_string(),
        path,
    })
}

pub fn create_contact(cfg: &ResolvedConfig, input: &ContactInput) -> McResult<Created> {
    let kind = EntityKind::Contact;
    check_mode(kind, cfg)?;
    validate_name_not_empty(&input.name, kind)?;
    let name = input.name.trim();
    let customer = frontmatter::strip_wikilink(input.customer.trim()).to_string();
    let cust_dir = find_customer_dir(cfg, &customer)?;
    let status = resolve_status(&input.status, kind, cfg)?;
    let id = entity::next_id(kind, cfg)?.to_string();
    let today = util::today_str();

    let mut fields = id_fields(&id);
    fields.extend([
        ("name".into(), s(name)),
        ("role".into(), s(present(&input.role).unwrap_or(""))),
        ("email".into(), s(present(&input.email).unwrap_or(""))),
        ("phone".into(), s(present(&input.phone).unwrap_or(""))),
        ("customer".into(), s(frontmatter::wrap_wikilink(&customer))),
        ("status".into(), s(status)),
        ("tags".into(), list(&input.tags)),
        ("created".into(), s(today.as_str())),
        ("updated".into(), s(today)),
    ]);
    let doc = render(cfg, "contact", &fields, &[("name", name), ("id", &id)])?;

    let contacts_dir = cust_dir.join("contacts");
    fs::create_dir_all(&contacts_dir)?;
    let path = contacts_dir.join(format!("{}-{}.md", id, slug_for(name)));
    util::atomic_write(&path, doc.as_bytes())?;
    Ok(Created {
        kind,
        id,
        name: name.to_string(),
        path,
    })
}

// ---------------------------------------------------------------------------
// Programmatic wrappers (string arguments, JSON result) -- used by MCP and API
// ---------------------------------------------------------------------------

fn opt(v: Option<&str>) -> Option<String> {
    v.map(str::to_string)
}

fn csv(v: Option<&str>) -> Vec<String> {
    v.map(util::parse_comma_list).unwrap_or_default()
}

pub fn create_customer_programmatic(
    cfg: &ResolvedConfig,
    name: &str,
    owner: Option<&str>,
    status: Option<&str>,
    tags: Option<&str>,
) -> McResult<JsonValue> {
    let input = CustomerInput {
        name: name.to_string(),
        owner: opt(owner),
        status: opt(status),
        tags: csv(tags),
    };
    Ok(create_customer(cfg, &input)?.to_json())
}

pub fn create_project_programmatic(
    cfg: &ResolvedConfig,
    name: &str,
    owner: Option<&str>,
    status: Option<&str>,
    customers: Option<&str>,
    tags: Option<&str>,
) -> McResult<JsonValue> {
    let input = ProjectInput {
        name: name.to_string(),
        owner: opt(owner),
        status: opt(status),
        customers: csv(customers),
        tags: csv(tags),
    };
    Ok(create_project(cfg, &input)?.to_json())
}

#[allow(clippy::too_many_arguments)]
pub fn create_meeting_programmatic(
    cfg: &ResolvedConfig,
    title: &str,
    date: Option<&str>,
    time: Option<&str>,
    duration: Option<&str>,
    status: Option<&str>,
    tags: Option<&str>,
    customers: Option<&str>,
    projects: Option<&str>,
    attendees: Option<&str>,
) -> McResult<JsonValue> {
    let input = MeetingInput {
        title: title.to_string(),
        date: opt(date),
        time: opt(time),
        duration: opt(duration),
        status: opt(status),
        tags: csv(tags),
        customers: csv(customers),
        projects: csv(projects),
        attendees: csv(attendees),
    };
    Ok(create_meeting(cfg, &input)?.to_json())
}

pub fn create_research_programmatic(
    cfg: &ResolvedConfig,
    title: &str,
    owner: Option<&str>,
    agents: Option<&str>,
    tags: Option<&str>,
) -> McResult<JsonValue> {
    let input = ResearchInput {
        title: title.to_string(),
        owner: opt(owner),
        agents: agents.map(util::parse_comma_list),
        tags: csv(tags),
    };
    Ok(create_research(cfg, &input)?.to_json())
}

#[allow(clippy::too_many_arguments)]
pub fn create_task_programmatic(
    cfg: &ResolvedConfig,
    title: &str,
    project: Option<&str>,
    customer: Option<&str>,
    owner: Option<&str>,
    status: Option<&str>,
    priority: Option<u32>,
    tags: Option<&str>,
    sprint: Option<&str>,
    depends_on: Option<&str>,
    due_date: Option<&str>,
) -> McResult<JsonValue> {
    let input = TaskInput {
        title: title.to_string(),
        project: opt(project),
        customer: opt(customer),
        owner: opt(owner),
        status: opt(status),
        priority,
        tags: csv(tags),
        sprint: opt(sprint),
        depends_on: csv(depends_on),
        due_date: opt(due_date),
    };
    Ok(create_task(cfg, &input)?.to_json())
}

#[allow(clippy::too_many_arguments)]
pub fn create_sprint_programmatic(
    cfg: &ResolvedConfig,
    title: &str,
    owner: Option<&str>,
    status: Option<&str>,
    goal: Option<&str>,
    start_date: Option<&str>,
    end_date: Option<&str>,
    projects: Option<&str>,
    tags: Option<&str>,
) -> McResult<JsonValue> {
    let input = SprintInput {
        title: title.to_string(),
        owner: opt(owner),
        status: opt(status),
        goal: opt(goal),
        start_date: opt(start_date),
        end_date: opt(end_date),
        projects: csv(projects),
        tags: csv(tags),
    };
    Ok(create_sprint(cfg, &input)?.to_json())
}

#[allow(clippy::too_many_arguments)]
pub fn create_proposal_programmatic(
    cfg: &ResolvedConfig,
    title: &str,
    author: Option<&str>,
    status: Option<&str>,
    proposal_type: Option<&str>,
    tags: Option<&str>,
    supersedes: Option<&str>,
) -> McResult<JsonValue> {
    let input = ProposalInput {
        title: title.to_string(),
        author: opt(author),
        status: opt(status),
        proposal_type: opt(proposal_type),
        tags: csv(tags),
        supersedes: opt(supersedes),
    };
    Ok(create_proposal(cfg, &input)?.to_json())
}

#[allow(clippy::too_many_arguments)]
pub fn create_contact_programmatic(
    cfg: &ResolvedConfig,
    name: &str,
    customer: &str,
    role: Option<&str>,
    email: Option<&str>,
    phone: Option<&str>,
    status: Option<&str>,
    tags: Option<&str>,
) -> McResult<JsonValue> {
    let input = ContactInput {
        name: name.to_string(),
        customer: customer.to_string(),
        role: opt(role),
        email: opt(email),
        phone: opt(phone),
        status: opt(status),
        tags: csv(tags),
    };
    Ok(create_contact(cfg, &input)?.to_json())
}

// ---------------------------------------------------------------------------
// Interactive CLI
// ---------------------------------------------------------------------------

/// Prompts for values the user did not pass as flags. With `yes` (or without
/// a terminal) every prompt silently takes its default.
struct Prompter {
    interactive: bool,
}

impl Prompter {
    fn new(yes: bool) -> Self {
        Self {
            interactive: !yes && std::io::IsTerminal::is_terminal(&std::io::stdin()),
        }
    }

    fn text(&self, given: Option<&str>, label: &str, default: &str) -> String {
        if let Some(v) = given {
            return v.to_string();
        }
        if !self.interactive {
            return default.to_string();
        }
        dialoguer::Input::<String>::new()
            .with_prompt(label)
            .default(default.to_string())
            .interact_text()
            .unwrap_or_else(|_| default.to_string())
    }

    fn optional(&self, given: Option<&str>, label: &str) -> String {
        if let Some(v) = given {
            return v.to_string();
        }
        if !self.interactive {
            return String::new();
        }
        dialoguer::Input::<String>::new()
            .with_prompt(format!("{} (blank to skip)", label))
            .allow_empty(true)
            .interact_text()
            .unwrap_or_default()
    }

    fn list(&self, given: Option<&str>, label: &str) -> Vec<String> {
        util::parse_comma_list(&self.optional(given, label))
    }

    fn select(&self, given: Option<&str>, label: &str, options: &[String]) -> String {
        if let Some(v) = given {
            return v.to_string();
        }
        let default = options.first().cloned().unwrap_or_default();
        if !self.interactive || options.is_empty() {
            return default;
        }
        match dialoguer::Select::new()
            .with_prompt(label)
            .items(options)
            .default(0)
            .interact_opt()
        {
            Ok(Some(idx)) => options[idx].clone(),
            _ => default,
        }
    }

    fn confirm(&self) -> bool {
        if !self.interactive {
            return true;
        }
        dialoguer::Confirm::new()
            .with_prompt("Create this entity?")
            .default(true)
            .interact()
            .unwrap_or_default()
    }
}

fn print_summary(kind: EntityKind, id: &str, fields: &[(&str, String)]) {
    println!();
    println!("  {} {}", "New".bold(), kind.label().bold());
    println!("  {}", ui::rule(36));
    println!("  {:<14} {}", "ID:".dimmed(), id);
    for (key, value) in fields {
        let display = if value.is_empty() {
            "(none)".dimmed().to_string()
        } else {
            value.to_string()
        };
        println!("  {:<14} {}", format!("{}:", key).dimmed(), display);
    }
    println!("  {}", ui::rule(36));
}

/// Show the summary, ask for confirmation, create, and report the result.
fn confirm_and_create(
    p: &Prompter,
    cfg: &ResolvedConfig,
    kind: EntityKind,
    fields: &[(&str, String)],
    create: impl FnOnce() -> McResult<Created>,
) -> McResult<()> {
    let preview_id = entity::next_id(kind, cfg)?;
    print_summary(kind, &preview_id.to_string(), fields);
    if !p.confirm() {
        println!("{}", "Cancelled.".dimmed());
        return Ok(());
    }
    let created = create()?;
    println!(
        "{} Created {} {} ({}) at {}",
        ui::glyphs().ok.green().bold(),
        kind.label(),
        created.id.cyan().bold(),
        created.name.bold(),
        created.path.display().to_string().dimmed()
    );
    Ok(())
}

fn priority_label(priority: u32) -> String {
    let name = match priority {
        1 => "critical",
        2 => "high",
        3 => "medium",
        4 => "low",
        _ => "invalid",
    };
    format!("{priority} ({name})")
}

pub fn run(entity: &NewEntity, cfg: &ResolvedConfig, yes: bool) -> McResult<()> {
    let p = Prompter::new(yes);
    match entity {
        NewEntity::Customer {
            name,
            owner,
            status,
            tags,
        } => {
            let kind = EntityKind::Customer;
            check_mode(kind, cfg)?;
            validate_name_not_empty(name, kind)?;
            let input = CustomerInput {
                name: name.clone(),
                owner: Some(p.text(owner.as_deref(), "Owner", "")),
                status: Some(p.select(status.as_deref(), "Status", kind.statuses(cfg))),
                tags: p.list(tags.as_deref(), "Tags (comma-separated)"),
            };
            resolve_status(&input.status, kind, cfg)?;
            let fields = [
                ("Name", input.name.clone()),
                ("Owner", input.owner.clone().unwrap_or_default()),
                ("Status", input.status.clone().unwrap_or_default()),
                ("Tags", input.tags.join(", ")),
            ];
            confirm_and_create(&p, cfg, kind, &fields, || create_customer(cfg, &input))
        }
        NewEntity::Project {
            name,
            owner,
            status,
            customers,
            tags,
        } => {
            let kind = EntityKind::Project;
            check_mode(kind, cfg)?;
            validate_name_not_empty(name, kind)?;
            let input = ProjectInput {
                name: name.clone(),
                owner: Some(p.text(owner.as_deref(), "Owner", "")),
                status: Some(p.select(status.as_deref(), "Status", kind.statuses(cfg))),
                tags: p.list(tags.as_deref(), "Tags (comma-separated)"),
                customers: p.list(customers.as_deref(), "Link customers (comma-separated IDs)"),
            };
            resolve_status(&input.status, kind, cfg)?;
            let fields = [
                ("Name", input.name.clone()),
                ("Owner", input.owner.clone().unwrap_or_default()),
                ("Status", input.status.clone().unwrap_or_default()),
                ("Tags", input.tags.join(", ")),
                ("Customers", input.customers.join(", ")),
            ];
            confirm_and_create(&p, cfg, kind, &fields, || create_project(cfg, &input))
        }
        NewEntity::Meeting {
            title,
            date,
            time,
            duration,
            status,
            tags,
            customers,
            projects,
            attendees,
        } => {
            let kind = EntityKind::Meeting;
            validate_name_not_empty(title, kind)?;
            let input = MeetingInput {
                title: title.clone(),
                date: Some(date.clone().unwrap_or_else(util::today_str)),
                time: Some(p.text(time.as_deref(), "Time (HH:MM)", "10:00")),
                duration: Some(p.text(duration.as_deref(), "Duration", "30m")),
                status: Some(p.select(status.as_deref(), "Status", kind.statuses(cfg))),
                tags: p.list(tags.as_deref(), "Tags (comma-separated)"),
                customers: p.list(customers.as_deref(), "Link customers (comma-separated IDs)"),
                projects: p.list(projects.as_deref(), "Link projects (comma-separated IDs)"),
                attendees: csv(attendees.as_deref()),
            };
            resolve_status(&input.status, kind, cfg)?;
            if let Some(d) = present(&input.date) {
                validate_date(d, "meeting date")?;
            }
            let fields = [
                ("Title", input.title.clone()),
                ("Date", input.date.clone().unwrap_or_default()),
                ("Time", input.time.clone().unwrap_or_default()),
                ("Duration", input.duration.clone().unwrap_or_default()),
                ("Status", input.status.clone().unwrap_or_default()),
                ("Tags", input.tags.join(", ")),
                ("Customers", input.customers.join(", ")),
                ("Projects", input.projects.join(", ")),
                ("Attendees", input.attendees.join(", ")),
            ];
            confirm_and_create(&p, cfg, kind, &fields, || create_meeting(cfg, &input))
        }
        NewEntity::Research {
            title,
            owner,
            agents,
            tags,
        } => {
            let kind = EntityKind::Research;
            validate_name_not_empty(title, kind)?;
            let input = ResearchInput {
                title: title.clone(),
                owner: Some(p.text(owner.as_deref(), "Owner", "")),
                agents: agents.as_deref().map(util::parse_comma_list),
                tags: p.list(tags.as_deref(), "Tags (comma-separated)"),
            };
            let agents_display = match &input.agents {
                Some(a) => a.join(", "),
                None => DEFAULT_RESEARCH_AGENTS.join(", "),
            };
            let fields = [
                ("Title", input.title.clone()),
                ("Owner", input.owner.clone().unwrap_or_default()),
                ("Agents", agents_display),
                ("Tags", input.tags.join(", ")),
            ];
            confirm_and_create(&p, cfg, kind, &fields, || create_research(cfg, &input))
        }
        NewEntity::Task {
            title,
            project,
            customer,
            owner,
            status,
            priority,
            tags,
            sprint,
            depends_on,
            due_date,
        } => {
            let kind = EntityKind::Task;
            validate_name_not_empty(title, kind)?;
            let input = TaskInput {
                title: title.clone(),
                project: project.clone(),
                customer: customer.clone(),
                owner: Some(p.text(owner.as_deref(), "Owner", "")),
                status: Some(p.select(status.as_deref(), "Status", kind.statuses(cfg))),
                priority: *priority,
                tags: p.list(tags.as_deref(), "Tags (comma-separated)"),
                sprint: sprint.clone(),
                depends_on: csv(depends_on.as_deref()),
                due_date: due_date.clone(),
            };
            resolve_status(&input.status, kind, cfg)?;
            let priority = input.priority.unwrap_or(DEFAULT_PRIORITY);
            validate_priority(priority)?;
            let fields = [
                ("Title", input.title.clone()),
                ("Status", input.status.clone().unwrap_or_default()),
                ("Priority", priority_label(priority)),
                ("Owner", input.owner.clone().unwrap_or_default()),
                ("Projects", input.project.clone().unwrap_or_default()),
                ("Customers", input.customer.clone().unwrap_or_default()),
                ("Sprint", input.sprint.clone().unwrap_or_default()),
                ("Tags", input.tags.join(", ")),
                ("Depends on", input.depends_on.join(", ")),
                ("Due date", input.due_date.clone().unwrap_or_default()),
            ];
            confirm_and_create(&p, cfg, kind, &fields, || create_task(cfg, &input))
        }
        NewEntity::Sprint {
            title,
            owner,
            status,
            goal,
            start_date,
            end_date,
            projects,
            tags,
        } => {
            let kind = EntityKind::Sprint;
            validate_name_not_empty(title, kind)?;
            let input = SprintInput {
                title: title.clone(),
                owner: Some(p.text(owner.as_deref(), "Owner", "")),
                status: Some(p.select(status.as_deref(), "Status", kind.statuses(cfg))),
                goal: Some(p.optional(goal.as_deref(), "Sprint goal")),
                start_date: Some(start_date.clone().unwrap_or_else(util::today_str)),
                end_date: end_date.clone(),
                tags: p.list(tags.as_deref(), "Tags (comma-separated)"),
                projects: p.list(projects.as_deref(), "Link projects (comma-separated IDs)"),
            };
            resolve_status(&input.status, kind, cfg)?;
            let fields = [
                ("Title", input.title.clone()),
                ("Status", input.status.clone().unwrap_or_default()),
                ("Goal", input.goal.clone().unwrap_or_default()),
                ("Start", input.start_date.clone().unwrap_or_default()),
                ("End", input.end_date.clone().unwrap_or_default()),
                ("Owner", input.owner.clone().unwrap_or_default()),
                ("Projects", input.projects.join(", ")),
                ("Tags", input.tags.join(", ")),
            ];
            confirm_and_create(&p, cfg, kind, &fields, || create_sprint(cfg, &input))
        }
        NewEntity::Proposal {
            title,
            author,
            status,
            proposal_type,
            tags,
            supersedes,
        } => {
            let kind = EntityKind::Proposal;
            check_mode(kind, cfg)?;
            validate_name_not_empty(title, kind)?;
            let types: Vec<String> = PROPOSAL_TYPES.iter().map(|t| t.to_string()).collect();
            let input = ProposalInput {
                title: title.clone(),
                author: Some(p.text(author.as_deref(), "Author", "")),
                status: Some(p.select(status.as_deref(), "Status", kind.statuses(cfg))),
                proposal_type: Some(p.select(proposal_type.as_deref(), "Type", &types)),
                tags: p.list(tags.as_deref(), "Tags (comma-separated)"),
                supersedes: supersedes.clone(),
            };
            resolve_status(&input.status, kind, cfg)?;
            let fields = [
                ("Title", input.title.clone()),
                ("Author", input.author.clone().unwrap_or_default()),
                ("Status", input.status.clone().unwrap_or_default()),
                ("Type", input.proposal_type.clone().unwrap_or_default()),
                ("Tags", input.tags.join(", ")),
                ("Supersedes", input.supersedes.clone().unwrap_or_default()),
            ];
            confirm_and_create(&p, cfg, kind, &fields, || create_proposal(cfg, &input))
        }
        NewEntity::Contact {
            name,
            customer,
            role,
            email,
            phone,
            status,
            tags,
        } => {
            let kind = EntityKind::Contact;
            check_mode(kind, cfg)?;
            validate_name_not_empty(name, kind)?;
            // Fail fast on an unknown customer, before any prompting.
            find_customer_dir(cfg, customer)?;
            let input = ContactInput {
                name: name.clone(),
                customer: customer.clone(),
                role: role.clone(),
                email: email.clone(),
                phone: phone.clone(),
                status: Some(p.select(status.as_deref(), "Status", kind.statuses(cfg))),
                tags: p.list(tags.as_deref(), "Tags (comma-separated)"),
            };
            resolve_status(&input.status, kind, cfg)?;
            let fields = [
                ("Name", input.name.clone()),
                ("Customer", input.customer.clone()),
                ("Role", input.role.clone().unwrap_or_default()),
                ("Email", input.email.clone().unwrap_or_default()),
                ("Phone", input.phone.clone().unwrap_or_default()),
                ("Status", input.status.clone().unwrap_or_default()),
                ("Tags", input.tags.join(", ")),
            ];
            confirm_and_create(&p, cfg, kind, &fields, || create_contact(cfg, &input))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::init;
    use crate::config;
    use tempfile::TempDir;

    fn setup_repo() -> (TempDir, ResolvedConfig) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        init::run(root, false, false, Some("TestRepo"), false, true).unwrap();
        let cfg = config::load_config(root, config::RepoMode::Standalone).unwrap();
        (tmp, cfg)
    }

    #[test]
    fn test_create_customer_and_verify_files() {
        let (_tmp, cfg) = setup_repo();

        let result =
            create_customer_programmatic(&cfg, "Acme Inc", Some("alice"), Some("active"), None)
                .unwrap();
        assert_eq!(result["id"], "CUST-001");
        assert_eq!(result["name"], "Acme Inc");

        // Verify directory and entity file exist
        let dir = cfg.customers_dir.join("CUST-001-acme-inc");
        assert!(dir.is_dir());
        assert!(dir.join("CUST-001.md").is_file());
        assert!(dir.join("contacts").is_dir());
        assert!(dir.join("contracts").is_dir());

        // Verify frontmatter
        let content = std::fs::read_to_string(dir.join("CUST-001.md")).unwrap();
        let (fm_str, _) = frontmatter::split_frontmatter(&content).unwrap();
        let fm = frontmatter::parse_raw(&fm_str, &dir.join("CUST-001.md")).unwrap();
        assert_eq!(frontmatter::get_str(&fm, "id").unwrap(), "CUST-001");
        assert_eq!(frontmatter::get_str(&fm, "name").unwrap(), "Acme Inc");
        assert_eq!(frontmatter::get_str(&fm, "status").unwrap(), "active");
        assert_eq!(frontmatter::get_str(&fm, "owner").unwrap(), "alice");
    }

    #[test]
    fn test_create_task_in_todo() {
        let (_tmp, cfg) = setup_repo();

        let result = create_task_programmatic(
            &cfg,
            "Fix login bug",
            None,
            None,
            Some("bob"),
            Some("todo"),
            Some(2),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(result["id"], "TASK-001");

        // Task should be in todo/
        let task_path = cfg.tasks_dir.join("todo").join("TASK-001-fix-login-bug.md");
        assert!(task_path.is_file());

        // Verify frontmatter
        let content = std::fs::read_to_string(&task_path).unwrap();
        let (fm_str, _) = frontmatter::split_frontmatter(&content).unwrap();
        let fm = frontmatter::parse_raw(&fm_str, &task_path).unwrap();
        assert_eq!(frontmatter::get_str(&fm, "status").unwrap(), "todo");
        assert_eq!(frontmatter::get_str(&fm, "owner").unwrap(), "bob");
    }

    #[test]
    fn test_invalid_status_rejected() {
        let (_tmp, cfg) = setup_repo();

        let result = create_customer_programmatic(&cfg, "Acme", Some("alice"), Some("bogus"), None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid customer status 'bogus'"));
        assert!(err.contains("active"));
    }

    #[test]
    fn test_empty_name_rejected() {
        let (_tmp, cfg) = setup_repo();

        let result = create_customer_programmatic(&cfg, "", None, Some("active"), None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cannot be empty"));

        // Also test whitespace-only
        let result = create_customer_programmatic(&cfg, "   ", None, Some("active"), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_task_title_rejected() {
        let (_tmp, cfg) = setup_repo();

        let result = create_task_programmatic(
            &cfg,
            "",
            None,
            None,
            None,
            Some("todo"),
            None,
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cannot be empty"));
    }

    #[test]
    fn test_invalid_task_status_rejected() {
        let (_tmp, cfg) = setup_repo();

        let result = create_task_programmatic(
            &cfg,
            "Some task",
            None,
            None,
            None,
            Some("invalid-status"),
            None,
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid task status"));
    }

    #[test]
    fn test_sequential_id_generation() {
        let (_tmp, cfg) = setup_repo();

        let r1 = create_customer_programmatic(&cfg, "First", None, Some("active"), None).unwrap();
        let r2 = create_customer_programmatic(&cfg, "Second", None, Some("active"), None).unwrap();
        assert_eq!(r1["id"], "CUST-001");
        assert_eq!(r2["id"], "CUST-002");
    }

    #[test]
    fn test_create_meeting() {
        let (_tmp, cfg) = setup_repo();

        let result = create_meeting_programmatic(
            &cfg,
            "Sprint Planning",
            Some("2025-01-15"),
            Some("09:00"),
            Some("60m"),
            Some("scheduled"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(result["id"], "MTG-001");

        let path_str = result["path"].as_str().unwrap();
        assert!(std::path::Path::new(path_str).is_file());
    }

    #[test]
    fn test_create_contact() {
        let (_tmp, cfg) = setup_repo();

        // First create a customer to attach the contact to
        create_customer_programmatic(&cfg, "Acme Inc", Some("alice"), Some("active"), None)
            .unwrap();

        let result = create_contact_programmatic(
            &cfg,
            "Alice Smith",
            "CUST-001",
            Some("VP Engineering"),
            Some("alice@acme.com"),
            Some("+1-555-0101"),
            Some("active"),
            None,
        )
        .unwrap();
        assert_eq!(result["id"], "CONT-001");
        assert_eq!(result["name"], "Alice Smith");

        // Verify file location
        let cust_dir = cfg.customers_dir.join("CUST-001-acme-inc");
        let contact_path = cust_dir.join("contacts").join("CONT-001-alice-smith.md");
        assert!(contact_path.is_file());

        // Verify frontmatter
        let content = std::fs::read_to_string(&contact_path).unwrap();
        let (fm_str, _) = frontmatter::split_frontmatter(&content).unwrap();
        let fm = frontmatter::parse_raw(&fm_str, &contact_path).unwrap();
        assert_eq!(frontmatter::get_str(&fm, "id").unwrap(), "CONT-001");
        assert_eq!(frontmatter::get_str(&fm, "name").unwrap(), "Alice Smith");
        assert_eq!(frontmatter::get_str(&fm, "role").unwrap(), "VP Engineering");
        assert_eq!(
            frontmatter::get_str(&fm, "email").unwrap(),
            "alice@acme.com"
        );
        assert_eq!(
            frontmatter::get_str(&fm, "customer").unwrap(),
            "[[CUST-001]]"
        );
        assert_eq!(frontmatter::get_str(&fm, "status").unwrap(), "active");
    }

    #[test]
    fn test_create_contact_invalid_customer() {
        let (_tmp, cfg) = setup_repo();

        let result = create_contact_programmatic(
            &cfg,
            "Alice Smith",
            "CUST-999",
            None,
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_create_contact_empty_name_rejected() {
        let (_tmp, cfg) = setup_repo();

        create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();

        let result =
            create_contact_programmatic(&cfg, "", "CUST-001", None, None, None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cannot be empty"));
    }

    #[test]
    fn test_create_contact_invalid_status_rejected() {
        let (_tmp, cfg) = setup_repo();

        create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();

        let result = create_contact_programmatic(
            &cfg,
            "Alice",
            "CUST-001",
            None,
            None,
            None,
            Some("bogus"),
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid contact status"));
    }

    #[test]
    fn test_contact_sequential_ids_across_customers() {
        let (_tmp, cfg) = setup_repo();

        create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();
        create_customer_programmatic(&cfg, "Beta", None, Some("active"), None).unwrap();

        let r1 =
            create_contact_programmatic(&cfg, "Alice", "CUST-001", None, None, None, None, None)
                .unwrap();
        let r2 = create_contact_programmatic(&cfg, "Bob", "CUST-002", None, None, None, None, None)
            .unwrap();

        assert_eq!(r1["id"], "CONT-001");
        assert_eq!(r2["id"], "CONT-002");
    }

    #[test]
    fn test_create_sprint() {
        let (_tmp, cfg) = setup_repo();

        let result = create_sprint_programmatic(
            &cfg,
            "Sprint 1",
            Some("alice"),
            Some("planning"),
            Some("Deliver MVP"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(result["id"], "SPR-001");

        let dir = cfg.sprints_dir.join("SPR-001-sprint-1");
        assert!(dir.join("SPR-001.md").is_file());
        assert!(dir.join("planning.md").is_file());
        assert!(dir.join("review.md").is_file());
        assert!(dir.join("retrospective.md").is_file());
    }

    fn read_fm(path: &Path) -> Value {
        frontmatter::parse_file(path).unwrap().0
    }

    #[test]
    fn test_meetings_same_day_same_title_do_not_overwrite() {
        let (_tmp, cfg) = setup_repo();
        let mut input = MeetingInput::new("Standup");
        input.date = Some("2026-03-02".into());
        let a = create_meeting(&cfg, &input).unwrap();
        let b = create_meeting(&cfg, &input).unwrap();
        assert_ne!(a.path, b.path);
        assert!(b.path.ends_with("2026-03-02-standup-2.md"));
        assert_eq!(
            frontmatter::get_str(&read_fm(&a.path), "id"),
            Some("MTG-001")
        );
        assert_eq!(
            frontmatter::get_str(&read_fm(&b.path), "id"),
            Some("MTG-002")
        );
    }

    #[test]
    fn test_meeting_date_is_validated() {
        let (_tmp, cfg) = setup_repo();
        for bad in [
            "../../escape",
            "2026-13-01",
            "tomorrow",
            "2026-3-2",
            " 2026-03-02x",
        ] {
            let mut input = MeetingInput::new("X");
            input.date = Some(bad.into());
            let err = create_meeting(&cfg, &input).unwrap_err();
            assert!(err.to_string().starts_with("Invalid meeting date"), "{err}");
        }
        assert!(!cfg.root.join("escape.md").exists());
    }

    #[test]
    fn test_research_agent_names_cannot_escape_directory() {
        let (_tmp, cfg) = setup_repo();
        let mut input = ResearchInput::new("Topic");
        input.agents = Some(vec!["../../evil".into(), "Claude Opus".into(), "  ".into()]);
        let created = create_research(&cfg, &input).unwrap();
        assert!(created.path.join("evil").is_dir());
        assert!(created.path.join("claude-opus").is_dir());
        assert!(!cfg.root.join("evil").exists());
        let fm = read_fm(&created.path.join("RES-001.md"));
        assert_eq!(
            frontmatter::get_string_list(&fm, "agents"),
            vec!["../../evil", "Claude Opus"]
        );
    }

    #[test]
    fn test_wikilinked_refs_are_not_double_wrapped() {
        let (_tmp, cfg) = setup_repo();
        create_customer_programmatic(&cfg, "Acme", None, None, None).unwrap();
        let created = create_project_programmatic(
            &cfg,
            "P",
            None,
            None,
            Some("[[CUST-001]], CUST-002"),
            None,
        )
        .unwrap();
        let path = PathBuf::from(created["path"].as_str().unwrap()).join("PROJ-001.md");
        let fm = read_fm(&path);
        assert_eq!(
            frontmatter::get_string_list(&fm, "customers"),
            vec!["[[CUST-001]]", "[[CUST-002]]"]
        );

        let contact = create_contact(&cfg, &ContactInput::new("Ann", "[[CUST-001]]")).unwrap();
        assert_eq!(
            frontmatter::get_str(&read_fm(&contact.path), "customer"),
            Some("[[CUST-001]]")
        );
    }

    #[test]
    fn test_task_priority_and_due_date_validated() {
        let (_tmp, cfg) = setup_repo();
        let mut input = TaskInput::new("T");
        input.priority = Some(7);
        assert!(create_task(&cfg, &input)
            .unwrap_err()
            .to_string()
            .starts_with("Invalid priority"));
        input.priority = Some(1);
        input.due_date = Some("31.12.2026".into());
        assert!(create_task(&cfg, &input)
            .unwrap_err()
            .to_string()
            .starts_with("Invalid due date"));
        // Nothing was written by the failed attempts.
        assert_eq!(entity::next_id(EntityKind::Task, &cfg).unwrap().number, 1);
    }

    #[test]
    fn test_task_with_unknown_project_allocates_nothing() {
        let (_tmp, cfg) = setup_repo();
        let mut input = TaskInput::new("Scoped");
        input.project = Some("PROJ-404".into());
        assert!(matches!(
            create_task(&cfg, &input),
            Err(McError::EntityNotFound(_))
        ));
        assert!(crate::data::collect_tasks(&cfg).unwrap().is_empty());
    }

    #[test]
    fn test_blank_status_uses_default_and_name_is_trimmed() {
        let (_tmp, cfg) = setup_repo();
        let created = create_customer_programmatic(&cfg, "  Acme  ", None, Some(""), None).unwrap();
        assert_eq!(created["name"], "Acme");
        let path = PathBuf::from(created["path"].as_str().unwrap()).join("CUST-001.md");
        assert_eq!(
            frontmatter::get_str(&read_fm(&path), "status"),
            Some("active")
        );
    }

    #[test]
    fn test_new_task_lands_in_status_folder() {
        let (_tmp, cfg) = setup_repo();
        for (n, (status, folder)) in [
            ("done", "done"),
            ("cancelled", "done"),
            ("in-progress", "todo"),
            ("backlog", "todo"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut input = TaskInput::new(format!("Beta {n}"));
            input.status = Some(status.into());
            let created = create_task(&cfg, &input).unwrap();
            assert_eq!(
                created.path.parent().unwrap(),
                cfg.tasks_dir.join(folder),
                "status {status}"
            );
            assert_eq!(
                created.path.parent().unwrap().file_name().unwrap(),
                entity::task_status_folder(status)
            );
        }
        // Finding and moving the task still works from its initial folder.
        let found = crate::data::find_entity_by_id("TASK-001", &cfg).unwrap();
        assert!(found.source_path.starts_with(cfg.tasks_dir.join("done")));
    }

    #[test]
    fn test_unsluggable_names_get_fallback_slug() {
        let (_tmp, cfg) = setup_repo();
        let created = create_task(&cfg, &TaskInput::new("!!!")).unwrap();
        assert!(created.path.ends_with("TASK-001-untitled.md"));
    }

    #[test]
    fn test_sprint_end_before_start_rejected() {
        let (_tmp, cfg) = setup_repo();
        let mut input = SprintInput::new("S");
        input.start_date = Some("2026-02-10".into());
        input.end_date = Some("2026-02-01".into());
        assert!(create_sprint(&cfg, &input)
            .unwrap_err()
            .to_string()
            .starts_with("Invalid end date"));
    }

    #[test]
    fn test_created_json_shape() {
        let (_tmp, cfg) = setup_repo();
        let c = create_customer_programmatic(&cfg, "Acme", None, None, None).unwrap();
        assert!(c.get("name").is_some() && c.get("title").is_none());
        let t = create_task_programmatic(
            &cfg, "T", None, None, None, None, None, None, None, None, None,
        )
        .unwrap();
        assert_eq!(t["id"], "TASK-001");
        assert_eq!(t["title"], "T");
        assert!(t.get("name").is_none());
    }

    #[test]
    fn test_proposal_and_research_defaults() {
        let (_tmp, cfg) = setup_repo();
        let p = create_proposal(&cfg, &ProposalInput::new("Use Rust")).unwrap();
        let fm = read_fm(&p.path);
        assert_eq!(frontmatter::get_str(&fm, "type"), Some("architecture"));
        assert_eq!(frontmatter::get_str(&fm, "status"), Some("draft"));
        assert_eq!(frontmatter::get_str(&fm, "supersedes"), Some(""));

        let r = create_research(&cfg, &ResearchInput::new("R")).unwrap();
        let fm = read_fm(&r.path.join("RES-001.md"));
        assert_eq!(frontmatter::get_string_list(&fm, "agents").len(), 4);
        for agent in DEFAULT_RESEARCH_AGENTS {
            assert!(r.path.join(agent).is_dir());
        }
    }
}
