use crate::cli::TaskSubcommand;
use crate::config::ResolvedConfig;
use crate::data::{self, TaskFilter};
use crate::error::{McError, McResult};
use crate::frontmatter;
use crate::util;
use colored::*;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// Active statuses (file lives in todo/).
const ACTIVE_STATUSES: &[&str] = &["backlog", "todo", "in-progress", "review"];
/// Finished statuses (file lives in done/).
const FINISHED_STATUSES: &[&str] = &["done", "cancelled"];

pub fn run(subcmd: &TaskSubcommand, cfg: &ResolvedConfig) -> McResult<()> {
    match subcmd {
        TaskSubcommand::Board {
            project,
            customer,
            sprint,
        } => run_board(
            cfg,
            project.as_deref(),
            customer.as_deref(),
            sprint.as_deref(),
        ),
        TaskSubcommand::Move { id, status, sprint } => run_move(cfg, id, status, sprint.as_deref()),
        TaskSubcommand::Next { project, customer } => {
            run_next(cfg, project.as_deref(), customer.as_deref())
        }
    }
}

// ---------------------------------------------------------------------------
// ANSI-aware helpers for board rendering
// ---------------------------------------------------------------------------

/// Compute visible length of a string, ignoring ANSI escape sequences.
fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else {
            len += 1;
        }
    }
    len
}

/// Pad a (possibly ANSI-colored) string to `width` visible characters.
fn pad_to(s: &str, width: usize) -> String {
    let vis = visible_len(s);
    if vis >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - vis))
    }
}

/// Truncate a plain string to `max` visible characters, appending ".." if needed.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}..",
            s.chars().take(max.saturating_sub(2)).collect::<String>()
        )
    }
}

fn run_board(
    cfg: &ResolvedConfig,
    project: Option<&str>,
    customer: Option<&str>,
    sprint: Option<&str>,
) -> McResult<()> {
    let filter = TaskFilter {
        status: None,
        tag: None,
        project,
        customer,
        priority: None,
        sprint,
        owner: None,
    };

    let tasks = data::collect_tasks_filtered(cfg, &filter)?;

    // Group tasks by status
    let all_columns = ["backlog", "todo", "in-progress", "review", "done"];
    let mut grouped: HashMap<&str, Vec<&data::EntityRecord>> = HashMap::new();
    for col in &all_columns {
        grouped.insert(col, Vec::new());
    }

    for task in &tasks {
        let status = frontmatter::get_str_or(&task.frontmatter, "status", "backlog");
        if let Some(col) = grouped.get_mut(status) {
            col.push(task);
        }
        // cancelled tasks are not shown on the board by default
    }

    // Sort each column by priority
    for col in grouped.values_mut() {
        col.sort_by(|a, b| {
            let pa = data::get_number(&a.frontmatter, "priority").unwrap_or(3);
            let pb = data::get_number(&b.frontmatter, "priority").unwrap_or(3);
            pa.cmp(&pb).then(a.id.cmp(&b.id))
        });
    }

    // Skip empty columns
    let columns: Vec<&str> = all_columns
        .iter()
        .filter(|c| !grouped[**c].is_empty())
        .copied()
        .collect();

    // Print header
    let title = if let Some(p) = project {
        format!("{} -- Task Board", p)
    } else if let Some(c) = customer {
        format!("{} -- Task Board", c)
    } else {
        "Task Board".to_string()
    };

    if let Some(sp) = sprint {
        println!("\n  {} (sprint: {})\n", title.bold(), sp.cyan());
    } else {
        println!("\n  {}\n", title.bold());
    }

    if columns.is_empty() {
        println!("  {}", "(no tasks)".dimmed());
        println!();
        return Ok(());
    }

    // Compute column width — distribute available width across active columns
    let indent = 2;
    let gap = 2; // space between columns
    let available = 100usize; // target total width
    let num_cols = columns.len();
    let col_width = ((available - indent) / num_cols).clamp(18, 28);

    // Header row
    let mut header_line = String::new();
    for col in &columns {
        let count = grouped[col].len();
        let header = format!("{} ({})", col.to_uppercase(), count);
        header_line.push_str(&pad_to(&header.bold().to_string(), col_width + gap));
    }
    println!("  {}", header_line.trim_end());

    // Separator — ASCII dashes, ANSI-safe
    let total_width = num_cols * col_width + (num_cols - 1) * gap;
    println!("  {}", "-".repeat(total_width));

    // Find max rows
    let max_rows = columns.iter().map(|c| grouped[c].len()).max().unwrap_or(0);

    for row in 0..max_rows {
        // Line 1: Task ID (colored by priority)
        let mut line = String::new();
        for col in &columns {
            let tasks_in_col = &grouped[col];
            if row < tasks_in_col.len() {
                let task = tasks_in_col[row];
                let id = frontmatter::get_str_or(&task.frontmatter, "id", "");
                let priority = data::get_number(&task.frontmatter, "priority").unwrap_or(3);
                let colored_id = match priority {
                    1 => id.red().bold().to_string(),
                    2 => id.yellow().bold().to_string(),
                    3 => id.normal().to_string(),
                    4 => id.dimmed().to_string(),
                    _ => id.to_string(),
                };
                line.push_str(&pad_to(&colored_id, col_width + gap));
            } else {
                line.push_str(&" ".repeat(col_width + gap));
            }
        }
        println!("  {}", line.trim_end());

        // Line 2: Title (dimmed, truncated)
        let mut line = String::new();
        for col in &columns {
            let tasks_in_col = &grouped[col];
            if row < tasks_in_col.len() {
                let task = tasks_in_col[row];
                let title_str = frontmatter::get_str_or(&task.frontmatter, "title", "");
                let title_trunc = truncate(title_str, col_width - 2);
                let colored_title = title_trunc.dimmed().to_string();
                line.push_str(&pad_to(&colored_title, col_width + gap));
            } else {
                line.push_str(&" ".repeat(col_width + gap));
            }
        }
        println!("  {}", line.trim_end());

        // Line 3: Priority dot + label + optional @owner
        let mut line = String::new();
        for col in &columns {
            let tasks_in_col = &grouped[col];
            if row < tasks_in_col.len() {
                let task = tasks_in_col[row];
                let priority = data::get_number(&task.frontmatter, "priority").unwrap_or(3);
                let owner = frontmatter::get_str_or(&task.frontmatter, "owner", "");

                let pri_text = match priority {
                    1 => format!("{}", "● critical".red()),
                    2 => format!("{}", "● high".yellow()),
                    3 => format!("{}", "● medium".normal()),
                    4 => format!("{}", "● low".dimmed()),
                    _ => "●".to_string(),
                };

                let cell = if !owner.is_empty() {
                    let max_owner = col_width.saturating_sub(visible_len(&pri_text) + 3);
                    let owner_trunc = truncate(owner, max_owner);
                    format!("{}  {}", pri_text, format!("@{}", owner_trunc).dimmed())
                } else {
                    pri_text
                };

                line.push_str(&pad_to(&cell, col_width + gap));
            } else {
                line.push_str(&" ".repeat(col_width + gap));
            }
        }
        println!("  {}", line.trim_end());

        // Blank line between cards
        if row < max_rows - 1 {
            println!();
        }
    }

    println!();

    Ok(())
}

fn run_move(
    cfg: &ResolvedConfig,
    id: &str,
    new_status: &str,
    sprint: Option<&str>,
) -> McResult<()> {
    // Validate the new status
    let valid_statuses = &cfg.statuses.task;
    if !valid_statuses.iter().any(|s| s == new_status) {
        return Err(McError::Other(format!(
            "Invalid task status '{}'. Valid statuses: {}",
            new_status,
            valid_statuses.join(", ")
        )));
    }

    // Find the task
    let task = data::find_entity_by_id(id, cfg)?;
    let old_path = task.source_path.clone();
    let old_status = frontmatter::get_str_or(&task.frontmatter, "status", "backlog").to_string();

    // Read the full file content
    let content = std::fs::read_to_string(&old_path)?;
    let (fm_str, body) = frontmatter::split_frontmatter(&content)
        .ok_or_else(|| McError::Other("Task file has no frontmatter".into()))?;
    let mut fm = frontmatter::parse_raw(&fm_str)?;

    // Update frontmatter fields
    frontmatter::set_str(&mut fm, "status", new_status);
    frontmatter::set_str(&mut fm, "updated", &util::today_str());
    if let Some(sp) = sprint {
        frontmatter::set_str(&mut fm, "sprint", sp);
    }

    let new_doc = frontmatter::serialize_document(&fm, &body);

    // Determine if file needs to move between todo/ and done/
    let old_is_active = ACTIVE_STATUSES.contains(&old_status.as_str());
    let new_is_active = ACTIVE_STATUSES.contains(&new_status);

    if old_is_active == new_is_active {
        // Same folder -- just update the file in place
        std::fs::write(&old_path, &new_doc)?;
        println!(
            "{} {} status: {} -> {}",
            "+".green().bold(),
            id.cyan().bold(),
            crate::commands::list::format_status(&old_status),
            crate::commands::list::format_status(new_status),
        );
    } else {
        // Need to move between todo/ and done/
        let parent = old_path
            .parent()
            .and_then(|p| p.parent())
            .ok_or_else(|| McError::Other("Cannot determine task directory".into()))?;
        let filename = old_path
            .file_name()
            .ok_or_else(|| McError::Other("Cannot determine task filename".into()))?;

        let target_subfolder = if new_is_active { "todo" } else { "done" };
        let target_dir = parent.join(target_subfolder);
        std::fs::create_dir_all(&target_dir)?;

        let new_path = target_dir.join(filename);

        // Write new content to target, then remove old file
        std::fs::write(&new_path, &new_doc)?;
        std::fs::remove_file(&old_path)?;

        let direction = if new_is_active {
            "done/ -> todo/"
        } else {
            "todo/ -> done/"
        };

        println!(
            "{} {} status: {} -> {} ({})",
            "+".green().bold(),
            id.cyan().bold(),
            crate::commands::list::format_status(&old_status),
            crate::commands::list::format_status(new_status),
            direction.dimmed(),
        );
    }

    if let Some(sp) = sprint {
        println!("  sprint: {}", sp.cyan());
    }

    Ok(())
}

fn run_next(cfg: &ResolvedConfig, project: Option<&str>, customer: Option<&str>) -> McResult<()> {
    // Collect all tasks, filtered by project/customer if specified
    let filter = TaskFilter {
        status: None,
        tag: None,
        project,
        customer,
        priority: None,
        sprint: None,
        owner: None,
    };

    let all_tasks = data::collect_tasks_filtered(cfg, &filter)?;

    // Build a set of done task IDs for dependency checking
    let done_ids: std::collections::HashSet<String> = all_tasks
        .iter()
        .filter(|t| {
            let status = frontmatter::get_str_or(&t.frontmatter, "status", "");
            FINISHED_STATUSES.contains(&status)
        })
        .map(|t| t.id.clone())
        .collect();

    // Filter to actionable tasks: status=todo (preferred) or backlog, not blocked
    let mut candidates: Vec<&data::EntityRecord> = all_tasks
        .iter()
        .filter(|t| {
            let status = frontmatter::get_str_or(&t.frontmatter, "status", "");
            status == "todo" || status == "backlog"
        })
        .filter(|t| {
            // Check dependencies -- all depends_on must be done
            let deps = frontmatter::get_string_list(&t.frontmatter, "depends_on");
            deps.iter().all(|dep| done_ids.contains(dep))
        })
        .collect();

    if candidates.is_empty() {
        let scope = if let Some(p) = project {
            format!(" for {}", p)
        } else if let Some(c) = customer {
            format!(" for {}", c)
        } else {
            String::new()
        };
        println!("{} No actionable tasks found{}.", "i".blue(), scope);
        return Ok(());
    }

    // Sort: "todo" before "backlog", then by priority (1=critical first), then by ID
    candidates.sort_by(|a, b| {
        let sa = frontmatter::get_str_or(&a.frontmatter, "status", "backlog");
        let sb = frontmatter::get_str_or(&b.frontmatter, "status", "backlog");
        let status_order = |s: &str| -> u8 {
            match s {
                "todo" => 0,
                "backlog" => 1,
                _ => 2,
            }
        };
        status_order(sa)
            .cmp(&status_order(sb))
            .then_with(|| {
                let pa = data::get_number(&a.frontmatter, "priority").unwrap_or(3);
                let pb = data::get_number(&b.frontmatter, "priority").unwrap_or(3);
                pa.cmp(&pb)
            })
            .then(a.id.cmp(&b.id))
    });

    let next = candidates[0];
    let id = frontmatter::get_str_or(&next.frontmatter, "id", "");
    let title = frontmatter::get_str_or(&next.frontmatter, "title", "");
    let status = frontmatter::get_str_or(&next.frontmatter, "status", "");
    let priority = data::get_number(&next.frontmatter, "priority").unwrap_or(3);
    let owner = frontmatter::get_str_or(&next.frontmatter, "owner", "");
    let deps = frontmatter::get_string_list(&next.frontmatter, "depends_on");

    let pri_label = match priority {
        1 => "CRITICAL".red().bold().to_string(),
        2 => "HIGH".yellow().bold().to_string(),
        3 => "MEDIUM".normal().to_string(),
        4 => "LOW".dimmed().to_string(),
        _ => format!("{}", priority),
    };

    println!();
    println!(
        "  {} {} [{}] {}",
        "->".green().bold(),
        id.cyan().bold(),
        pri_label,
        title.bold()
    );
    println!(
        "    {} {}  {} {}",
        "status:".dimmed(),
        crate::commands::list::format_status(status),
        "owner:".dimmed(),
        if owner.is_empty() {
            "(unassigned)".dimmed().to_string()
        } else {
            owner.to_string()
        },
    );
    if !deps.is_empty() {
        let dep_display: Vec<String> = deps
            .iter()
            .map(|d| {
                if done_ids.contains(d) {
                    format!("{} +", d).green().to_string()
                } else {
                    d.yellow().to_string()
                }
            })
            .collect();
        println!("    {} {}", "depends on:".dimmed(), dep_display.join(", "));
    }

    // Show how many other candidates there are
    if candidates.len() > 1 {
        println!(
            "\n    {} {} other actionable task(s)",
            "+".dimmed(),
            (candidates.len() - 1).to_string().dimmed()
        );
    }
    println!();

    Ok(())
}

// ---------------------------------------------------------------------------
// Programmatic move function (no prompts, no printing, returns JSON)
// ---------------------------------------------------------------------------

pub fn move_task_programmatic(
    cfg: &ResolvedConfig,
    id: &str,
    new_status: &str,
    sprint: Option<&str>,
) -> McResult<JsonValue> {
    // Validate the new status
    let valid_statuses = &cfg.statuses.task;
    if !valid_statuses.iter().any(|s| s == new_status) {
        return Err(McError::Other(format!(
            "Invalid task status '{}'. Valid statuses: {}",
            new_status,
            valid_statuses.join(", ")
        )));
    }

    // Find the task
    let task = data::find_entity_by_id(id, cfg)?;
    let old_path = task.source_path.clone();
    let old_status = frontmatter::get_str_or(&task.frontmatter, "status", "backlog").to_string();

    // Read the full file content
    let content = std::fs::read_to_string(&old_path)?;
    let (fm_str, body) = frontmatter::split_frontmatter(&content)
        .ok_or_else(|| McError::Other("Task file has no frontmatter".into()))?;
    let mut fm = frontmatter::parse_raw(&fm_str)?;

    // Update frontmatter fields
    frontmatter::set_str(&mut fm, "status", new_status);
    frontmatter::set_str(&mut fm, "updated", &util::today_str());
    if let Some(sp) = sprint {
        frontmatter::set_str(&mut fm, "sprint", sp);
    }

    let new_doc = frontmatter::serialize_document(&fm, &body);

    // Determine if file needs to move between todo/ and done/
    let old_is_active = ACTIVE_STATUSES.contains(&old_status.as_str());
    let new_is_active = ACTIVE_STATUSES.contains(&new_status);

    let final_path = if old_is_active == new_is_active {
        // Same folder -- just update the file in place
        std::fs::write(&old_path, &new_doc)?;
        old_path.clone()
    } else {
        // Need to move between todo/ and done/
        let parent = old_path
            .parent()
            .and_then(|p| p.parent())
            .ok_or_else(|| McError::Other("Cannot determine task directory".into()))?;
        let filename = old_path
            .file_name()
            .ok_or_else(|| McError::Other("Cannot determine task filename".into()))?;

        let target_subfolder = if new_is_active { "todo" } else { "done" };
        let target_dir = parent.join(target_subfolder);
        std::fs::create_dir_all(&target_dir)?;

        let new_path = target_dir.join(filename);
        std::fs::write(&new_path, &new_doc)?;
        std::fs::remove_file(&old_path)?;
        new_path
    };

    Ok(serde_json::json!({
        "id": id,
        "old_status": old_status,
        "new_status": new_status,
        "path": final_path.display().to_string(),
    }))
}
