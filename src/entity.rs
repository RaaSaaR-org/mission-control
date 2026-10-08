use crate::config::ResolvedConfig;
use crate::data;
use crate::error::{McError, McResult};
use crate::frontmatter;
use regex::Regex;
use std::fmt;
use std::path::{Path, PathBuf};

/// The entity kinds managed by MissionControl.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Customer,
    Project,
    Meeting,
    Research,
    Task,
    Sprint,
    Milestone,
    Proposal,
    Contact,
}

impl EntityKind {
    /// Every entity kind, in display order.
    pub const ALL: [EntityKind; 9] = [
        EntityKind::Customer,
        EntityKind::Project,
        EntityKind::Meeting,
        EntityKind::Research,
        EntityKind::Task,
        EntityKind::Sprint,
        EntityKind::Milestone,
        EntityKind::Proposal,
        EntityKind::Contact,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            EntityKind::Customer => "customer",
            EntityKind::Project => "project",
            EntityKind::Meeting => "meeting",
            EntityKind::Research => "research",
            EntityKind::Task => "task",
            EntityKind::Sprint => "sprint",
            EntityKind::Milestone => "milestone",
            EntityKind::Proposal => "proposal",
            EntityKind::Contact => "contact",
        }
    }

    pub fn label_plural(&self) -> &'static str {
        match self {
            EntityKind::Customer => "customers",
            EntityKind::Project => "projects",
            EntityKind::Meeting => "meetings",
            EntityKind::Research => "research",
            EntityKind::Task => "tasks",
            EntityKind::Sprint => "sprints",
            EntityKind::Milestone => "milestones",
            EntityKind::Proposal => "proposals",
            EntityKind::Contact => "contacts",
        }
    }

    pub fn prefix<'a>(&self, cfg: &'a ResolvedConfig) -> &'a str {
        match self {
            EntityKind::Customer => &cfg.id_prefixes.customer,
            EntityKind::Project => &cfg.id_prefixes.project,
            EntityKind::Meeting => &cfg.id_prefixes.meeting,
            EntityKind::Research => &cfg.id_prefixes.research,
            EntityKind::Task => &cfg.id_prefixes.task,
            EntityKind::Sprint => &cfg.id_prefixes.sprint,
            EntityKind::Milestone => &cfg.id_prefixes.milestone,
            EntityKind::Proposal => &cfg.id_prefixes.proposal,
            EntityKind::Contact => &cfg.id_prefixes.contact,
        }
    }

    pub fn base_dir<'a>(&self, cfg: &'a ResolvedConfig) -> &'a Path {
        match self {
            EntityKind::Customer => &cfg.customers_dir,
            EntityKind::Project => &cfg.projects_dir,
            EntityKind::Meeting => &cfg.meetings_dir,
            EntityKind::Research => &cfg.research_dir,
            EntityKind::Task => &cfg.tasks_dir,
            EntityKind::Sprint => &cfg.sprints_dir,
            EntityKind::Milestone => &cfg.milestones_dir,
            EntityKind::Proposal => &cfg.proposals_dir,
            EntityKind::Contact => &cfg.customers_dir, // contacts live under customers/*/contacts/
        }
    }

    pub fn statuses<'a>(&self, cfg: &'a ResolvedConfig) -> &'a [String] {
        match self {
            EntityKind::Customer => &cfg.statuses.customer,
            EntityKind::Project => &cfg.statuses.project,
            EntityKind::Meeting => &cfg.statuses.meeting,
            EntityKind::Research => &cfg.statuses.research,
            EntityKind::Task => &cfg.statuses.task,
            EntityKind::Sprint => &cfg.statuses.sprint,
            EntityKind::Milestone => &cfg.statuses.milestone,
            EntityKind::Proposal => &cfg.statuses.proposal,
            EntityKind::Contact => &cfg.statuses.contact,
        }
    }

    pub fn from_str_loose(s: &str) -> McResult<Self> {
        match s.to_lowercase().as_str() {
            "customer" | "customers" => Ok(EntityKind::Customer),
            "project" | "projects" => Ok(EntityKind::Project),
            "meeting" | "meetings" => Ok(EntityKind::Meeting),
            "research" => Ok(EntityKind::Research),
            "task" | "tasks" => Ok(EntityKind::Task),
            "sprint" | "sprints" => Ok(EntityKind::Sprint),
            "milestone" | "milestones" => Ok(EntityKind::Milestone),
            "proposal" | "proposals" | "prop" => Ok(EntityKind::Proposal),
            "contact" | "contacts" => Ok(EntityKind::Contact),
            _ => Err(McError::usage(format!("Unknown entity kind: {s}"), None)),
        }
    }

    /// Parse an entity kind from an ID prefix like "CUST-001".
    /// When configured prefixes overlap, the longest matching prefix wins.
    pub fn from_id(id: &str, cfg: &ResolvedConfig) -> McResult<Self> {
        Self::ALL
            .into_iter()
            .filter(|k| {
                id.strip_prefix(k.prefix(cfg))
                    .is_some_and(|rest| rest.starts_with('-'))
            })
            // `max_by_key` keeps the last of equal maxima; reversing keeps the
            // first in `ALL` order when two kinds share a prefix.
            .rev()
            .max_by_key(|k| k.prefix(cfg).len())
            .ok_or_else(|| {
                let examples: Vec<String> = Self::ALL
                    .iter()
                    .map(|k| format!("{}-001", k.prefix(cfg)))
                    .collect();
                McError::InvalidId(format!(
                    "{} (expected format like {})",
                    id,
                    examples.join(", ")
                ))
            })
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

/// Immediate subdirectories of `dir`, sorted. Missing or unreadable dirs yield nothing.
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|ft| ft.is_dir()))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

/// Task statuses that end a task's lifecycle; their files live in `done/`.
/// Every other status, including custom ones from the config (`blocked`,
/// `waiting`), is active and lives in `todo/`.
pub const FINISHED_TASK_STATUSES: &[&str] = &["done", "cancelled"];

/// Whether a task with `status` is finished (see [`FINISHED_TASK_STATUSES`]).
pub fn is_finished_task_status(status: &str) -> bool {
    FINISHED_TASK_STATUSES.contains(&status)
}

/// The subfolder of a `tasks/` directory (`todo` or `done`) where a task with
/// `status` belongs. Shared by `mc new task`, `mc task move` and `mc validate`.
pub fn task_status_folder(status: &str) -> &'static str {
    if is_finished_task_status(status) {
        "done"
    } else {
        "todo"
    }
}

/// A task location discovered on disk.
pub struct TaskLocation {
    pub tasks_dir: PathBuf,
}

/// Collect all directories that can contain tasks:
/// global `tasks/`, each `projects/*/tasks/`, each `customers/*/tasks/`.
pub fn collect_all_task_dirs(cfg: &ResolvedConfig) -> Vec<TaskLocation> {
    std::iter::once(cfg.tasks_dir.clone())
        .chain(
            subdirs(&cfg.projects_dir)
                .into_iter()
                .map(|d| d.join("tasks")),
        )
        .chain(
            subdirs(&cfg.customers_dir)
                .into_iter()
                .map(|d| d.join("tasks")),
        )
        .map(|tasks_dir| TaskLocation { tasks_dir })
        .collect()
}

/// A contact directory discovered on disk.
pub struct ContactLocation {
    pub contacts_dir: PathBuf,
}

/// Collect all directories that can contain contacts: each `customers/*/contacts/`.
pub fn collect_all_contact_dirs(cfg: &ResolvedConfig) -> Vec<ContactLocation> {
    subdirs(&cfg.customers_dir)
        .into_iter()
        .map(|d| ContactLocation {
            contacts_dir: d.join("contacts"),
        })
        .collect()
}

/// Scan for the next available ID for a given entity kind.
///
/// The maximum is taken over both naming conventions on disk (entity directory
/// names like `CUST-007-acme`, file names like `TASK-012-fix.md`) and the `id`
/// fields of the entities themselves, so a renamed directory or an oddly named
/// file cannot cause an ID to be handed out twice. Meetings and proposals are
/// identified by frontmatter only (their file names are date/slug based).
/// Always returns max+1 (no gap-filling).
///
/// Reading the maximum and writing the new entity is a check-then-act pair:
/// callers that allocate an ID hold [`crate::lock`]'s repo write lock across
/// both (every `create_*` in `commands::new` does).
pub fn next_id(kind: EntityKind, cfg: &ResolvedConfig) -> McResult<EntityId> {
    let prefix = kind.prefix(cfg);
    let id_re = Regex::new(&format!(r"^{}-(\d+)", regex::escape(prefix)))
        .map_err(|e| McError::Other(format!("invalid ID prefix '{prefix}': {e}")))?;
    let number = |s: &str| -> Option<u32> { id_re.captures(s)?.get(1)?.as_str().parse().ok() };
    let name_of = |p: &Path| -> String {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };

    let names: Vec<String> = match kind {
        EntityKind::Customer
        | EntityKind::Project
        | EntityKind::Research
        | EntityKind::Sprint
        | EntityKind::Milestone => subdirs(kind.base_dir(cfg))
            .iter()
            .map(|d| name_of(d))
            .collect(),
        EntityKind::Task => data::task_files(cfg).iter().map(|p| name_of(p)).collect(),
        EntityKind::Contact => data::contact_files(cfg)
            .iter()
            .map(|p| name_of(p))
            .collect(),
        EntityKind::Meeting | EntityKind::Proposal => Vec::new(),
    };

    let ids: Vec<String> = match kind {
        // Meetings/proposals may live in nested folders that collection
        // ignores; scan every markdown file's `id`.
        EntityKind::Meeting | EntityKind::Proposal => data::md_files_below(kind.base_dir(cfg))
            .iter()
            .filter_map(|p| {
                let content = std::fs::read_to_string(p).ok()?;
                let (fm_str, _) = frontmatter::split_frontmatter(&content)?;
                let fm = frontmatter::parse_raw(&fm_str, p).ok()?;
                frontmatter::get_str(&fm, "id").map(str::to_string)
            })
            .collect(),
        _ => data::collect_entities(kind, cfg)?
            .into_iter()
            .map(|r| r.id)
            .collect(),
    };

    let max_num = names
        .iter()
        .chain(ids.iter())
        .filter_map(|s| number(s))
        .max()
        .unwrap_or(0);

    Ok(EntityId::new(prefix, max_num.saturating_add(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{init, new};
    use crate::config;
    use tempfile::TempDir;

    fn setup_repo() -> (TempDir, ResolvedConfig) {
        let tmp = TempDir::new().unwrap();
        init::run(tmp.path(), false, false, Some("T"), false, true).unwrap();
        let cfg = config::load_config(tmp.path(), config::RepoMode::Standalone).unwrap();
        (tmp, cfg)
    }

    #[test]
    fn test_from_id_all_kinds() {
        let (_tmp, cfg) = setup_repo();
        for kind in EntityKind::ALL {
            let id = format!("{}-001", kind.prefix(&cfg));
            assert_eq!(EntityKind::from_id(&id, &cfg).unwrap(), kind);
        }
        assert!(EntityKind::from_id("NOPE-001", &cfg).is_err());
        assert!(EntityKind::from_id("CUST001", &cfg).is_err());
    }

    #[test]
    fn test_from_id_longest_prefix_wins() {
        let (_tmp, mut cfg) = setup_repo();
        cfg.id_prefixes.task = "T".into();
        cfg.id_prefixes.sprint = "T-S".into();
        assert_eq!(
            EntityKind::from_id("T-S-001", &cfg).unwrap(),
            EntityKind::Sprint
        );
        assert_eq!(
            EntityKind::from_id("T-001", &cfg).unwrap(),
            EntityKind::Task
        );
        // Duplicate prefixes resolve to the earlier kind, as before.
        cfg.id_prefixes.contact = cfg.id_prefixes.customer.clone();
        assert_eq!(
            EntityKind::from_id("CUST-001", &cfg).unwrap(),
            EntityKind::Customer
        );
    }

    #[test]
    fn test_next_id_starts_at_one() {
        let (_tmp, cfg) = setup_repo();
        for kind in EntityKind::ALL {
            assert_eq!(next_id(kind, &cfg).unwrap().number, 1, "{kind}");
        }
    }

    #[test]
    fn test_next_id_respects_frontmatter_ids_in_renamed_dirs() {
        let (_tmp, cfg) = setup_repo();
        let dir = cfg.customers_dir.join("acme-renamed");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("CUST-007.md"),
            "---\nid: CUST-007\nname: Acme\n---\n",
        )
        .unwrap();
        assert_eq!(
            next_id(EntityKind::Customer, &cfg).unwrap().to_string(),
            "CUST-008"
        );
    }

    #[test]
    fn test_next_id_tasks_across_scopes_and_odd_names() {
        let (_tmp, cfg) = setup_repo();
        new::create_project_programmatic(&cfg, "P", None, Some("active"), None, None).unwrap();
        let mut input = new::TaskInput::new("Scoped");
        input.project = Some("PROJ-001".into());
        new::create_task(&cfg, &input).unwrap();
        std::fs::write(
            cfg.tasks_dir.join("todo").join("misc.md"),
            "---\nid: TASK-010\ntitle: odd\n---\n",
        )
        .unwrap();
        assert_eq!(
            next_id(EntityKind::Task, &cfg).unwrap().to_string(),
            "TASK-011"
        );
    }

    #[test]
    fn test_next_id_meetings_in_nested_folders() {
        let (_tmp, cfg) = setup_repo();
        let nested = cfg.meetings_dir.join("2025");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("old.md"), "---\nid: MTG-041\n---\n").unwrap();
        assert_eq!(
            next_id(EntityKind::Meeting, &cfg).unwrap().to_string(),
            "MTG-042"
        );
    }

    #[test]
    fn test_custom_task_statuses_are_active() {
        assert_eq!(task_status_folder("done"), "done");
        assert_eq!(task_status_folder("cancelled"), "done");
        for status in [
            "backlog",
            "todo",
            "in-progress",
            "review",
            "blocked",
            "waiting",
        ] {
            assert_eq!(task_status_folder(status), "todo", "{status}");
        }
    }

    #[test]
    fn test_collect_all_task_dirs_includes_scopes() {
        let (_tmp, cfg) = setup_repo();
        new::create_project_programmatic(&cfg, "P", None, Some("active"), None, None).unwrap();
        new::create_customer_programmatic(&cfg, "C", None, Some("active"), None).unwrap();
        let dirs: Vec<PathBuf> = collect_all_task_dirs(&cfg)
            .into_iter()
            .map(|l| l.tasks_dir)
            .collect();
        assert_eq!(dirs.len(), 3);
        assert_eq!(dirs[0], cfg.tasks_dir);
        assert_eq!(collect_all_contact_dirs(&cfg).len(), 1);
    }
}
