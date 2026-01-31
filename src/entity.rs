use crate::config::ResolvedConfig;
use crate::error::{McError, McResult};
use crate::frontmatter;
use regex::Regex;
use std::fmt;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// The five entity kinds managed by MissionControl.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Customer,
    Project,
    Meeting,
    Research,
    Task,
}

impl EntityKind {
    pub fn label(&self) -> &'static str {
        match self {
            EntityKind::Customer => "customer",
            EntityKind::Project => "project",
            EntityKind::Meeting => "meeting",
            EntityKind::Research => "research",
            EntityKind::Task => "task",
        }
    }

    pub fn label_plural(&self) -> &'static str {
        match self {
            EntityKind::Customer => "customers",
            EntityKind::Project => "projects",
            EntityKind::Meeting => "meetings",
            EntityKind::Research => "research",
            EntityKind::Task => "tasks",
        }
    }

    pub fn prefix<'a>(&self, cfg: &'a ResolvedConfig) -> &'a str {
        match self {
            EntityKind::Customer => &cfg.id_prefixes.customer,
            EntityKind::Project => &cfg.id_prefixes.project,
            EntityKind::Meeting => &cfg.id_prefixes.meeting,
            EntityKind::Research => &cfg.id_prefixes.research,
            EntityKind::Task => &cfg.id_prefixes.task,
        }
    }

    pub fn base_dir<'a>(&self, cfg: &'a ResolvedConfig) -> &'a Path {
        match self {
            EntityKind::Customer => &cfg.customers_dir,
            EntityKind::Project => &cfg.projects_dir,
            EntityKind::Meeting => &cfg.meetings_dir,
            EntityKind::Research => &cfg.research_dir,
            EntityKind::Task => &cfg.tasks_dir,
        }
    }

    pub fn statuses<'a>(&self, cfg: &'a ResolvedConfig) -> &'a [String] {
        match self {
            EntityKind::Customer => &cfg.statuses.customer,
            EntityKind::Project => &cfg.statuses.project,
            EntityKind::Meeting => &cfg.statuses.meeting,
            EntityKind::Research => &cfg.statuses.research,
            EntityKind::Task => &cfg.statuses.task,
        }
    }

    pub fn from_str_loose(s: &str) -> McResult<Self> {
        match s.to_lowercase().as_str() {
            "customer" | "customers" => Ok(EntityKind::Customer),
            "project" | "projects" => Ok(EntityKind::Project),
            "meeting" | "meetings" => Ok(EntityKind::Meeting),
            "research" => Ok(EntityKind::Research),
            "task" | "tasks" => Ok(EntityKind::Task),
            _ => Err(McError::Other(format!("Unknown entity kind: {s}"))),
        }
    }

    /// Parse an entity kind from an ID prefix like "CUST-001".
    pub fn from_id(id: &str, cfg: &ResolvedConfig) -> McResult<Self> {
        if id.starts_with(&format!("{}-", cfg.id_prefixes.customer)) {
            Ok(EntityKind::Customer)
        } else if id.starts_with(&format!("{}-", cfg.id_prefixes.project)) {
            Ok(EntityKind::Project)
        } else if id.starts_with(&format!("{}-", cfg.id_prefixes.meeting)) {
            Ok(EntityKind::Meeting)
        } else if id.starts_with(&format!("{}-", cfg.id_prefixes.research)) {
            Ok(EntityKind::Research)
        } else if id.starts_with(&format!("{}-", cfg.id_prefixes.task)) {
            Ok(EntityKind::Task)
        } else {
            Err(McError::InvalidId(format!(
                "{} (expected format like {}-001, {}-002, {}-003, {}-001, or {}-001)",
                id,
                cfg.id_prefixes.customer,
                cfg.id_prefixes.project,
                cfg.id_prefixes.meeting,
                cfg.id_prefixes.research,
                cfg.id_prefixes.task,
            )))
        }
    }
}

impl fmt::Display for EntityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Formatted entity ID like "CUST-001".
#[derive(Debug, Clone)]
pub struct EntityId {
    pub prefix: String,
    pub number: u32,
}

impl EntityId {
    pub fn new(prefix: &str, number: u32) -> Self {
        Self {
            prefix: prefix.to_string(),
            number,
        }
    }

    pub fn to_string_padded(&self) -> String {
        format!("{}-{:03}", self.prefix, self.number)
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string_padded())
    }
}

/// A task location discovered on disk.
#[allow(dead_code)]
pub struct TaskLocation {
    pub tasks_dir: PathBuf,
    pub project_id: Option<String>,
    pub customer_id: Option<String>,
}

/// Collect all directories that can contain tasks:
/// global `tasks/`, each `projects/*/tasks/`, each `customers/*/tasks/`.
pub fn collect_all_task_dirs(cfg: &ResolvedConfig) -> Vec<TaskLocation> {
    let mut locations = Vec::new();

    // Global tasks dir
    locations.push(TaskLocation {
        tasks_dir: cfg.tasks_dir.clone(),
        project_id: None,
        customer_id: None,
    });

    // Project-scoped tasks
    if cfg.projects_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&cfg.projects_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                    let dir_name = entry.file_name().to_string_lossy().to_string();
                    let proj_id = extract_id_from_dirname(&dir_name, &cfg.id_prefixes.project);
                    let tasks_subdir = entry.path().join("tasks");
                    locations.push(TaskLocation {
                        tasks_dir: tasks_subdir,
                        project_id: proj_id,
                        customer_id: None,
                    });
                }
            }
        }
    }

    // Customer-scoped tasks
    if cfg.customers_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&cfg.customers_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                    let dir_name = entry.file_name().to_string_lossy().to_string();
                    let cust_id = extract_id_from_dirname(&dir_name, &cfg.id_prefixes.customer);
                    let tasks_subdir = entry.path().join("tasks");
                    locations.push(TaskLocation {
                        tasks_dir: tasks_subdir,
                        project_id: None,
                        customer_id: cust_id,
                    });
                }
            }
        }
    }

    locations
}

/// Extract an entity ID (e.g. "PROJ-001") from a directory name like "PROJ-001-robot-arm".
fn extract_id_from_dirname(dirname: &str, prefix: &str) -> Option<String> {
    let re = Regex::new(&format!(r"^({})-(\d{{3}})", regex::escape(prefix))).unwrap();
    re.captures(dirname)
        .map(|caps| format!("{}-{}", &caps[1], &caps[2]))
}

/// Scan for the next available ID for a given entity kind.
/// For directory-based entities (Customer, Project, Research): scan directory names.
/// For meetings: scan frontmatter `id` fields.
/// For tasks: scan all task locations (both `todo/` and `done/` subfolders).
/// Always returns max+1 (no gap-filling).
pub fn next_id(kind: EntityKind, cfg: &ResolvedConfig) -> McResult<EntityId> {
    let prefix = kind.prefix(cfg);
    let mut max_num: u32 = 0;

    let id_re = Regex::new(&format!(r"^{}-(\d+)", regex::escape(prefix))).unwrap();

    match kind {
        EntityKind::Customer | EntityKind::Project | EntityKind::Research => {
            let base = kind.base_dir(cfg);
            // Scan directory names
            if base.is_dir() {
                for entry in std::fs::read_dir(base)? {
                    let entry = entry?;
                    if entry.file_type()?.is_dir() {
                        let name = entry.file_name();
                        let name = name.to_string_lossy();
                        if let Some(caps) = id_re.captures(&name) {
                            if let Ok(n) = caps[1].parse::<u32>() {
                                max_num = max_num.max(n);
                            }
                        }
                    }
                }
            }
        }
        EntityKind::Meeting => {
            let base = kind.base_dir(cfg);
            // Scan frontmatter id fields in meeting .md files
            if base.is_dir() {
                for entry in WalkDir::new(base).into_iter().filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.extension().is_some_and(|e| e == "md") {
                        if let Ok(content) = std::fs::read_to_string(path) {
                            if let Some((fm_str, _)) = frontmatter::split_frontmatter(&content) {
                                if let Ok(val) = frontmatter::parse_raw(&fm_str) {
                                    if let Some(id_val) = frontmatter::get_str(&val, "id") {
                                        if let Some(caps) = id_re.captures(id_val) {
                                            if let Ok(n) = caps[1].parse::<u32>() {
                                                max_num = max_num.max(n);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        EntityKind::Task => {
            // Scan all task locations (global + per-project + per-customer)
            let locations = collect_all_task_dirs(cfg);
            for loc in &locations {
                for subfolder in &["todo", "done"] {
                    let dir = loc.tasks_dir.join(subfolder);
                    if dir.is_dir() {
                        if let Ok(entries) = std::fs::read_dir(&dir) {
                            for entry in entries.filter_map(|e| e.ok()) {
                                let name = entry.file_name();
                                let name = name.to_string_lossy();
                                if let Some(caps) = id_re.captures(&name) {
                                    if let Ok(n) = caps[1].parse::<u32>() {
                                        max_num = max_num.max(n);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(EntityId::new(prefix, max_num + 1))
}
