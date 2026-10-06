use crate::config::ResolvedConfig;
use crate::entity::{self, EntityKind};
use crate::error::{McError, McResult};
use crate::frontmatter;
use regex::Regex;
use serde_json::Value as JsonValue;
use serde_yaml::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

/// Matches ID-based entity filenames like `CUST-001.md`, `PROJ-002.md`.
static ID_FILENAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z]+-\d+\.md$").expect("static regex is valid"));

/// A loaded entity with frontmatter, body, and source path.
#[derive(Clone)]
pub struct EntityRecord {
    pub kind: EntityKind,
    pub id: String,
    pub frontmatter: Value,
    pub body: String,
    pub source_path: PathBuf,
}

/// Status breakdown for a single entity kind.
pub struct StatusCounts {
    pub label: String,
    pub total: usize,
    pub by_status: Vec<(String, usize)>,
}

/// A recently modified file entry.
pub struct RecentFile {
    pub id: String,
    pub name: String,
    pub modified: std::time::SystemTime,
    pub path: PathBuf,
}

/// Filters for task queries.
pub struct TaskFilter<'a> {
    pub status: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub project: Option<&'a str>,
    pub customer: Option<&'a str>,
    pub priority: Option<u32>,
    pub sprint: Option<&'a str>,
    pub owner: Option<&'a str>,
}

/// Filters for contact queries.
pub struct ContactFilter<'a> {
    pub status: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub customer: Option<&'a str>,
}

/// Read a markdown file and parse its frontmatter.
///
/// Returns `None` for unreadable files, files without a frontmatter block, and
/// invalid YAML -- collection functions skip such files (`mc validate` reports them).
fn read_parts(path: &Path) -> Option<(Value, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    let (fm_str, body) = frontmatter::split_frontmatter(&content)?;
    let fm = frontmatter::parse_raw(&fm_str, path).ok()?;
    Some((fm, body))
}

fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "md")
}

/// `.md` files directly inside `dir`, sorted by name. Missing dirs yield nothing.
fn md_files_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_markdown(p))
        .collect();
    files.sort();
    files
}

/// Candidate task files: `todo/` and `done/` under every task location.
pub fn task_files(cfg: &ResolvedConfig) -> Vec<PathBuf> {
    entity::collect_all_task_dirs(cfg)
        .iter()
        .flat_map(|loc| {
            ["todo", "done"]
                .into_iter()
                .flat_map(|sub| md_files_in(&loc.tasks_dir.join(sub)))
        })
        .collect()
}

/// Candidate contact files: every `customers/*/contacts/*.md`.
pub fn contact_files(cfg: &ResolvedConfig) -> Vec<PathBuf> {
    entity::collect_all_contact_dirs(cfg)
        .iter()
        .flat_map(|loc| md_files_in(&loc.contacts_dir))
        .collect()
}

/// Whether `path` is the canonical file of a directory-tree entity: a file
/// directly in the kind's base dir, an ID-named file (`CUST-001.md`), or an
/// `_index.md` / `overview.md`.
fn is_canonical(path: &Path, base: &Path) -> bool {
    let filename = path.file_name().unwrap_or_default().to_string_lossy();
    path.parent() == Some(base)
        || ID_FILENAME_RE.is_match(&filename)
        || filename == "_index.md"
        || filename == "overview.md"
}

/// All `.md` files below `base` in a deterministic (name-sorted) order.
pub fn md_files_below(base: &Path) -> Vec<PathBuf> {
    if !base.is_dir() {
        return Vec::new();
    }
    WalkDir::new(base)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && is_markdown(e.path()))
        .map(|e| e.into_path())
        .collect()
}

/// The files that can hold entities of `kind`.
fn entity_files(kind: EntityKind, cfg: &ResolvedConfig) -> Vec<PathBuf> {
    match kind {
        EntityKind::Task => task_files(cfg),
        EntityKind::Contact => contact_files(cfg),
        _ => {
            let base = kind.base_dir(cfg);
            md_files_below(base)
                .into_iter()
                .filter(|p| is_canonical(p, base))
                .collect()
        }
    }
}

/// Parse `files` into records of `kind`, keeping the first file per ID and
/// skipping files whose `id` lacks the kind's prefix. Sorted by ID.
fn load_records(kind: EntityKind, cfg: &ResolvedConfig, files: Vec<PathBuf>) -> Vec<EntityRecord> {
    let id_prefix = format!("{}-", kind.prefix(cfg));
    let mut seen_ids = HashSet::new();
    let mut records: Vec<EntityRecord> = files
        .into_iter()
        .filter_map(|path| {
            let (fm, body) = read_parts(&path)?;
            let id = frontmatter::get_str(&fm, "id")?.to_string();
            if !id.starts_with(&id_prefix) || !seen_ids.insert(id.clone()) {
                return None;
            }
            Some(EntityRecord {
                kind,
                id,
                frontmatter: fm,
                body,
                source_path: path,
            })
        })
        .collect();
    records.sort_by(|a, b| a.id.cmp(&b.id));
    records
}

/// Collect all canonical entities of a given kind, sorted by ID.
/// Tasks and contacts are discovered across all their scoped locations.
pub fn collect_entities(kind: EntityKind, cfg: &ResolvedConfig) -> McResult<Vec<EntityRecord>> {
    Ok(load_records(kind, cfg, entity_files(kind, cfg)))
}

/// Collect all tasks from all locations (global, per-project, per-customer).
pub fn collect_tasks(cfg: &ResolvedConfig) -> McResult<Vec<EntityRecord>> {
    collect_entities(EntityKind::Task, cfg)
}

fn status_matches(fm: &Value, status: &str) -> bool {
    frontmatter::get_str(fm, "status").is_some_and(|s| s.eq_ignore_ascii_case(status))
}

fn has_tag(fm: &Value, tag: &str) -> bool {
    frontmatter::get_string_list(fm, "tags")
        .iter()
        .any(|t| t.eq_ignore_ascii_case(tag))
}

fn links_to(fm: &Value, key: &str, target: &str) -> bool {
    frontmatter::get_link_list(fm, key)
        .iter()
        .any(|v| v.eq_ignore_ascii_case(target))
}

fn link_is(fm: &Value, key: &str, target: &str) -> bool {
    frontmatter::get_link_str(fm, key).is_some_and(|v| v.eq_ignore_ascii_case(target))
}

impl TaskFilter<'_> {
    /// A filter that matches every task.
    pub fn all() -> Self {
        TaskFilter {
            status: None,
            tag: None,
            project: None,
            customer: None,
            priority: None,
            sprint: None,
            owner: None,
        }
    }

    /// Whether a task's frontmatter satisfies every set filter (case-insensitive).
    pub fn matches(&self, fm: &Value) -> bool {
        self.status.is_none_or(|s| status_matches(fm, s))
            && self.tag.is_none_or(|t| has_tag(fm, t))
            && self.project.is_none_or(|p| links_to(fm, "projects", p))
            && self.customer.is_none_or(|c| links_to(fm, "customers", c))
            && self
                .priority
                .is_none_or(|p| get_number(fm, "priority") == Some(p))
            && self.sprint.is_none_or(|s| link_is(fm, "sprint", s))
            && self.owner.is_none_or(|o| {
                frontmatter::get_str(fm, "owner").is_some_and(|v| v.eq_ignore_ascii_case(o))
            })
    }
}

impl ContactFilter<'_> {
    /// Whether a contact's frontmatter satisfies every set filter (case-insensitive).
    pub fn matches(&self, fm: &Value) -> bool {
        self.status.is_none_or(|s| status_matches(fm, s))
            && self.tag.is_none_or(|t| has_tag(fm, t))
            && self.customer.is_none_or(|c| link_is(fm, "customer", c))
    }
}

/// Collect tasks with rich filtering support, sorted by ID.
pub fn collect_tasks_filtered(
    cfg: &ResolvedConfig,
    filter: &TaskFilter,
) -> McResult<Vec<EntityRecord>> {
    let mut tasks = collect_tasks(cfg)?;
    tasks.retain(|e| filter.matches(&e.frontmatter));
    Ok(tasks)
}

/// Find a single entity by its ID.
///
/// Files whose name starts with the ID are checked first (tasks and contacts
/// are named `TASK-001-slug.md`), so lookups usually parse a single file.
/// For directory-tree kinds, canonical files win over other files that happen
/// to carry the same `id`.
pub fn find_entity_by_id(id: &str, cfg: &ResolvedConfig) -> McResult<EntityRecord> {
    let kind = EntityKind::from_id(id, cfg)?;

    let candidates = match kind {
        EntityKind::Task => task_files(cfg),
        EntityKind::Contact => contact_files(cfg),
        _ => {
            let base = kind.base_dir(cfg);
            let (mut canonical, other): (Vec<_>, Vec<_>) = md_files_below(base)
                .into_iter()
                .partition(|p| is_canonical(p, base));
            canonical.extend(other);
            canonical
        }
    };

    let id_named = |p: &PathBuf| {
        let name_has_id = |s: &std::ffi::OsStr| {
            let s = s.to_string_lossy();
            s.strip_prefix(id)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '.']))
        };
        p.file_name().is_some_and(name_has_id)
            || p.parent()
                .and_then(|d| d.file_name())
                .is_some_and(name_has_id)
    };
    let (likely, rest): (Vec<_>, Vec<_>) = candidates.into_iter().partition(id_named);

    likely
        .into_iter()
        .chain(rest)
        .find_map(|path| {
            let (fm, body) = read_parts(&path)?;
            (frontmatter::get_str(&fm, "id") == Some(id)).then(|| EntityRecord {
                kind,
                id: id.to_string(),
                frontmatter: fm,
                body,
                source_path: path,
            })
        })
        .ok_or_else(|| McError::EntityNotFound(id.to_string()))
}

/// Collect entities with optional status and tag filters, sorted by ID.
pub fn collect_filtered(
    kind: EntityKind,
    cfg: &ResolvedConfig,
    status: Option<&str>,
    tag: Option<&str>,
) -> McResult<Vec<EntityRecord>> {
    let mut entries = collect_entities(kind, cfg)?;
    entries.retain(|e| {
        status.is_none_or(|s| status_matches(&e.frontmatter, s))
            && tag.is_none_or(|t| has_tag(&e.frontmatter, t))
    });
    Ok(entries)
}

/// Status breakdown of already-loaded records (missing status counts as
/// `unknown`), sorted by count descending, then status name.
pub fn status_counts_of(kind: EntityKind, records: &[EntityRecord]) -> StatusCounts {
    let mut status_counts: HashMap<String, usize> = HashMap::new();
    for rec in records {
        let status = frontmatter::get_str(&rec.frontmatter, "status").unwrap_or("unknown");
        *status_counts.entry(status.to_string()).or_insert(0) += 1;
    }

    let total = records.len();
    let mut by_status: Vec<(String, usize)> = status_counts.into_iter().collect();
    by_status.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    StatusCounts {
        label: kind.label_plural().to_string(),
        total,
        by_status,
    }
}

/// Count entities by status for a given kind.
pub fn count_by_status(kind: EntityKind, cfg: &ResolvedConfig) -> McResult<StatusCounts> {
    Ok(status_counts_of(kind, &collect_entities(kind, cfg)?))
}

/// Collect all contacts from all customer directories.
pub fn collect_contacts(cfg: &ResolvedConfig) -> McResult<Vec<EntityRecord>> {
    collect_entities(EntityKind::Contact, cfg)
}

/// Collect contacts with rich filtering support, sorted by ID.
pub fn collect_contacts_filtered(
    cfg: &ResolvedConfig,
    filter: &ContactFilter,
) -> McResult<Vec<EntityRecord>> {
    let mut contacts = collect_contacts(cfg)?;
    contacts.retain(|e| filter.matches(&e.frontmatter));
    Ok(contacts)
}

/// Get recently modified files across all entity directories.
///
/// Files are ranked by mtime first and only the newest ones are parsed, so
/// the cost is a `stat` per file plus `limit` (or a few more) parses.
pub fn recent_activity(cfg: &ResolvedConfig, limit: usize) -> McResult<Vec<RecentFile>> {
    let dirs = [
        &cfg.customers_dir,
        &cfg.projects_dir,
        &cfg.meetings_dir,
        &cfg.research_dir,
        &cfg.tasks_dir,
        &cfg.sprints_dir,
        &cfg.proposals_dir,
    ];

    let mut seen = HashSet::new();
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = dirs
        .iter()
        .flat_map(|dir| md_files_below(dir))
        .filter(|p| seen.insert(p.clone()))
        .filter_map(|p| {
            let modified = p.metadata().and_then(|m| m.modified()).ok()?;
            Some((modified, p))
        })
        .collect();
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    let files = candidates
        .into_iter()
        .filter_map(|(modified, path)| {
            let (fm, _) = read_parts(&path)?;
            let id = frontmatter::get_str(&fm, "id").unwrap_or("").to_string();
            let name = frontmatter::get_str(&fm, "name")
                .or_else(|| frontmatter::get_str(&fm, "title"))
                .unwrap_or("")
                .to_string();
            Some(RecentFile {
                id,
                name,
                modified,
                path,
            })
        })
        .take(limit)
        .collect();

    Ok(files)
}

/// Get a non-negative integer field from a YAML Mapping Value.
///
/// Quoted numbers (`priority: "2"`) are accepted; values beyond `u32::MAX`
/// saturate instead of wrapping so range checks still flag them.
pub fn get_number(val: &Value, key: &str) -> Option<u32> {
    let v = val
        .as_mapping()
        .and_then(|m| m.get(Value::String(key.to_string())))?;
    let n = match v {
        Value::Number(n) => n.as_u64()?,
        Value::String(s) => s.trim().parse::<u64>().ok()?,
        _ => return None,
    };
    Some(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Convert serde_yaml::Value to serde_json::Value.
pub fn yaml_to_json(yaml: &Value) -> JsonValue {
    match yaml {
        Value::Null => JsonValue::Null,
        Value::Bool(b) => JsonValue::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                JsonValue::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                JsonValue::Number(u.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map(JsonValue::Number)
                    .unwrap_or(JsonValue::Null)
            } else {
                JsonValue::Null
            }
        }
        Value::String(s) => JsonValue::String(s.clone()),
        Value::Sequence(seq) => JsonValue::Array(seq.iter().map(yaml_to_json).collect()),
        Value::Mapping(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                let key = match k {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    Value::Null => "null".to_string(),
                    other => serde_yaml::to_string(other)
                        .map(|s| s.trim_end().to_string())
                        .unwrap_or_default(),
                };
                obj.insert(key, yaml_to_json(v));
            }
            JsonValue::Object(obj)
        }
        Value::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{init, new};
    use crate::config;
    use tempfile::TempDir;

    fn setup_repo() -> (TempDir, config::ResolvedConfig) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        init::run(root, false, false, Some("TestRepo"), false, true).unwrap();
        let cfg = config::load_config(root, config::RepoMode::Standalone).unwrap();
        (tmp, cfg)
    }

    #[test]
    fn test_collect_entities_empty_repo() {
        let (_tmp, cfg) = setup_repo();

        let customers = collect_entities(EntityKind::Customer, &cfg).unwrap();
        assert!(customers.is_empty());

        let tasks = collect_tasks(&cfg).unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_collect_entities_after_creation() {
        let (_tmp, cfg) = setup_repo();

        new::create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();
        new::create_customer_programmatic(&cfg, "Beta Corp", None, Some("active"), None).unwrap();

        let customers = collect_entities(EntityKind::Customer, &cfg).unwrap();
        assert_eq!(customers.len(), 2);
    }

    #[test]
    fn test_collect_tasks_after_creation() {
        let (_tmp, cfg) = setup_repo();

        new::create_task_programmatic(
            &cfg,
            "Task A",
            None,
            None,
            None,
            Some("todo"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        new::create_task_programmatic(
            &cfg,
            "Task B",
            None,
            None,
            None,
            Some("backlog"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let tasks = collect_tasks(&cfg).unwrap();
        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn test_collect_tasks_filtered_by_status() {
        let (_tmp, cfg) = setup_repo();

        new::create_task_programmatic(
            &cfg,
            "Task A",
            None,
            None,
            None,
            Some("todo"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        new::create_task_programmatic(
            &cfg,
            "Task B",
            None,
            None,
            None,
            Some("backlog"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let filter = TaskFilter {
            status: Some("todo"),
            tag: None,
            project: None,
            customer: None,
            priority: None,
            sprint: None,
            owner: None,
        };
        let filtered = collect_tasks_filtered(&cfg, &filter).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "TASK-001");
    }

    #[test]
    fn test_find_entity_by_id() {
        let (_tmp, cfg) = setup_repo();

        new::create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();

        let entity = find_entity_by_id("CUST-001", &cfg).unwrap();
        assert_eq!(entity.id, "CUST-001");
        assert_eq!(entity.kind, EntityKind::Customer);
    }

    #[test]
    fn test_find_entity_by_id_not_found() {
        let (_tmp, cfg) = setup_repo();

        let result = find_entity_by_id("CUST-999", &cfg);
        assert!(result.is_err());
    }

    #[test]
    fn test_collect_contacts_empty() {
        let (_tmp, cfg) = setup_repo();

        let contacts = collect_contacts(&cfg).unwrap();
        assert!(contacts.is_empty());
    }

    #[test]
    fn test_collect_contacts_after_creation() {
        let (_tmp, cfg) = setup_repo();

        new::create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();
        new::create_contact_programmatic(
            &cfg,
            "Alice",
            "CUST-001",
            Some("VP"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        new::create_contact_programmatic(
            &cfg,
            "Bob",
            "CUST-001",
            Some("CTO"),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let contacts = collect_contacts(&cfg).unwrap();
        assert_eq!(contacts.len(), 2);
    }

    #[test]
    fn test_find_contact_by_id() {
        let (_tmp, cfg) = setup_repo();

        new::create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();
        new::create_contact_programmatic(&cfg, "Alice", "CUST-001", None, None, None, None, None)
            .unwrap();

        let contact = find_entity_by_id("CONT-001", &cfg).unwrap();
        assert_eq!(contact.id, "CONT-001");
        assert_eq!(contact.kind, EntityKind::Contact);
    }

    #[test]
    fn test_collect_contacts_filtered_by_customer() {
        let (_tmp, cfg) = setup_repo();

        new::create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();
        new::create_customer_programmatic(&cfg, "Beta", None, Some("active"), None).unwrap();
        new::create_contact_programmatic(&cfg, "Alice", "CUST-001", None, None, None, None, None)
            .unwrap();
        new::create_contact_programmatic(&cfg, "Bob", "CUST-002", None, None, None, None, None)
            .unwrap();

        let filter = ContactFilter {
            status: None,
            tag: None,
            customer: Some("CUST-001"),
        };
        let filtered = collect_contacts_filtered(&cfg, &filter).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "CONT-001");
    }

    #[test]
    fn test_count_contacts_by_status() {
        let (_tmp, cfg) = setup_repo();

        new::create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();
        new::create_contact_programmatic(
            &cfg,
            "Alice",
            "CUST-001",
            None,
            None,
            None,
            Some("active"),
            None,
        )
        .unwrap();
        new::create_contact_programmatic(
            &cfg,
            "Bob",
            "CUST-001",
            None,
            None,
            None,
            Some("inactive"),
            None,
        )
        .unwrap();

        let counts = count_by_status(EntityKind::Contact, &cfg).unwrap();
        assert_eq!(counts.total, 2);
        assert!(counts
            .by_status
            .iter()
            .any(|(s, c)| s == "active" && *c == 1));
        assert!(counts
            .by_status
            .iter()
            .any(|(s, c)| s == "inactive" && *c == 1));
    }

    #[test]
    fn test_count_by_status() {
        let (_tmp, cfg) = setup_repo();

        new::create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();
        new::create_customer_programmatic(&cfg, "Beta", None, Some("active"), None).unwrap();
        new::create_customer_programmatic(&cfg, "Gamma", None, Some("inactive"), None).unwrap();

        let counts = count_by_status(EntityKind::Customer, &cfg).unwrap();
        assert_eq!(counts.total, 3);
        assert!(counts
            .by_status
            .iter()
            .any(|(s, c)| s == "active" && *c == 2));
        assert!(counts
            .by_status
            .iter()
            .any(|(s, c)| s == "inactive" && *c == 1));
    }

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn test_collect_skips_malformed_files_without_panicking() {
        let (_tmp, cfg) = setup_repo();
        new::create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();
        let dir = cfg.customers_dir.join("CUST-009-broken");
        write(&dir.join("CUST-009.md"), "---\nid: [unclosed\n---\n");
        write(
            &cfg.customers_dir.join("no-frontmatter.md"),
            "# just text\n",
        );
        write(&cfg.customers_dir.join("empty.md"), "");
        write(
            &cfg.customers_dir.join("wrong-prefix.md"),
            "---\nid: PROJ-001\n---\n",
        );

        let customers = collect_entities(EntityKind::Customer, &cfg).unwrap();
        assert_eq!(customers.len(), 1);
        assert_eq!(customers[0].id, "CUST-001");
        assert_eq!(
            count_by_status(EntityKind::Customer, &cfg).unwrap().total,
            1
        );
    }

    #[test]
    fn test_collect_entities_sorted_and_deduplicated() {
        let (_tmp, cfg) = setup_repo();
        write(
            &cfg.meetings_dir.join("b.md"),
            "---\nid: MTG-002\ntitle: B\n---\n",
        );
        write(
            &cfg.meetings_dir.join("a.md"),
            "---\nid: MTG-001\ntitle: A\n---\n",
        );
        write(
            &cfg.meetings_dir.join("c.md"),
            "---\nid: MTG-001\ntitle: Duplicate\n---\n",
        );
        let meetings = collect_entities(EntityKind::Meeting, &cfg).unwrap();
        let ids: Vec<&str> = meetings.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["MTG-001", "MTG-002"]);
        assert_eq!(
            frontmatter::get_str(&meetings[0].frontmatter, "title"),
            Some("A")
        );
    }

    #[test]
    fn test_find_entity_prefers_canonical_file() {
        let (_tmp, cfg) = setup_repo();
        new::create_research_programmatic(&cfg, "LLMs", None, Some("claude"), None).unwrap();
        let dir = std::fs::read_dir(&cfg.research_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with("RES-001"))
            .unwrap()
            .path();
        // A notes file in a subfolder that re-uses the ID must not shadow the entity.
        write(
            &dir.join("claude").join("a-notes.md"),
            "---\nid: RES-001\ntitle: Notes\n---\n",
        );
        let rec = find_entity_by_id("RES-001", &cfg).unwrap();
        assert_eq!(rec.source_path, dir.join("RES-001.md"));
        assert_eq!(
            frontmatter::get_str(&rec.frontmatter, "title"),
            Some("LLMs")
        );
    }

    #[test]
    fn test_find_task_with_unconventional_filename() {
        let (_tmp, cfg) = setup_repo();
        write(
            &cfg.tasks_dir.join("todo").join("misc.md"),
            "---\nid: TASK-042\ntitle: Odd\nstatus: todo\n---\n",
        );
        let rec = find_entity_by_id("TASK-042", &cfg).unwrap();
        assert_eq!(rec.kind, EntityKind::Task);
        assert!(find_entity_by_id("TASK-004", &cfg).is_err());
    }

    #[test]
    fn test_task_filter_matches() {
        let fm = frontmatter::parse_raw(
            "status: In-Progress\npriority: '2'\nprojects: ['[[PROJ-001]]']\nsprint: '[[SPR-003]]'\nowner: Alice\ntags: [Backend]",
            Path::new("t.md"),
        )
        .unwrap();
        let mut f = TaskFilter::all();
        assert!(f.matches(&fm));
        f.status = Some("in-progress");
        f.priority = Some(2);
        f.project = Some("proj-001");
        f.sprint = Some("SPR-003");
        f.owner = Some("alice");
        f.tag = Some("backend");
        assert!(f.matches(&fm));
        f.customer = Some("CUST-001");
        assert!(!f.matches(&fm));
    }

    #[test]
    fn test_get_number_variants() {
        let fm = frontmatter::parse_raw(
            "a: 3\nb: '4'\nc: -1\nd: 99999999999\ne: high\nf: 2.5",
            Path::new("t.md"),
        )
        .unwrap();
        assert_eq!(get_number(&fm, "a"), Some(3));
        assert_eq!(get_number(&fm, "b"), Some(4));
        assert_eq!(get_number(&fm, "c"), None);
        assert_eq!(get_number(&fm, "d"), Some(u32::MAX));
        assert_eq!(get_number(&fm, "e"), None);
        assert_eq!(get_number(&fm, "f"), None);
        assert_eq!(get_number(&fm, "missing"), None);
    }

    #[test]
    fn test_yaml_to_json_non_string_keys() {
        let fm = frontmatter::parse_raw(
            "priorities:\n  1: critical\n  2: high\nbig: 18446744073709551615",
            Path::new("c.yml"),
        )
        .unwrap();
        let json = yaml_to_json(&fm);
        assert_eq!(json["priorities"]["1"], "critical");
        assert_eq!(json["priorities"]["2"], "high");
        assert_eq!(json["big"], serde_json::json!(u64::MAX));
    }

    #[test]
    fn test_recent_activity_limit_and_order() {
        let (_tmp, cfg) = setup_repo();
        for name in ["One", "Two", "Three"] {
            new::create_customer_programmatic(&cfg, name, None, Some("active"), None).unwrap();
        }
        let newest = cfg.customers_dir.join("CUST-002-two").join("CUST-002.md");
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&newest)
            .unwrap()
            .set_modified(later)
            .unwrap();

        let recent = recent_activity(&cfg, 2).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id, "CUST-002");
        assert_eq!(recent[0].name, "Two");
    }

    #[test]
    fn test_status_counts_of_records() {
        let (_tmp, cfg) = setup_repo();
        new::create_customer_programmatic(&cfg, "A", None, Some("active"), None).unwrap();
        new::create_customer_programmatic(&cfg, "B", None, Some("inactive"), None).unwrap();
        new::create_customer_programmatic(&cfg, "C", None, Some("active"), None).unwrap();
        let recs = collect_entities(EntityKind::Customer, &cfg).unwrap();
        let counts = status_counts_of(EntityKind::Customer, &recs);
        assert_eq!(counts.label, "customers");
        assert_eq!(counts.total, 3);
        assert_eq!(
            counts.by_status,
            vec![("active".to_string(), 2), ("inactive".to_string(), 1)]
        );
    }
}
