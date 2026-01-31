use crate::config::ResolvedConfig;
use crate::entity::{self, EntityKind};
use crate::error::{McError, McResult};
use crate::frontmatter;
use serde_json::Value as JsonValue;
use serde_yaml::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use walkdir::WalkDir;

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

/// Collect all canonical entities of a given kind (not tasks -- use collect_tasks for those).
pub fn collect_entities(kind: EntityKind, cfg: &ResolvedConfig) -> McResult<Vec<EntityRecord>> {
    if kind == EntityKind::Task {
        return collect_tasks(cfg);
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
        if path.extension().map_or(true, |e| e != "md") {
            continue;
        }

        let filename = path.file_name().unwrap_or_default().to_string_lossy();
        let is_canonical = filename == "_index.md"
            || filename == "overview.md"
            || path.parent() == Some(base);

        if !is_canonical {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(path) {
            if let Some((fm_str, body)) = frontmatter::split_frontmatter(&content) {
                if let Ok(fm) = frontmatter::parse_raw(&fm_str) {
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
                    if path.extension().map_or(true, |e| e != "md") {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Some((fm_str, body)) = frontmatter::split_frontmatter(&content) {
                            if let Ok(fm) = frontmatter::parse_raw(&fm_str) {
                                if let Some(id) = frontmatter::get_str(&fm, "id") {
                                    if id.starts_with(&id_prefix)
                                        && seen_ids.insert(id.to_string())
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
                .map_or(false, |s| s.eq_ignore_ascii_case(status))
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
            frontmatter::get_string_list(&e.frontmatter, "projects")
                .iter()
                .any(|p| p.eq_ignore_ascii_case(project))
        });
    }
    if let Some(customer) = filter.customer {
        tasks.retain(|e| {
            frontmatter::get_string_list(&e.frontmatter, "customers")
                .iter()
                .any(|c| c.eq_ignore_ascii_case(customer))
        });
    }
    if let Some(priority) = filter.priority {
        tasks.retain(|e| {
            get_number(&e.frontmatter, "priority").map_or(false, |p| p == priority)
        });
    }
    if let Some(sprint) = filter.sprint {
        tasks.retain(|e| {
            frontmatter::get_str(&e.frontmatter, "sprint")
                .map_or(false, |s| s.eq_ignore_ascii_case(sprint))
        });
    }
    if let Some(owner) = filter.owner {
        tasks.retain(|e| {
            frontmatter::get_str(&e.frontmatter, "owner")
                .map_or(false, |o| o.eq_ignore_ascii_case(owner))
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

    let base = kind.base_dir(cfg);
    if !base.is_dir() {
        return Err(McError::EntityNotFound(id.to_string()));
    }

    for entry in WalkDir::new(base).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "md") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Some((fm_str, body)) = frontmatter::split_frontmatter(&content) {
                if let Ok(fm) = frontmatter::parse_raw(&fm_str) {
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
                    if path.extension().map_or(true, |e| e != "md") {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Some((fm_str, body)) = frontmatter::split_frontmatter(&content) {
                            if let Ok(fm) = frontmatter::parse_raw(&fm_str) {
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
                .map_or(false, |s| s.eq_ignore_ascii_case(status))
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

    let base = kind.base_dir(cfg);
    let prefix = kind.prefix(cfg);
    let mut status_counts: HashMap<String, usize> = HashMap::new();
    let mut seen_ids = HashSet::new();

    if base.is_dir() {
        let id_prefix = format!("{}-", prefix);

        for entry in WalkDir::new(base).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().map_or(true, |e| e != "md") {
                continue;
            }

            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            let is_canonical = filename == "_index.md"
                || filename == "overview.md"
                || path.parent() == Some(base);
            if !is_canonical {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(path) {
                if let Some((fm_str, _)) = frontmatter::split_frontmatter(&content) {
                    if let Ok(fm) = frontmatter::parse_raw(&fm_str) {
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

/// Get recently modified files across all entity directories.
pub fn recent_activity(cfg: &ResolvedConfig, limit: usize) -> McResult<Vec<RecentFile>> {
    let mut files = Vec::new();

    let dirs: Vec<&PathBuf> = vec![
        &cfg.customers_dir,
        &cfg.projects_dir,
        &cfg.meetings_dir,
        &cfg.research_dir,
        &cfg.tasks_dir,
    ];

    for dir in &dirs {
        if !dir.is_dir() {
            continue;
        }
        for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().map_or(true, |e| e != "md") {
                continue;
            }
            if let Ok(meta) = path.metadata() {
                if let Ok(modified) = meta.modified() {
                    let (id, name) = if let Ok(content) = std::fs::read_to_string(path) {
                        if let Some((fm_str, _)) = frontmatter::split_frontmatter(&content) {
                            if let Ok(fm) = frontmatter::parse_raw(&fm_str) {
                                let id = frontmatter::get_str(&fm, "id")
                                    .unwrap_or("")
                                    .to_string();
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
        .and_then(|m| m.get(&Value::String(key.to_string())))
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
