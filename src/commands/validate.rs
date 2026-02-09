use crate::config::{RepoMode, ResolvedConfig};
use crate::data;
use crate::entity::{self, EntityKind};
use crate::error::{McError, McResult};
use crate::frontmatter;
use colored::*;
use regex::Regex;
use serde::Serialize;
use std::path::Path;
use walkdir::WalkDir;

#[derive(Serialize)]
pub struct ValidationIssue {
    pub path: String,
    pub check: String,
    pub message: String,
}

pub fn run(cfg: &ResolvedConfig) -> McResult<()> {
    println!("{} Validating repo...\n", "⟳".blue());

    let issues = validate_programmatic(cfg)?;

    if issues.is_empty() {
        println!("{} All checks passed!", "✓".green().bold());
        Ok(())
    } else {
        println!("{} {} issue(s) found:\n", "✗".red().bold(), issues.len());
        for (i, issue) in issues.iter().enumerate() {
            println!(
                "  {}. [{}] {}\n     {}",
                (i + 1).to_string().red(),
                issue.check.yellow(),
                issue.path.dimmed(),
                issue.message
            );
        }
        Err(McError::ValidationFailed(issues.len()))
    }
}

/// Run validation and return structured issues without printing.
pub fn validate_programmatic(cfg: &ResolvedConfig) -> McResult<Vec<ValidationIssue>> {
    let mut issues: Vec<ValidationIssue> = Vec::new();

    if cfg.mode == RepoMode::Standalone {
        validate_entity_dirs(EntityKind::Customer, cfg, &mut issues)?;
        validate_entity_dirs(EntityKind::Project, cfg, &mut issues)?;
    }
    validate_meetings(cfg, &mut issues)?;
    validate_entity_dirs(EntityKind::Research, cfg, &mut issues)?;
    validate_entity_dirs(EntityKind::Sprint, cfg, &mut issues)?;
    validate_tasks(cfg, &mut issues)?;

    Ok(issues)
}

fn validate_entity_dirs(
    kind: EntityKind,
    cfg: &ResolvedConfig,
    issues: &mut Vec<ValidationIssue>,
) -> McResult<()> {
    let base = kind.base_dir(cfg);
    let prefix = kind.prefix(cfg);

    if !base.is_dir() {
        return Ok(());
    }

    // Check folder naming: PREFIX-NNN-slug
    let dir_re = Regex::new(&format!(
        r"^{}-\d{{3}}-[a-z0-9]+(-[a-z0-9]+)*$",
        regex::escape(prefix)
    ))
    .unwrap();

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
                message: format!(
                    "Directory name does not match expected pattern: {}-NNN-slug",
                    prefix
                ),
            });
        }

        // Check for _index.md or overview.md
        let index_file = match kind {
            EntityKind::Project => entry.path().join("overview.md"),
            _ => entry.path().join("_index.md"),
        };

        if !index_file.is_file() {
            issues.push(ValidationIssue {
                path: index_file.display().to_string(),
                check: "missing-index".into(),
                message: "Required index file not found".into(),
            });
            continue;
        }

        // Validate frontmatter
        validate_frontmatter_file(&index_file, kind, prefix, cfg, issues);
    }

    Ok(())
}

fn validate_meetings(cfg: &ResolvedConfig, issues: &mut Vec<ValidationIssue>) -> McResult<()> {
    let base = &cfg.meetings_dir;
    if !base.is_dir() {
        return Ok(());
    }

    let filename_re = Regex::new(r"^\d{4}-\d{2}-\d{2}-.+\.md$").unwrap();

    for entry in WalkDir::new(base)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_dir() || path.extension().is_none_or(|e| e != "md") {
            continue;
        }

        let filename = path.file_name().unwrap().to_string_lossy().to_string();

        // Check meeting filename pattern
        if !filename_re.is_match(&filename) {
            issues.push(ValidationIssue {
                path: filename.clone(),
                check: "meeting-filename".into(),
                message: "Meeting filename does not match YYYY-MM-DD-slug.md pattern".into(),
            });
        }

        validate_frontmatter_file(
            path,
            EntityKind::Meeting,
            &cfg.id_prefixes.meeting,
            cfg,
            issues,
        );
    }

    Ok(())
}

/// Validate all task files across all locations.
fn validate_tasks(cfg: &ResolvedConfig, issues: &mut Vec<ValidationIssue>) -> McResult<()> {
    let locations = entity::collect_all_task_dirs(cfg);
    let prefix = &cfg.id_prefixes.task;
    let filename_re = Regex::new(&format!(
        r"^{}-\d{{3}}-[a-z0-9]+(-[a-z0-9]+)*\.md$",
        regex::escape(prefix)
    ))
    .unwrap();

    let active_statuses = ["backlog", "todo", "in-progress", "review"];
    let finished_statuses = ["done", "cancelled"];

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
                            message: format!(
                                "Unexpected subfolder '{}' in tasks directory (expected only 'todo' and 'done')",
                                name
                            ),
                        });
                    }
                }
            }
        }

        for (subfolder, expected_statuses) in &[
            ("todo", active_statuses.as_slice()),
            ("done", finished_statuses.as_slice()),
        ] {
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

                    let filename = path.file_name().unwrap().to_string_lossy().to_string();

                    // Check filename pattern
                    if !filename_re.is_match(&filename) {
                        issues.push(ValidationIssue {
                            path: path.display().to_string(),
                            check: "task-filename".into(),
                            message: format!(
                                "Task filename does not match {}-NNN-slug.md pattern",
                                prefix
                            ),
                        });
                    }

                    // Validate frontmatter
                    validate_task_frontmatter_file(&path, prefix, cfg, expected_statuses, issues);
                }
            }
        }
    }

    Ok(())
}

fn validate_task_frontmatter_file(
    path: &Path,
    prefix: &str,
    cfg: &ResolvedConfig,
    expected_statuses: &[&str],
    issues: &mut Vec<ValidationIssue>,
) {
    let path_str = path.display().to_string();

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            issues.push(ValidationIssue {
                path: path_str,
                check: "read-error".into(),
                message: "Could not read file".into(),
            });
            return;
        }
    };

    let (fm_str, _body) = match frontmatter::split_frontmatter(&content) {
        Some(parts) => parts,
        None => {
            issues.push(ValidationIssue {
                path: path_str,
                check: "frontmatter-presence".into(),
                message: "No YAML frontmatter found".into(),
            });
            return;
        }
    };

    let fm = match frontmatter::parse_raw(&fm_str) {
        Ok(v) => v,
        Err(_) => {
            issues.push(ValidationIssue {
                path: path_str,
                check: "yaml-validity".into(),
                message: "Invalid YAML in frontmatter".into(),
            });
            return;
        }
    };

    // Required: id
    let id = match frontmatter::get_str(&fm, "id") {
        Some(id) => id.to_string(),
        None => {
            issues.push(ValidationIssue {
                path: path_str,
                check: "required-fields".into(),
                message: "Missing required 'id' field".into(),
            });
            return;
        }
    };

    if !id.starts_with(&format!("{}-", prefix)) {
        issues.push(ValidationIssue {
            path: path_str.clone(),
            check: "id-consistency".into(),
            message: format!(
                "ID '{}' does not start with expected prefix '{}-'",
                id, prefix
            ),
        });
    }

    // Required: title
    if frontmatter::get_str(&fm, "title").is_none() {
        issues.push(ValidationIssue {
            path: path_str.clone(),
            check: "required-fields".into(),
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
                message: format!(
                    "Invalid status '{}', expected one of: {}",
                    status,
                    valid_statuses.join(", ")
                ),
            });
        }

        // Folder↔status sync check
        if !expected_statuses.contains(&status) {
            let folder = if expected_statuses.contains(&"backlog") {
                "todo"
            } else {
                "done"
            };
            issues.push(ValidationIssue {
                path: path_str.clone(),
                check: "folder-status-sync".into(),
                message: format!(
                    "Task with status '{}' is in '{}/' folder but should be in '{}'",
                    status,
                    folder,
                    if folder == "todo" { "done/" } else { "todo/" }
                ),
            });
        }
    }

    // Priority validity (1-4)
    if let Some(priority) = data::get_number(&fm, "priority") {
        if !(1..=4).contains(&priority) {
            issues.push(ValidationIssue {
                path: path_str.clone(),
                check: "priority-range".into(),
                message: format!(
                    "Priority {} is out of range (expected 1-4: 1=critical, 2=high, 3=medium, 4=low)",
                    priority
                ),
            });
        }
    }
}

fn validate_frontmatter_file(
    path: &Path,
    kind: EntityKind,
    prefix: &str,
    cfg: &ResolvedConfig,
    issues: &mut Vec<ValidationIssue>,
) {
    let path_str = path.display().to_string();

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            issues.push(ValidationIssue {
                path: path_str,
                check: "read-error".into(),
                message: "Could not read file".into(),
            });
            return;
        }
    };

    // Check 2: frontmatter presence
    let (fm_str, _body) = match frontmatter::split_frontmatter(&content) {
        Some(parts) => parts,
        None => {
            issues.push(ValidationIssue {
                path: path_str,
                check: "frontmatter-presence".into(),
                message: "No YAML frontmatter found".into(),
            });
            return;
        }
    };

    // Check 3: YAML validity
    let fm = match frontmatter::parse_raw(&fm_str) {
        Ok(v) => v,
        Err(_) => {
            issues.push(ValidationIssue {
                path: path_str,
                check: "yaml-validity".into(),
                message: "Invalid YAML in frontmatter".into(),
            });
            return;
        }
    };

    // Check 4: required 'id' field
    let id = match frontmatter::get_str(&fm, "id") {
        Some(id) => id.to_string(),
        None => {
            issues.push(ValidationIssue {
                path: path_str,
                check: "required-fields".into(),
                message: "Missing required 'id' field".into(),
            });
            return;
        }
    };

    // Check 5: ID starts with correct prefix
    if !id.starts_with(&format!("{}-", prefix)) {
        issues.push(ValidationIssue {
            path: path_str.clone(),
            check: "id-consistency".into(),
            message: format!(
                "ID '{}' does not start with expected prefix '{}-'",
                id, prefix
            ),
        });
    }

    // Check 6: required name/title field
    let has_name = match kind {
        EntityKind::Customer | EntityKind::Project => frontmatter::get_str(&fm, "name").is_some(),
        EntityKind::Meeting | EntityKind::Research | EntityKind::Task | EntityKind::Sprint => {
            frontmatter::get_str(&fm, "title").is_some()
        }
    };
    if !has_name {
        let field = match kind {
            EntityKind::Customer | EntityKind::Project => "name",
            _ => "title",
        };
        issues.push(ValidationIssue {
            path: path_str.clone(),
            check: "required-fields".into(),
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
                message: format!(
                    "Invalid status '{}', expected one of: {}",
                    status,
                    valid_statuses.join(", ")
                ),
            });
        }
    }

    // Check 8: slug consistency (for directory-based entities)
    if kind != EntityKind::Meeting && kind != EntityKind::Task && kind != EntityKind::Sprint {
        if let Some(slug) = frontmatter::get_str(&fm, "slug") {
            // Check that the parent directory contains the slug
            if let Some(parent) = path.parent() {
                let dir_name = parent.file_name().unwrap_or_default().to_string_lossy();
                if !dir_name.contains(slug) {
                    issues.push(ValidationIssue {
                        path: path_str,
                        check: "slug-consistency".into(),
                        message: format!(
                            "Slug '{}' does not match directory name '{}'",
                            slug, dir_name
                        ),
                    });
                }
            }
        }
    }
}
