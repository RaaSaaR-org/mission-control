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
/// `PREFIX-NNN` followed by anything: (prefix, number, rest).
static ID_PARTS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.*?)-(\d+)(.*)$").expect("static regex is valid"));

/// Sort key for IDs: prefix, then number, so `TASK-999` < `TASK-1000`.
/// IDs without a number sort by their text, after numbered IDs.
pub fn id_sort_key(id: &str) -> (&str, u64, &str) {
    match ID_PARTS_RE.captures(id) {
        Some(c) => (
            c.get(1).map_or("", |m| m.as_str()),
            c[2].parse().unwrap_or(u64::MAX),
            c.get(3).map_or("", |m| m.as_str()),
        ),
        None => (id, u64::MAX, ""),
    }
}

/// Case- and padding-insensitive form of an ID (`task-1`, `TASK-001` and
/// `task1` give the same key), for matching hand-written references.
pub fn loose_id_key(id: &str) -> Option<(String, u64)> {
    let id = frontmatter::strip_wikilink(id.trim());
    let digits = id.len() - id.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let (prefix, number) = id.split_at(id.len() - digits);
    let prefix = prefix.trim_end_matches(['-', '_', ' ']);
    Some((prefix.to_ascii_uppercase(), number.parse().ok()?))
}

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
    pub milestone: Option<&'a str>,
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

/// A `.md` file name, in any case (`Notes.MD` too).
pub(crate) fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
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
            Some(record(cfg, kind, id, fm, body, path))
        })
        .collect();
    records.sort_by(|a, b| id_sort_key(&a.id).cmp(&id_sort_key(&b.id)));
    records
}

fn record(
    cfg: &ResolvedConfig,
    kind: EntityKind,
    id: String,
    mut fm: Value,
    body: String,
    path: PathBuf,
) -> EntityRecord {
    if kind == EntityKind::Contact {
        fill_contact_customer(cfg, &mut fm, &path);
    }
    EntityRecord {
        kind,
        id,
        frontmatter: fm,
        body,
        source_path: path,
    }
}

/// Contacts live in `customers/<CUST-NNN-slug>/contacts/`, so the folder
/// already names their customer. When the file has no `customer` field, the
/// in-memory record gets it from the folder (`[[CUST-NNN]]`), so filters,
/// lists, `mc show` and JSON see it. The file is not changed.
fn fill_contact_customer(cfg: &ResolvedConfig, fm: &mut Value, path: &Path) {
    if frontmatter::get_link_str(fm, "customer").is_some_and(|c| !c.trim().is_empty()) {
        return;
    }
    let folder = path
        .parent()
        .filter(|d| d.file_name().is_some_and(|n| n == "contacts"))
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned());
    let prefix = format!("{}-", cfg.id_prefixes.customer);
    let Some(rest) = folder.as_deref().and_then(|f| f.strip_prefix(&prefix)) else {
        return;
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty() && fm.is_mapping() {
        let id = format!("{prefix}{digits}");
        frontmatter::set_str(fm, "customer", &frontmatter::wrap_wikilink(&id));
    }
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

/// Whether a task belongs to milestone `id`; an empty `id` matches tasks
/// without one (no `milestone` key, or an empty value).
fn in_milestone(fm: &Value, id: &str) -> bool {
    if id.trim().is_empty() {
        return frontmatter::get_link_str(fm, "milestone").is_none_or(|v| v.trim().is_empty());
    }
    link_is(fm, "milestone", id)
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

            milestone: None,
        }
    }

    /// Whether a task's frontmatter satisfies every set filter (case-insensitive).
    /// The sprint must be named exactly as stored; see [`Self::matches_with_sprints`].
    pub fn matches(&self, fm: &Value) -> bool {
        self.matches_with_sprints(fm, None)
    }

    /// [`Self::matches`], where a task is in the filtered sprint when its
    /// `sprint` is any of `sprints` (from [`sprint_aliases`]: the sprint's ID
    /// and its title, which older files store).
    pub fn matches_with_sprints(&self, fm: &Value, sprints: Option<&[String]>) -> bool {
        let in_sprint = |s: &str| match sprints {
            Some(aliases) => frontmatter::get_link_str(fm, "sprint")
                .is_some_and(|v| aliases.iter().any(|a| a.eq_ignore_ascii_case(v.trim()))),
            None => link_is(fm, "sprint", s),
        };
        self.status.is_none_or(|s| status_matches(fm, s))
            && self.tag.is_none_or(|t| has_tag(fm, t))
            && self.project.is_none_or(|p| links_to(fm, "projects", p))
            && self.customer.is_none_or(|c| links_to(fm, "customers", c))
            && self
                .priority
                .is_none_or(|p| get_number(fm, "priority") == Some(p))
            && self.sprint.is_none_or(in_sprint)
            && self.milestone.is_none_or(|m| in_milestone(fm, m))
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
    let sprints = filter.sprint.map(|s| sprint_aliases(cfg, s));
    let milestone = filter
        .milestone
        .map(|s| crate::commands::new::resolve_milestone(cfg, s))
        .transpose()?;
    let filter = TaskFilter {
        milestone: milestone.as_deref(),
        ..*filter
    };
    tasks.retain(|e| filter.matches_with_sprints(&e.frontmatter, sprints.as_deref()));
    Ok(tasks)
}

/// The values a task's `sprint` field can hold for the sprint that `input`
/// names: `input` itself, plus the sprint's ID and title when `input` is a
/// (loose) sprint ID or a title. New files store the ID; older ones may
/// store the title.
pub fn sprint_aliases(cfg: &ResolvedConfig, input: &str) -> Vec<String> {
    let input = frontmatter::strip_wikilink(input.trim()).trim();
    let mut aliases = vec![input.to_string()];
    if let Some(sprint) = find_sprint(cfg, input) {
        aliases.push(sprint.id.clone());
        if let Some(title) = frontmatter::get_str(&sprint.frontmatter, "title") {
            aliases.push(title.trim().to_string());
        }
    }
    aliases
}

/// The sprint that `input` names: by (loose) ID first, else by title
/// (case-insensitive).
pub fn find_sprint(cfg: &ResolvedConfig, input: &str) -> Option<EntityRecord> {
    if input.is_empty() || !cfg.entity_available(&EntityKind::Sprint) {
        return None;
    }
    let sprints = collect_entities(EntityKind::Sprint, cfg).ok()?;
    let id = crate::cli::suggest::normalize_id(input, cfg, Some(EntityKind::Sprint))
        .ok()
        .filter(|(_, kind)| *kind == EntityKind::Sprint)
        .map(|(id, _)| id);
    let lower = input.to_lowercase();
    sprints
        .iter()
        .find(|s| id.as_deref() == Some(s.id.as_str()) || s.id.eq_ignore_ascii_case(input))
        .or_else(|| {
            sprints.iter().find(|s| {
                frontmatter::get_str(&s.frontmatter, "title")
                    .is_some_and(|t| t.trim().to_lowercase() == lower)
            })
        })
        .cloned()
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
            (frontmatter::get_str(&fm, "id") == Some(id))
                .then(|| record(cfg, kind, id.to_string(), fm, body, path))
        })
        .ok_or_else(|| McError::EntityNotFound(id.to_string()))
}

/// A file named after `id` (`TASK-010-x.md`, `CUST-001/…`) whose frontmatter
/// does not parse, with the parser's message. Collection skips such files;
/// this explains why an ID that is on disk is "not found".
pub fn unreadable_file_for(id: &str, cfg: &ResolvedConfig) -> Option<(PathBuf, String)> {
    let kind = EntityKind::from_id(id, cfg).ok()?;
    let named = |s: &std::ffi::OsStr| {
        let s = s.to_string_lossy();
        s.strip_prefix(id)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '.']))
    };
    entity_files(kind, cfg)
        .into_iter()
        .filter(|p| {
            p.file_name().is_some_and(named)
                || p.parent().and_then(|d| d.file_name()).is_some_and(named)
        })
        .find_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            let message = match frontmatter::split_frontmatter(&content) {
                None => "no YAML frontmatter block".to_string(),
                Some((fm, _)) => match frontmatter::parse_in_file(&content, &fm, &path) {
                    Ok(_) => return None,
                    Err(McError::Frontmatter { message, .. }) => message,
                    Err(e) => e.to_string(),
                },
            };
            Some((path, message))
        })
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
    // Records carry the customer from their folder when the file has none.
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
        &cfg.milestones_dir,
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

            milestone: None,
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
    fn upper_case_md_extension_is_still_markdown() {
        let (_tmp, cfg) = setup_repo();
        write(
            &cfg.tasks_dir.join("todo").join("TASK-005-hand-made.MD"),
            "---\nid: TASK-005\ntitle: Hand made\nstatus: todo\n---\n",
        );
        let tasks = collect_tasks(&cfg).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "TASK-005");
        // ID allocation sees it too, so the next task doesn't reuse its ID.
        assert_eq!(
            crate::entity::next_id(EntityKind::Task, &cfg)
                .unwrap()
                .to_string(),
            "TASK-006"
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

    #[test]
    fn test_ids_sort_numerically() {
        let mut ids = vec!["TASK-1000", "TASK-101", "TASK-001", "TASK-999", "TASK-100"];
        ids.sort_by_key(|id| id_sort_key(id));
        assert_eq!(
            ids,
            ["TASK-001", "TASK-100", "TASK-101", "TASK-999", "TASK-1000"]
        );
        let (_tmp, cfg) = setup_repo();
        for id in ["TASK-1000", "TASK-999"] {
            write(
                &cfg.tasks_dir.join("todo").join(format!("{id}-x.md")),
                &format!("---\nid: {id}\ntitle: x\n---\n"),
            );
        }
        let ids: Vec<String> = collect_tasks(&cfg)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, ["TASK-999", "TASK-1000"]);
    }

    #[test]
    fn test_loose_id_keys() {
        assert_eq!(loose_id_key("task-1"), loose_id_key("TASK-001"));
        assert_eq!(loose_id_key("[[task1]]"), Some(("TASK".into(), 1)));
        assert_eq!(loose_id_key("nope"), None);
    }

    #[test]
    fn test_contacts_take_their_customer_from_the_folder() {
        let (_tmp, cfg) = setup_repo();
        new::create_customer_programmatic(&cfg, "Acme", None, None, None).unwrap();
        let dir = cfg.customers_dir.join("CUST-001-acme").join("contacts");
        write(
            &dir.join("CONT-001-ann.md"),
            "---\nid: CONT-001\nname: Ann\nstatus: active\n---\n",
        );
        let filter = ContactFilter {
            status: None,
            tag: None,
            customer: Some("CUST-001"),
        };
        let found = collect_contacts_filtered(&cfg, &filter).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            frontmatter::get_str(&found[0].frontmatter, "customer"),
            Some("[[CUST-001]]")
        );
        let one = find_entity_by_id("CONT-001", &cfg).unwrap();
        assert_eq!(
            frontmatter::get_link_str(&one.frontmatter, "customer"),
            Some("CUST-001")
        );
        // The file itself is not changed.
        assert!(!std::fs::read_to_string(dir.join("CONT-001-ann.md"))
            .unwrap()
            .contains("customer"));
    }

    #[test]
    fn test_sprint_filter_matches_id_and_legacy_title() {
        let (_tmp, cfg) = setup_repo();
        new::create_sprint(&cfg, &new::SprintInput::new("Sprint 1 - Research")).unwrap();
        let mut input = new::TaskInput::new("By ID");
        input.sprint = Some("SPR-001".into());
        new::create_task(&cfg, &input).unwrap();
        write(
            &cfg.tasks_dir.join("todo").join("TASK-002-legacy.md"),
            "---\nid: TASK-002\ntitle: Legacy\nstatus: todo\nsprint: Sprint 1 - Research\n---\n",
        );
        new::create_task(&cfg, &new::TaskInput::new("No sprint")).unwrap();
        for given in ["SPR-001", "spr-1", "sprint 1 - research"] {
            let filter = TaskFilter {
                sprint: Some(given),
                ..TaskFilter::all()
            };
            let ids: Vec<String> = collect_tasks_filtered(&cfg, &filter)
                .unwrap()
                .into_iter()
                .map(|t| t.id)
                .collect();
            assert_eq!(ids, ["TASK-001", "TASK-002"], "{given}");
        }
        // Unknown sprints still match verbatim labels.
        assert_eq!(sprint_aliases(&cfg, "2026-W05"), ["2026-W05"]);
    }

    #[test]
    fn test_unreadable_file_for_names_the_broken_file() {
        let (_tmp, cfg) = setup_repo();
        let path = cfg.tasks_dir.join("todo").join("TASK-010-broken.md");
        write(&path, "---\nid: TASK-010\ntitle: \"unterminated\n---\n");
        assert!(find_entity_by_id("TASK-010", &cfg).is_err());
        let (found, message) = unreadable_file_for("TASK-010", &cfg).unwrap();
        assert_eq!(found, path);
        // Lines count from the top of the file, as in `mc validate`.
        assert!(
            message.contains("quoted scalar at line 3 column"),
            "{message}"
        );
        assert!(unreadable_file_for("TASK-011", &cfg).is_none());
    }
}
