use crate::config::ResolvedConfig;
use crate::entity::{self, EntityKind};
use crate::error::{McError, McResult};
use crate::frontmatter;
use regex::Regex;
use serde_json::Value as JsonValue;
use serde_yaml::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::LazyLock;
use walkdir::WalkDir;

/// Matches ID-based entity filenames like `CUST-001.md`, `PROJ-002.md`.
static ID_FILENAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Z]+-\d+\.md$").unwrap());

/// A loaded entity with frontmatter, body, and source path.
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

/// Collect all canonical entities of a given kind (not tasks or contacts -- use dedicated functions).
pub fn collect_entities(kind: EntityKind, cfg: &ResolvedConfig) -> McResult<Vec<EntityRecord>> {
    if kind == EntityKind::Task {
        return collect_tasks(cfg);
    }
    if kind == EntityKind::Contact {
        return collect_contacts(cfg);
    }

    let base = kind.base_dir(cfg);
    let prefix = kind.prefix(cfg);
    let mut records = Vec::new();
    let mut seen_ids = HashSet::new();

    if !base.is_dir() {
        return Ok(records);
    }

    let id_prefix = format!("{}-", prefix);

    for entry in WalkDir::new(base).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }

        let filename = path.file_name().unwrap_or_default().to_string_lossy();
        let is_canonical = path.parent() == Some(base)
            || ID_FILENAME_RE.is_match(&filename)
            || filename == "_index.md"
            || filename == "overview.md";

        if !is_canonical {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(path) {
            if let Some((fm_str, body)) = frontmatter::split_frontmatter(&content) {
                if let Ok(fm) = frontmatter::parse_raw(&fm_str, path) {
                    if let Some(id) = frontmatter::get_str(&fm, "id") {
                        if id.starts_with(&id_prefix) && seen_ids.insert(id.to_string()) {
                            records.push(EntityRecord {
                                kind,
                                id: id.to_string(),
                                frontmatter: fm,
                                body,
                                source_path: path.to_path_buf(),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(records)
}

/// Collect all tasks from all locations (global, per-project, per-customer).
pub fn collect_tasks(cfg: &ResolvedConfig) -> McResult<Vec<EntityRecord>> {
    let locations = entity::collect_all_task_dirs(cfg);
    let prefix = &cfg.id_prefixes.task;
    let id_prefix = format!("{}-", prefix);
    let mut records = Vec::new();
    let mut seen_ids = HashSet::new();

    for loc in &locations {
        for subfolder in &["todo", "done"] {
            let dir = loc.tasks_dir.join(subfolder);
            if !dir.is_dir() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.extension().is_none_or(|e| e != "md") {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Some((fm_str, body)) = frontmatter::split_frontmatter(&content) {
                            if let Ok(fm) = frontmatter::parse_raw(&fm_str, &path) {
                                if let Some(id) = frontmatter::get_str(&fm, "id") {
                                    if id.starts_with(&id_prefix) && seen_ids.insert(id.to_string())
                                    {
                                        records.push(EntityRecord {
                                            kind: EntityKind::Task,
                                            id: id.to_string(),
                                            frontmatter: fm,
                                            body,
                                            source_path: path.to_path_buf(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    records.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(records)
}

/// Collect tasks with rich filtering support.
pub fn collect_tasks_filtered(
    cfg: &ResolvedConfig,
    filter: &TaskFilter,
) -> McResult<Vec<EntityRecord>> {
    let mut tasks = collect_tasks(cfg)?;

    if let Some(status) = filter.status {
        tasks.retain(|e| {
            frontmatter::get_str(&e.frontmatter, "status")
                .is_some_and(|s| s.eq_ignore_ascii_case(status))
        });
    }
    if let Some(tag) = filter.tag {
        tasks.retain(|e| {
            frontmatter::get_string_list(&e.frontmatter, "tags")
                .iter()
                .any(|t| t.eq_ignore_ascii_case(tag))
        });
    }
    if let Some(project) = filter.project {
        tasks.retain(|e| {
            frontmatter::get_link_list(&e.frontmatter, "projects")
                .iter()
                .any(|p| p.eq_ignore_ascii_case(project))
        });
    }
    if let Some(customer) = filter.customer {
        tasks.retain(|e| {
            frontmatter::get_link_list(&e.frontmatter, "customers")
                .iter()
                .any(|c| c.eq_ignore_ascii_case(customer))
        });
    }
    if let Some(priority) = filter.priority {
        tasks.retain(|e| get_number(&e.frontmatter, "priority") == Some(priority));
    }
    if let Some(sprint) = filter.sprint {
        tasks.retain(|e| {
            frontmatter::get_link_str(&e.frontmatter, "sprint")
                .is_some_and(|s| s.eq_ignore_ascii_case(sprint))
        });
    }
    if let Some(owner) = filter.owner {
        tasks.retain(|e| {
            frontmatter::get_str(&e.frontmatter, "owner")
                .is_some_and(|o| o.eq_ignore_ascii_case(owner))
        });
    }

    tasks.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(tasks)
}

/// Find a single entity by its ID.
pub fn find_entity_by_id(id: &str, cfg: &ResolvedConfig) -> McResult<EntityRecord> {
    let kind = EntityKind::from_id(id, cfg)?;

    if kind == EntityKind::Task {
        return find_task_by_id(id, cfg);
    }
    if kind == EntityKind::Contact {
        return find_contact_by_id(id, cfg);
    }

    let base = kind.base_dir(cfg);
    if !base.is_dir() {
        return Err(McError::EntityNotFound(id.to_string()));
    }

    for entry in WalkDir::new(base).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Some((fm_str, body)) = frontmatter::split_frontmatter(&content) {
                if let Ok(fm) = frontmatter::parse_raw(&fm_str, path) {
                    if frontmatter::get_str(&fm, "id") == Some(id) {
                        return Ok(EntityRecord {
                            kind,
                            id: id.to_string(),
                            frontmatter: fm,
                            body,
                            source_path: path.to_path_buf(),
                        });
                    }
                }
            }
        }
    }

    Err(McError::EntityNotFound(id.to_string()))
}

/// Find a task by its ID, scanning all task locations.
fn find_task_by_id(id: &str, cfg: &ResolvedConfig) -> McResult<EntityRecord> {
    let locations = entity::collect_all_task_dirs(cfg);

    for loc in &locations {
        for subfolder in &["todo", "done"] {
            let dir = loc.tasks_dir.join(subfolder);
            if !dir.is_dir() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.extension().is_none_or(|e| e != "md") {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Some((fm_str, body)) = frontmatter::split_frontmatter(&content) {
                            if let Ok(fm) = frontmatter::parse_raw(&fm_str, &path) {
                                if frontmatter::get_str(&fm, "id") == Some(id) {
                                    return Ok(EntityRecord {
                                        kind: EntityKind::Task,
                                        id: id.to_string(),
                                        frontmatter: fm,
                                        body,
                                        source_path: path.to_path_buf(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Err(McError::EntityNotFound(id.to_string()))
}

/// Collect entities with optional status and tag filters, sorted by ID.
pub fn collect_filtered(
    kind: EntityKind,
    cfg: &ResolvedConfig,
    status: Option<&str>,
    tag: Option<&str>,
) -> McResult<Vec<EntityRecord>> {
    let mut entries = collect_entities(kind, cfg)?;

    if let Some(status) = status {
        entries.retain(|e| {
            frontmatter::get_str(&e.frontmatter, "status")
                .is_some_and(|s| s.eq_ignore_ascii_case(status))
        });
    }
    if let Some(tag) = tag {
        entries.retain(|e| {
            frontmatter::get_string_list(&e.frontmatter, "tags")
                .iter()
                .any(|t| t.eq_ignore_ascii_case(tag))
        });
    }

    entries.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(entries)
}

/// Count entities by status for a given kind.
pub fn count_by_status(kind: EntityKind, cfg: &ResolvedConfig) -> McResult<StatusCounts> {
    if kind == EntityKind::Task {
        return count_tasks_by_status(cfg);
    }
    if kind == EntityKind::Contact {
        return count_contacts_by_status(cfg);
    }

    let base = kind.base_dir(cfg);
    let prefix = kind.prefix(cfg);
    let mut status_counts: HashMap<String, usize> = HashMap::new();
    let mut seen_ids = HashSet::new();

    if base.is_dir() {
        let id_prefix = format!("{}-", prefix);

        for entry in WalkDir::new(base).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }

            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            let is_canonical = path.parent() == Some(base)
                || ID_FILENAME_RE.is_match(&filename)
                || filename == "_index.md"
                || filename == "overview.md";
            if !is_canonical {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(path) {
                if let Some((fm_str, _)) = frontmatter::split_frontmatter(&content) {
                    if let Ok(fm) = frontmatter::parse_raw(&fm_str, path) {
                        if let Some(id) = frontmatter::get_str(&fm, "id") {
                            if id.starts_with(&id_prefix) && seen_ids.insert(id.to_string()) {
                                let status = frontmatter::get_str(&fm, "status")
                                    .unwrap_or("unknown")
                                    .to_string();
                                *status_counts.entry(status).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    let total: usize = status_counts.values().sum();
    let mut by_status: Vec<(String, usize)> = status_counts.into_iter().collect();
    by_status.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    Ok(StatusCounts {
        label: kind.label_plural().to_string(),
        total,
        by_status,
    })
}

/// Count tasks by status across all locations.
fn count_tasks_by_status(cfg: &ResolvedConfig) -> McResult<StatusCounts> {
    let tasks = collect_tasks(cfg)?;
    let mut status_counts: HashMap<String, usize> = HashMap::new();

    for task in &tasks {
        let status = frontmatter::get_str(&task.frontmatter, "status")
            .unwrap_or("unknown")
            .to_string();
        *status_counts.entry(status).or_insert(0) += 1;
    }

    let total: usize = status_counts.values().sum();
    let mut by_status: Vec<(String, usize)> = status_counts.into_iter().collect();
    by_status.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    Ok(StatusCounts {
        label: "tasks".to_string(),
        total,
        by_status,
    })
}

/// Collect all contacts from all customer directories.
pub fn collect_contacts(cfg: &ResolvedConfig) -> McResult<Vec<EntityRecord>> {
    let locations = entity::collect_all_contact_dirs(cfg);
    let prefix = &cfg.id_prefixes.contact;
    let id_prefix = format!("{}-", prefix);
    let mut records = Vec::new();
    let mut seen_ids = HashSet::new();

    for loc in &locations {
        if !loc.contacts_dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&loc.contacts_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "md") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some((fm_str, body)) = frontmatter::split_frontmatter(&content) {
                        if let Ok(fm) = frontmatter::parse_raw(&fm_str, &path) {
                            if let Some(id) = frontmatter::get_str(&fm, "id") {
                                if id.starts_with(&id_prefix) && seen_ids.insert(id.to_string()) {
                                    records.push(EntityRecord {
                                        kind: EntityKind::Contact,
                                        id: id.to_string(),
                                        frontmatter: fm,
                                        body,
                                        source_path: path.to_path_buf(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    records.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(records)
}

/// Find a contact by its ID, scanning all customer contact directories.
fn find_contact_by_id(id: &str, cfg: &ResolvedConfig) -> McResult<EntityRecord> {
    let locations = entity::collect_all_contact_dirs(cfg);

    for loc in &locations {
        if !loc.contacts_dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&loc.contacts_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "md") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some((fm_str, body)) = frontmatter::split_frontmatter(&content) {
                        if let Ok(fm) = frontmatter::parse_raw(&fm_str, &path) {
                            if frontmatter::get_str(&fm, "id") == Some(id) {
                                return Ok(EntityRecord {
                                    kind: EntityKind::Contact,
                                    id: id.to_string(),
                                    frontmatter: fm,
                                    body,
                                    source_path: path.to_path_buf(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    Err(McError::EntityNotFound(id.to_string()))
}

/// Count contacts by status across all customer directories.
fn count_contacts_by_status(cfg: &ResolvedConfig) -> McResult<StatusCounts> {
    let contacts = collect_contacts(cfg)?;
    let mut status_counts: HashMap<String, usize> = HashMap::new();

    for contact in &contacts {
        let status = frontmatter::get_str(&contact.frontmatter, "status")
            .unwrap_or("unknown")
            .to_string();
        *status_counts.entry(status).or_insert(0) += 1;
    }

    let total: usize = status_counts.values().sum();
    let mut by_status: Vec<(String, usize)> = status_counts.into_iter().collect();
    by_status.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    Ok(StatusCounts {
        label: "contacts".to_string(),
        total,
        by_status,
    })
}

/// Collect contacts with rich filtering support.
pub fn collect_contacts_filtered(
    cfg: &ResolvedConfig,
    filter: &ContactFilter,
) -> McResult<Vec<EntityRecord>> {
    let mut contacts = collect_contacts(cfg)?;

    if let Some(status) = filter.status {
        contacts.retain(|e| {
            frontmatter::get_str(&e.frontmatter, "status")
                .is_some_and(|s| s.eq_ignore_ascii_case(status))
        });
    }
    if let Some(tag) = filter.tag {
        contacts.retain(|e| {
            frontmatter::get_string_list(&e.frontmatter, "tags")
                .iter()
                .any(|t| t.eq_ignore_ascii_case(tag))
        });
    }
    if let Some(customer) = filter.customer {
        contacts.retain(|e| {
            frontmatter::get_link_str(&e.frontmatter, "customer")
                .is_some_and(|c| c.eq_ignore_ascii_case(customer))
        });
    }

    contacts.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(contacts)
}

/// Get recently modified files across all entity directories.
pub fn recent_activity(cfg: &ResolvedConfig, limit: usize) -> McResult<Vec<RecentFile>> {
    let mut files = Vec::new();

    let dirs: Vec<&PathBuf> = vec![
        &cfg.customers_dir,
        &cfg.projects_dir,
        &cfg.meetings_dir,
        &cfg.research_dir,
        &cfg.tasks_dir,
        &cfg.sprints_dir,
        &cfg.proposals_dir,
    ];

    for dir in &dirs {
        if !dir.is_dir() {
            continue;
        }
        for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            if let Ok(meta) = path.metadata() {
                if let Ok(modified) = meta.modified() {
                    let (id, name) = if let Ok(content) = std::fs::read_to_string(path) {
                        if let Some((fm_str, _)) = frontmatter::split_frontmatter(&content) {
                            if let Ok(fm) = frontmatter::parse_raw(&fm_str, path) {
                                let id = frontmatter::get_str(&fm, "id").unwrap_or("").to_string();
                                let name = frontmatter::get_str(&fm, "name")
                                    .or_else(|| frontmatter::get_str(&fm, "title"))
                                    .unwrap_or("")
                                    .to_string();
                                (id, name)
                            } else {
                                continue;
                            }
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    };

                    files.push(RecentFile {
                        id,
                        name,
                        modified,
                        path: path.to_path_buf(),
                    });
                }
            }
        }
    }

    files.sort_by(|a, b| b.modified.cmp(&a.modified));
    files.truncate(limit);

    Ok(files)
}

/// Get a numeric field from a YAML Mapping Value.
pub fn get_number(val: &Value, key: &str) -> Option<u32> {
    val.as_mapping()
        .and_then(|m| m.get(Value::String(key.to_string())))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
}

/// Convert serde_yaml::Value to serde_json::Value.
pub fn yaml_to_json(yaml: &Value) -> JsonValue {
    match yaml {
        Value::Null => JsonValue::Null,
        Value::Bool(b) => JsonValue::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                JsonValue::Number(i.into())
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
                    _ => format!("{:?}", k),
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

        let contact = find_contact_by_id("CONT-001", &cfg).unwrap();
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

        let counts = count_contacts_by_status(&cfg).unwrap();
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
}
