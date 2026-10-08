use crate::cli::suggest;
use crate::cli::ui;
use crate::config::{RepoMode, ResolvedConfig};
use crate::data;
use crate::entity::{self, EntityKind};
use crate::error::{McError, McResult};
use crate::frontmatter;
use crate::util;
use colored::*;
use regex::Regex;
use serde::Serialize;
use serde_yaml::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// How serious an issue is. Errors fail `mc validate`; warnings (e.g. a link
/// to a missing entity) are reported but don't change the exit code.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Serialize)]
pub struct ValidationIssue {
    pub path: String,
    pub check: String,
    pub severity: Severity,
    pub message: String,
}

impl ValidationIssue {
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// `(id, file)` for every entity file validated, for the duplicate-ID check.
type SeenIds = Vec<(String, PathBuf)>;

pub fn run(cfg: &ResolvedConfig) -> McResult<()> {
    let issues = validate_programmatic(cfg)?;
    let errors = issues.iter().filter(|i| i.is_error()).count();
    let warnings = issues.len() - errors;
    let result = if errors == 0 {
        Ok(())
    } else {
        Err(McError::ValidationFailed(errors))
    };

    if ui::get().json {
        let out = serde_json::json!({
            "ok": errors == 0,
            "count": issues.len(),
            "errors": errors,
            "warnings": warnings,
            "issues": issues,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return result;
    }

    if issues.is_empty() {
        ui::success("All checks passed".bold());
        return Ok(());
    }

    // Group by file (relative to the repo root), preserving discovery order.
    let mut groups: Vec<(String, Vec<&ValidationIssue>)> = Vec::new();
    for issue in &issues {
        let path = display_path(&issue.path, cfg);
        match groups.iter_mut().find(|(p, _)| *p == path) {
            Some((_, list)) => list.push(issue),
            None => groups.push((path, vec![issue])),
        }
    }

    let g = ui::glyphs();
    let check_w = issues.iter().map(|i| i.check.len()).max().unwrap_or(0);
    let mut counts = Vec::new();
    if errors > 0 {
        counts.push(ui::count(errors, "error", "errors"));
    }
    if warnings > 0 {
        counts.push(ui::count(warnings, "warning", "warnings"));
    }
    let mark = if errors > 0 {
        g.err.red().bold()
    } else {
        g.warn.yellow().bold()
    };
    println!(
        "{} {} in {}\n",
        mark,
        counts.join(", ").bold(),
        ui::count(groups.len(), "file", "files")
    );
    for (path, list) in &groups {
        println!("  {}", ui::clean(path).bold());
        for issue in list {
            let check = if issue.is_error() {
                issue.check.red().to_string()
            } else {
                issue.check.yellow().to_string()
            };
            println!(
                "    {}  {}",
                ui::pad(&check, check_w),
                ui::clean(&issue.message)
            );
        }
        println!();
    }

    // Per-check summary helps spot systemic problems (e.g. a renamed status).
    let mut by_check: Vec<(&str, usize)> = Vec::new();
    for issue in &issues {
        match by_check.iter_mut().find(|(c, _)| *c == issue.check) {
            Some((_, n)) => *n += 1,
            None => by_check.push((&issue.check, 1)),
        }
    }
    if by_check.len() > 1 {
        let parts: Vec<String> = by_check.iter().map(|(c, n)| format!("{c} {n}")).collect();
        println!("  {}\n", parts.join(&format!(" {} ", g.sep)).dimmed());
    }
    result
}

/// Issue paths are a mix of absolute paths and bare names; show them relative.
fn display_path(path: &str, cfg: &ResolvedConfig) -> String {
    Path::new(path)
        .strip_prefix(&cfg.root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.to_string())
}

/// Run validation and return structured issues without printing.
pub fn validate_programmatic(cfg: &ResolvedConfig) -> McResult<Vec<ValidationIssue>> {
    let mut issues: Vec<ValidationIssue> = Vec::new();
    let mut ids: SeenIds = Vec::new();

    if cfg.mode == RepoMode::Standalone {
        validate_entity_dirs(EntityKind::Customer, cfg, &mut issues, &mut ids)?;
        validate_entity_dirs(EntityKind::Project, cfg, &mut issues, &mut ids)?;
        validate_contacts(cfg, &mut issues, &mut ids)?;
    }
    validate_meetings(cfg, &mut issues, &mut ids)?;
    validate_entity_dirs(EntityKind::Research, cfg, &mut issues, &mut ids)?;
    validate_entity_dirs(EntityKind::Sprint, cfg, &mut issues, &mut ids)?;
    validate_entity_dirs(EntityKind::Milestone, cfg, &mut issues, &mut ids)?;
    validate_proposals(cfg, &mut issues, &mut ids)?;
    validate_tasks(cfg, &mut issues, &mut ids)?;
    validate_duplicate_ids(cfg, ids, &mut issues);
    validate_references(cfg, &mut issues)?;

    Ok(issues)
}

fn validate_entity_dirs(
    kind: EntityKind,
    cfg: &ResolvedConfig,
    issues: &mut Vec<ValidationIssue>,
    ids: &mut SeenIds,
) -> McResult<()> {
    let base = kind.base_dir(cfg);
    let prefix = kind.prefix(cfg);

    if !base.is_dir() {
        return Ok(());
    }

    // Check folder naming: PREFIX-NNN-slug
    let dir_re = Regex::new(&format!(
        r"^{}-\d{{3,}}-[a-z0-9]+(-[a-z0-9]+)*$",
        regex::escape(prefix)
    ))
    .expect("regex with escaped prefix is always valid");

    // Regex to extract entity ID from directory name (e.g. "CUST-001" from "CUST-001-acme")
    let id_re = Regex::new(&format!(r"^({}-\d+)", regex::escape(prefix)))
        .expect("regex with escaped prefix is always valid");

    for entry in std::fs::read_dir(base)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().to_string();

        // Check 1: folder naming regex
        if !dir_re.is_match(&dir_name) {
            issues.push(ValidationIssue {
                path: dir_name.clone(),
                check: "folder-naming".into(),
                severity: Severity::Error,
                message: format!(
                    "Directory name does not match expected pattern: {}-NNN-slug",
                    prefix
                ),
            });
        }

        // Check for ID-based filename (e.g. CUST-001.md), falling back to legacy names
        let index_file = if let Some(caps) = id_re.captures(&dir_name) {
            let id_file = entry.path().join(format!("{}.md", &caps[1]));
            if id_file.is_file() {
                id_file
            } else {
                // Backward compat: try legacy filenames
                match kind {
                    EntityKind::Project => entry.path().join("overview.md"),
                    _ => entry.path().join("_index.md"),
                }
            }
        } else {
            match kind {
                EntityKind::Project => entry.path().join("overview.md"),
                _ => entry.path().join("_index.md"),
            }
        };

        if !index_file.is_file() {
            issues.push(ValidationIssue {
                path: index_file.display().to_string(),
                check: "missing-index".into(),
                severity: Severity::Error,
                message: "Required index file not found".into(),
            });
            continue;
        }

        // Validate frontmatter
        if let Some(id) = validate_frontmatter_file(&index_file, kind, prefix, cfg, issues) {
            ids.push((id, index_file));
        }
    }

    Ok(())
}

fn validate_meetings(
    cfg: &ResolvedConfig,
    issues: &mut Vec<ValidationIssue>,
    ids: &mut SeenIds,
) -> McResult<()> {
    let base = &cfg.meetings_dir;
    if !base.is_dir() {
        return Ok(());
    }

    let filename_re =
        Regex::new(r"^\d{4}-\d{2}-\d{2}-.+\.md$").expect("static regex pattern is always valid");

    for entry in WalkDir::new(base)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_dir() || !data::is_markdown(path) {
            continue;
        }

        let Some(fname) = path.file_name() else {
            continue;
        };
        let filename = fname.to_string_lossy().to_string();

        // Check meeting filename pattern
        if !filename_re.is_match(&filename) {
            issues.push(ValidationIssue {
                path: filename.clone(),
                check: "meeting-filename".into(),
                severity: Severity::Error,
                message: "Meeting filename does not match YYYY-MM-DD-slug.md pattern".into(),
            });
        }

        if let Some(id) = validate_frontmatter_file(
            path,
            EntityKind::Meeting,
            &cfg.id_prefixes.meeting,
            cfg,
            issues,
        ) {
            ids.push((id, path.to_path_buf()));
        }
    }

    Ok(())
}

fn validate_proposals(
    cfg: &ResolvedConfig,
    issues: &mut Vec<ValidationIssue>,
    ids: &mut SeenIds,
) -> McResult<()> {
    let base = &cfg.proposals_dir;
    if !base.is_dir() {
        return Ok(());
    }

    let prefix = &cfg.id_prefixes.proposal;
    let filename_re = Regex::new(&format!(
        r"^{}-\d{{3,}}-[a-z0-9]+(-[a-z0-9]+)*\.md$",
        regex::escape(prefix)
    ))
    .expect("regex with escaped prefix is always valid");

    for entry in WalkDir::new(base)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_dir() || !data::is_markdown(path) {
            continue;
        }

        let Some(fname) = path.file_name() else {
            continue;
        };
        let filename = fname.to_string_lossy().to_string();

        if !filename_re.is_match(&filename) {
            issues.push(ValidationIssue {
                path: filename.clone(),
                check: "proposal-filename".into(),
                severity: Severity::Error,
                message: format!(
                    "Proposal filename does not match {}-NNN-slug.md pattern",
                    prefix
                ),
            });
        }

        if let Some(id) = validate_frontmatter_file(path, EntityKind::Proposal, prefix, cfg, issues)
        {
            ids.push((id, path.to_path_buf()));
        }
    }

    Ok(())
}

/// Validate all task files across all locations.
fn validate_tasks(
    cfg: &ResolvedConfig,
    issues: &mut Vec<ValidationIssue>,
    ids: &mut SeenIds,
) -> McResult<()> {
    let locations = entity::collect_all_task_dirs(cfg);
    let prefix = &cfg.id_prefixes.task;
    let filename_re = Regex::new(&format!(
        r"^{}-\d{{3,}}-[a-z0-9]+(-[a-z0-9]+)*\.md$",
        regex::escape(prefix)
    ))
    .expect("regex with escaped prefix is always valid");

    for loc in &locations {
        if !loc.tasks_dir.is_dir() {
            continue;
        }

        // Check that only todo/ and done/ subfolders exist
        if let Ok(entries) = std::fs::read_dir(&loc.tasks_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name != "todo" && name != "done" {
                        issues.push(ValidationIssue {
                            path: entry.path().display().to_string(),
                            check: "task-subfolder".into(),
                            severity: Severity::Error,
                            message: format!(
                                "Unexpected subfolder '{}' in tasks directory (expected only 'todo' and 'done')",
                                name
                            ),
                        });
                    }
                }
            }
        }

        for subfolder in ["todo", "done"] {
            let dir = loc.tasks_dir.join(subfolder);
            if !dir.is_dir() {
                continue;
            }

            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if !data::is_markdown(&path) {
                        continue;
                    }

                    let Some(fname) = path.file_name() else {
                        continue;
                    };
                    let filename = fname.to_string_lossy().to_string();

                    // Check filename pattern
                    if !filename_re.is_match(&filename) {
                        issues.push(ValidationIssue {
                            path: path.display().to_string(),
                            check: "task-filename".into(),
                            severity: Severity::Error,
                            message: format!(
                                "Task filename does not match {}-NNN-slug.md pattern",
                                prefix
                            ),
                        });
                    }

                    // Validate frontmatter
                    if let Some(id) =
                        validate_task_frontmatter_file(&path, prefix, cfg, subfolder, issues)
                    {
                        ids.push((id, path));
                    }
                }
            }
        }
    }

    Ok(())
}

/// Validate all contact files across customer directories.
fn validate_contacts(
    cfg: &ResolvedConfig,
    issues: &mut Vec<ValidationIssue>,
    ids: &mut SeenIds,
) -> McResult<()> {
    let locations = entity::collect_all_contact_dirs(cfg);
    let prefix = &cfg.id_prefixes.contact;
    let filename_re = Regex::new(&format!(
        r"^{}-\d{{3,}}-[a-z0-9]+(-[a-z0-9]+)*\.md$",
        regex::escape(prefix)
    ))
    .expect("regex with escaped prefix is always valid");

    for loc in &locations {
        if !loc.contacts_dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&loc.contacts_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if !data::is_markdown(&path) {
                    continue;
                }

                let Some(fname) = path.file_name() else {
                    continue;
                };
                let filename = fname.to_string_lossy().to_string();

                if !filename_re.is_match(&filename) {
                    issues.push(ValidationIssue {
                        path: path.display().to_string(),
                        check: "contact-filename".into(),
                        severity: Severity::Error,
                        message: format!(
                            "Contact filename does not match {}-NNN-slug.md pattern",
                            prefix
                        ),
                    });
                }

                if let Some(id) =
                    validate_frontmatter_file(&path, EntityKind::Contact, prefix, cfg, issues)
                {
                    ids.push((id, path));
                }
            }
        }
    }

    Ok(())
}

/// Read `path` and parse its frontmatter, reporting read errors, a missing
/// frontmatter block, invalid YAML and a missing `id`. Returns the parsed
/// frontmatter and the ID when all of that is fine.
fn read_frontmatter(path: &Path, issues: &mut Vec<ValidationIssue>) -> Option<(Value, String)> {
    let path_str = path.display().to_string();
    let mut fail = |check: &str, message: String| {
        issues.push(ValidationIssue {
            path: path_str.clone(),
            check: check.into(),
            severity: Severity::Error,
            message,
        });
    };

    let Ok(content) = std::fs::read_to_string(path) else {
        fail("read-error", "Could not read file".into());
        return None;
    };

    // Check 2: frontmatter presence
    let Some((fm_str, _body)) = frontmatter::split_frontmatter(&content) else {
        fail("frontmatter-presence", "No YAML frontmatter found".into());
        return None;
    };

    // Check 3: YAML validity
    let fm: Value = match serde_yaml::from_str(&fm_str) {
        Ok(v) => v,
        Err(e) => {
            fail(
                "yaml-validity",
                format!(
                    "Invalid YAML in frontmatter: {}",
                    frontmatter::yaml_error_in_file(&content, &e)
                ),
            );
            return None;
        }
    };

    // Check 4: required 'id' field
    let Some(id) = frontmatter::get_str(&fm, "id").map(str::to_string) else {
        fail("required-fields", "Missing required 'id' field".into());
        return None;
    };
    Some((fm, id))
}

fn id_prefix_issue(path: &Path, id: &str, prefix: &str) -> Option<ValidationIssue> {
    (!id.starts_with(&format!("{}-", prefix))).then(|| ValidationIssue {
        path: path.display().to_string(),
        check: "id-consistency".into(),
        severity: Severity::Error,
        message: format!(
            "ID '{}' does not start with expected prefix '{}-'",
            id, prefix
        ),
    })
}

/// Validate a task file found in the `subfolder` (`todo` or `done`) of a
/// tasks directory. Returns the task's ID if it has one.
fn validate_task_frontmatter_file(
    path: &Path,
    prefix: &str,
    cfg: &ResolvedConfig,
    subfolder: &str,
    issues: &mut Vec<ValidationIssue>,
) -> Option<String> {
    let path_str = path.display().to_string();
    let (fm, id) = read_frontmatter(path, issues)?;
    issues.extend(id_prefix_issue(path, &id, prefix));

    // Required: title
    if frontmatter::get_str(&fm, "title").is_none() {
        issues.push(ValidationIssue {
            path: path_str.clone(),
            check: "required-fields".into(),
            severity: Severity::Error,
            message: "Missing required 'title' field".into(),
        });
    }

    // Status validity
    if let Some(status) = frontmatter::get_str(&fm, "status") {
        let valid_statuses = EntityKind::Task.statuses(cfg);
        if !valid_statuses.iter().any(|s| s == status) {
            issues.push(ValidationIssue {
                path: path_str.clone(),
                check: "status-validity".into(),
                severity: Severity::Error,
                message: format!(
                    "Invalid status '{}', expected one of: {}",
                    status,
                    valid_statuses.join(", ")
                ),
            });
        } else {
            // Folder↔status sync: the same rule `mc new` and `mc task move` use.
            let expected = entity::task_status_folder(status);
            if expected != subfolder {
                issues.push(ValidationIssue {
                    path: path_str.clone(),
                    check: "folder-status-sync".into(),
                    severity: Severity::Error,
                    message: format!(
                        "Status '{status}' belongs in '{expected}/' but the file is in '{subfolder}/' (fix with `mc task move {id} {status}`)"
                    ),
                });
            }
        }
    }

    // Priority validity (1-4)
    if let Some(priority) = data::get_number(&fm, "priority") {
        if !(1..=4).contains(&priority) {
            issues.push(ValidationIssue {
                path: path_str.clone(),
                check: "priority-range".into(),
                severity: Severity::Error,
                message: format!(
                    "Priority {} is out of range (expected 1-4: 1=critical, 2=high, 3=medium, 4=low)",
                    priority
                ),
            });
        }
    }
    Some(id)
}

/// Validate a non-task entity file. Returns the entity's ID if it has one.
fn validate_frontmatter_file(
    path: &Path,
    kind: EntityKind,
    prefix: &str,
    cfg: &ResolvedConfig,
    issues: &mut Vec<ValidationIssue>,
) -> Option<String> {
    let path_str = path.display().to_string();
    let (fm, id) = read_frontmatter(path, issues)?;

    // Check 5: ID starts with correct prefix
    issues.extend(id_prefix_issue(path, &id, prefix));

    // Check 6: required name/title field
    let has_name = match kind {
        EntityKind::Customer | EntityKind::Project | EntityKind::Contact => {
            frontmatter::get_str(&fm, "name").is_some()
        }
        EntityKind::Meeting
        | EntityKind::Research
        | EntityKind::Task
        | EntityKind::Sprint
        | EntityKind::Milestone
        | EntityKind::Proposal => frontmatter::get_str(&fm, "title").is_some(),
    };
    if !has_name {
        let field = match kind {
            EntityKind::Customer | EntityKind::Project | EntityKind::Contact => "name",
            _ => "title",
        };
        issues.push(ValidationIssue {
            path: path_str.clone(),
            check: "required-fields".into(),
            severity: Severity::Error,
            message: format!("Missing required '{}' field", field),
        });
    }

    // Check 7: status validity
    if let Some(status) = frontmatter::get_str(&fm, "status") {
        let valid_statuses = kind.statuses(cfg);
        if !valid_statuses.iter().any(|s| s == status) {
            issues.push(ValidationIssue {
                path: path_str.clone(),
                check: "status-validity".into(),
                severity: Severity::Error,
                message: format!(
                    "Invalid status '{}', expected one of: {}",
                    status,
                    valid_statuses.join(", ")
                ),
            });
        }
    }

    // Check 8: slug consistency (for directory-based entities)
    if kind != EntityKind::Meeting
        && kind != EntityKind::Task
        && kind != EntityKind::Sprint
        && kind != EntityKind::Proposal
        && kind != EntityKind::Contact
    {
        if let Some(slug) = frontmatter::get_str(&fm, "slug") {
            // Check that the parent directory contains the slug
            if let Some(parent) = path.parent() {
                let dir_name = parent.file_name().unwrap_or_default().to_string_lossy();
                if !dir_name.contains(slug) {
                    issues.push(ValidationIssue {
                        path: path_str,
                        check: "slug-consistency".into(),
                        severity: Severity::Error,
                        message: format!(
                            "Slug '{}' does not match directory name '{}'",
                            slug, dir_name
                        ),
                    });
                }
            }
        }
    }
    Some(id)
}

/// Report every file whose ID another file also uses. Collection keeps only
/// the first file per ID, so the others silently vanish from every view.
fn validate_duplicate_ids(cfg: &ResolvedConfig, ids: SeenIds, issues: &mut Vec<ValidationIssue>) {
    let mut by_id: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for (id, path) in ids {
        by_id.entry(id).or_default().push(path);
    }
    for (id, paths) in by_id.iter().filter(|(_, p)| p.len() > 1) {
        for path in paths {
            let others: Vec<String> = paths
                .iter()
                .filter(|p| *p != path)
                .map(|p| display_path(&p.display().to_string(), cfg))
                .collect();
            issues.push(ValidationIssue {
                path: path.display().to_string(),
                check: "duplicate-id".into(),
                severity: Severity::Error,
                message: format!(
                    "ID '{id}' is also used by {} (mc shows only one of them; give the others new IDs)",
                    others.join(", ")
                ),
            });
        }
    }
}

/// Frontmatter fields that link to other entities, and the kind they link to.
const REFERENCE_FIELDS: &[(&str, EntityKind)] = &[
    ("customers", EntityKind::Customer),
    ("customer", EntityKind::Customer),
    ("projects", EntityKind::Project),
    ("project", EntityKind::Project),
    ("depends_on", EntityKind::Task),
    ("sprint", EntityKind::Sprint),
    ("milestone", EntityKind::Milestone),
    ("supersedes", EntityKind::Proposal),
    ("superseded_by", EntityKind::Proposal),
];

/// `[[target|alias]]` → (`target`, `Some(alias)`); plain values have no alias.
fn split_link(raw: &str) -> (&str, Option<&str>) {
    let raw = raw.trim();
    match raw.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        Some(inner) => match inner.split_once('|') {
            Some((t, a)) => (t.trim(), Some(a.trim()).filter(|a| !a.is_empty())),
            None => (inner.trim(), None),
        },
        None => (raw, None),
    }
}

/// Lowercased, whitespace-collapsed form for comparing display names.
fn name_key(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Warn about links that don't resolve: `depends_on`, `projects`,
/// `customers`, `sprint`, ... pointing at missing entities (or entities of
/// the wrong kind), non-canonical spellings like `task-1`, and
/// `[[ID|Alias]]` links whose alias names someone other than the entity.
/// Attendees are only checked when they look like an entity ID, since
/// free-text names and links to personal notes are common there.
fn validate_references(cfg: &ResolvedConfig, issues: &mut Vec<ValidationIssue>) -> McResult<()> {
    let mut records = Vec::new();
    for kind in EntityKind::ALL {
        if cfg.entity_available(&kind) {
            records.extend(data::collect_entities(kind, cfg)?);
        }
    }
    let known: HashMap<&str, (EntityKind, &str)> = records
        .iter()
        .map(|r| {
            let name = ["name", "title"]
                .into_iter()
                .find_map(|k| frontmatter::get_str(&r.frontmatter, k))
                .filter(|n| !n.trim().is_empty())
                .unwrap_or(&r.id);
            (r.id.as_str(), (r.kind, name))
        })
        .collect();
    // Look entities up by name to suggest the ID a broken link probably meant.
    let mut by_name: HashMap<(&str, String), &str> = HashMap::new();
    for r in &records {
        let (kind, name) = known[r.id.as_str()];
        by_name
            .entry((kind.label(), name_key(name)))
            .or_insert(r.id.as_str());
    }
    let named = |kind: EntityKind, text: &str| -> String {
        by_name
            .get(&(kind.label(), name_key(text)))
            .map(|id| format!(" (did you mean {id}?)"))
            .unwrap_or_default()
    };
    let prefixes = EntityKind::ALL
        .iter()
        .map(|k| regex::escape(k.prefix(cfg)))
        .collect::<Vec<_>>()
        .join("|");
    // An ID at the start of a link target (`CONT-003` in `CONT-003-jane-doe`).
    let id_re = Regex::new(&format!(r"^(?:{prefixes})-\d+(?:-|$)")).expect("escaped regex");
    let leading_id = |target: &str| -> Option<String> {
        id_re
            .find(target)
            .map(|m| m.as_str().trim_end_matches('-').to_string())
    };

    // A target that isn't a known ID: suggest the canonical form if the
    // loose spelling (`task-1`) resolves, otherwise it's broken.
    // A sprint may also be named by its title (`mc new` and the filters
    // accept it), so a title is only a spelling to update.
    let unresolved = |field: &str, target: &str, kind: EntityKind| -> (&'static str, String) {
        let canonical = match suggest::normalize_id(target, cfg, Some(kind)) {
            Ok((id, k)) if k == kind && known.contains_key(id.as_str()) => Some(id),
            _ if kind == EntityKind::Sprint => by_name
                .get(&(kind.label(), name_key(target)))
                .map(|id| id.to_string()),
            _ => None,
        };
        match canonical {
            Some(id) => (
                "reference-form",
                format!("{field}: '{target}' should be written as {id}"),
            ),
            _ => (
                "broken-reference",
                format!(
                    "{field}: '{target}' does not match any {}{}",
                    kind.label(),
                    named(kind, target)
                ),
            ),
        }
    };
    // `[[CONT-003-jane-doe|Jane Doe]]` whose ID belongs to someone else: the
    // slug (or, for attendees without one, the alias) names another entity.
    // Short aliases like `[[PROJ-001|Platform]]` are fine.
    let mismatch = |field: &str,
                    raw: &str,
                    target: &str,
                    (id, kind, name): (&str, EntityKind, &str),
                    alias: Option<&str>| {
        let slug = target[id.len()..].trim_start_matches('-');
        let names_other = if !slug.is_empty() {
            !util::slug_variants(name).iter().any(|s| s == slug)
        } else {
            field == "attendees" && alias.is_some_and(|a| name_key(a) != name_key(name))
        };
        names_other.then(|| {
            let hint = alias.map(|a| named(kind, a)).unwrap_or_default();
            (
                "alias-mismatch",
                format!("{field}: {raw} links to {id}, which is '{name}'{hint}"),
            )
        })
    };

    for r in &records {
        let mut found: Vec<(&str, String)> = Vec::new();
        for &(field, kind) in REFERENCE_FIELDS {
            if !cfg.entity_available(&kind) {
                continue;
            }
            for raw in frontmatter::get_string_list(&r.frontmatter, field) {
                let (target, alias) = split_link(&raw);
                if target.is_empty() {
                    continue;
                }
                let id = leading_id(target).unwrap_or_else(|| target.to_string());
                match known.get(id.as_str()) {
                    Some(&(k, name)) if k == kind => {
                        found.extend(mismatch(field, &raw, target, (&id, kind, name), alias))
                    }
                    Some(&(k, _)) => found.push((
                        "broken-reference",
                        format!("{field}: {id} is a {}, not a {}", k.label(), kind.label()),
                    )),
                    None => found.push(unresolved(field, target, kind)),
                }
            }
        }
        if cfg.entity_available(&EntityKind::Contact) {
            for raw in frontmatter::get_string_list(&r.frontmatter, "attendees") {
                let (target, alias) = split_link(&raw);
                let Some(id) = leading_id(target) else {
                    continue;
                };
                match known.get(id.as_str()) {
                    Some(&(kind, name)) => found.extend(mismatch(
                        "attendees",
                        &raw,
                        target,
                        (&id, kind, name),
                        alias,
                    )),
                    None => found.push(unresolved("attendees", target, EntityKind::Contact)),
                }
            }
        }
        issues.extend(found.into_iter().map(|(check, message)| ValidationIssue {
            path: r.source_path.display().to_string(),
            check: check.into(),
            severity: Severity::Warning,
            message,
        }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::init;
    use crate::config;
    use tempfile::TempDir;

    fn setup() -> (TempDir, ResolvedConfig) {
        let tmp = TempDir::new().unwrap();
        init::run(tmp.path(), false, false, Some("T"), false, true).unwrap();
        let cfg = config::load_config(tmp.path(), RepoMode::Standalone).unwrap();
        (tmp, cfg)
    }

    fn task(cfg: &ResolvedConfig, folder: &str, file: &str, fm: &str) -> PathBuf {
        let dir = cfg.tasks_dir.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(file);
        std::fs::write(&path, format!("---\n{fm}\n---\n\n# Body\n")).unwrap();
        path
    }

    fn checks<'a>(issues: &'a [ValidationIssue], check: &str) -> Vec<&'a ValidationIssue> {
        issues.iter().filter(|i| i.check == check).collect()
    }

    #[test]
    fn duplicate_ids_are_reported_for_every_file() {
        let (_tmp, cfg) = setup();
        let fm = "id: TASK-001\ntitle: A\nstatus: todo";
        let a = task(&cfg, "todo", "TASK-001-a.md", fm);
        let b = task(&cfg, "todo", "TASK-001-b.md", fm);
        task(
            &cfg,
            "todo",
            "TASK-002-c.md",
            "id: TASK-002\ntitle: C\nstatus: todo",
        );

        let issues = validate_programmatic(&cfg).unwrap();
        let dups = checks(&issues, "duplicate-id");
        assert_eq!(
            dups.len(),
            2,
            "{:?}",
            dups.iter().map(|i| &i.message).collect::<Vec<_>>()
        );
        assert!(dups.iter().all(|i| i.is_error()));
        let for_a = dups
            .iter()
            .find(|i| i.path == a.display().to_string())
            .unwrap();
        assert!(
            for_a.message.contains("tasks/todo/TASK-001-b.md"),
            "{}",
            for_a.message
        );
        assert!(dups.iter().any(|i| i.path == b.display().to_string()));
    }

    #[test]
    fn folder_check_follows_task_status_folder_for_custom_statuses() {
        let (tmp, _) = setup();
        let config_path = tmp.path().join("config/config.yml");
        let config = std::fs::read_to_string(&config_path).unwrap();
        std::fs::write(
            &config_path,
            config.replace(
                "    - review\n    - done\n",
                "    - review\n    - blocked\n    - done\n",
            ),
        )
        .unwrap();
        let cfg = config::load_config(tmp.path(), RepoMode::Standalone).unwrap();

        // Where `mc new` / `mc task move` put a blocked task is fine...
        let home = entity::task_status_folder("blocked");
        let other = if home == "todo" { "done" } else { "todo" };
        task(
            &cfg,
            home,
            "TASK-001-a.md",
            "id: TASK-001\ntitle: A\nstatus: blocked",
        );
        assert!(checks(&validate_programmatic(&cfg).unwrap(), "folder-status-sync").is_empty());

        // ...and the other folder is flagged, pointing at the same place.
        std::fs::remove_file(cfg.tasks_dir.join(home).join("TASK-001-a.md")).unwrap();
        task(
            &cfg,
            other,
            "TASK-001-a.md",
            "id: TASK-001\ntitle: A\nstatus: blocked",
        );
        let issues = validate_programmatic(&cfg).unwrap();
        let sync = checks(&issues, "folder-status-sync");
        assert_eq!(sync.len(), 1);
        assert!(
            sync[0].message.contains(&format!("belongs in '{home}/'")),
            "{}",
            sync[0].message
        );
    }

    #[test]
    fn yaml_errors_say_what_and_where() {
        let (_tmp, cfg) = setup();
        task(
            &cfg,
            "todo",
            "TASK-001-a.md",
            "id: TASK-001\ntitle: A\nstatus: [todo\npriority: 3",
        );
        let issues = validate_programmatic(&cfg).unwrap();
        let yaml = checks(&issues, "yaml-validity");
        assert_eq!(yaml.len(), 1);
        let msg = &yaml[0].message;
        assert!(msg.starts_with("Invalid YAML in frontmatter: "), "{msg}");
        // The `[` opens on line 4 of the file (line 3 of the frontmatter).
        assert!(msg.contains("sequence at line 4"), "{msg}");
    }

    #[test]
    fn sprint_titles_are_a_spelling_not_a_broken_link() {
        let (_tmp, cfg) = setup();
        let sprint = crate::commands::new::SprintInput {
            title: "2026-W05".into(),
            ..Default::default()
        };
        crate::commands::new::create_sprint(&cfg, &sprint).unwrap();
        task(
            &cfg,
            "todo",
            "TASK-001-a.md",
            "id: TASK-001\ntitle: A\nstatus: todo\nsprint: 2026-w05",
        );
        let issues = validate_programmatic(&cfg).unwrap();
        let all: Vec<&str> = issues.iter().map(|i| i.message.as_str()).collect();
        assert!(checks(&issues, "broken-reference").is_empty(), "{all:?}");
        let form = checks(&issues, "reference-form");
        assert_eq!(form.len(), 1, "{all:?}");
        assert_eq!(
            form[0].message,
            "sprint: '2026-w05' should be written as SPR-001"
        );
    }

    #[test]
    fn broken_references_are_warnings() {
        let (_tmp, cfg) = setup();
        task(
            &cfg,
            "todo",
            "TASK-001-a.md",
            "id: TASK-001\ntitle: A\nstatus: todo",
        );
        task(
            &cfg,
            "todo",
            "TASK-002-b.md",
            "id: TASK-002\ntitle: B\nstatus: todo\ndepends_on: ['[[task-1]]', '[[TASK-077]]']\ncustomers: [CUST-099]\nprojects: [TASK-001]\nsprint: '[[nope]]'",
        );
        let contacts = cfg.customers_dir.join("CUST-001-acme/contacts");
        std::fs::create_dir_all(&contacts).unwrap();
        std::fs::write(
            cfg.customers_dir.join("CUST-001-acme/CUST-001.md"),
            "---\nid: CUST-001\nname: Acme\nstatus: active\n---\n",
        )
        .unwrap();
        for (id, name) in [("CONT-001", "Alice Smith"), ("CONT-002", "Bob Jones")] {
            std::fs::write(
                contacts.join(format!("{id}-x.md")),
                format!("---\nid: {id}\nname: {name}\nstatus: active\n---\n"),
            )
            .unwrap();
        }
        std::fs::write(
            cfg.meetings_dir.join("2026-01-01-sync.md"),
            "---\nid: MTG-001\ntitle: Sync\nstatus: scheduled\nattendees:\n- '[[CONT-001-bob-jones|Bob Jones]]'\n- '[[CONT-002-bob-jones|Bob Jones]]'\n- '[[someone|Someone]]'\n- Free Text\n- '[[CONT-009]]'\n---\n",
        )
        .unwrap();

        let issues = validate_programmatic(&cfg).unwrap();
        assert!(
            issues
                .iter()
                .all(|i| i.check == "task-filename" || !i.is_error()),
            "{:?}",
            issues
                .iter()
                .filter(|i| i.is_error())
                .map(|i| &i.message)
                .collect::<Vec<_>>()
        );
        let msgs = |check: &str| -> Vec<String> {
            checks(&issues, check)
                .iter()
                .map(|i| i.message.clone())
                .collect()
        };
        assert_eq!(
            msgs("reference-form"),
            ["depends_on: 'task-1' should be written as TASK-001"]
        );
        let broken = msgs("broken-reference");
        assert!(
            broken.contains(&"depends_on: 'TASK-077' does not match any task".to_string()),
            "{broken:?}"
        );
        assert!(
            broken.contains(&"customers: 'CUST-099' does not match any customer".to_string()),
            "{broken:?}"
        );
        assert!(
            broken.contains(&"projects: TASK-001 is a task, not a project".to_string()),
            "{broken:?}"
        );
        assert!(
            broken.contains(&"sprint: 'nope' does not match any sprint".to_string()),
            "{broken:?}"
        );
        assert!(
            broken.contains(&"attendees: 'CONT-009' does not match any contact".to_string()),
            "{broken:?}"
        );
        assert_eq!(
            msgs("alias-mismatch"),
            ["attendees: [[CONT-001-bob-jones|Bob Jones]] links to CONT-001, which is 'Alice Smith' (did you mean CONT-002?)"]
        );
        assert!(issues.iter().all(|i| i.check != "duplicate-id"));
    }
}
