use crate::cli::suggest;
use crate::cli::ui;
use crate::cli::TaskSubcommand;
use crate::commands::index;
use crate::config::ResolvedConfig;
use crate::data::{self, EntityRecord, TaskFilter};
use crate::entity::{self, EntityKind};
use crate::error::{McError, McResult};
use crate::frontmatter;
use crate::util;
use colored::*;
use serde_json::Value as JsonValue;
use std::collections::HashSet;

/// Finished statuses (end of the task lifecycle).
const FINISHED_STATUSES: &[&str] = &["done", "cancelled"];

pub fn run(subcmd: &TaskSubcommand, cfg: &ResolvedConfig) -> McResult<()> {
    match subcmd {
        TaskSubcommand::Board {
            project,
            customer,
            sprint,
            owner,
            limit,
            all,
        } => {
            let filter = TaskFilter {
                status: None,
                tag: None,
                project: project.as_deref(),
                customer: customer.as_deref(),
                priority: None,
                sprint: sprint.as_deref(),
                owner: owner.as_deref(),
            };
            run_board(cfg, &filter, *limit, *all)
        }
        TaskSubcommand::Move { id, status, sprint } => run_move(cfg, id, status, sprint.as_deref()),
        TaskSubcommand::Next {
            project,
            customer,
            owner,
            limit,
        } => run_next(
            cfg,
            project.as_deref(),
            customer.as_deref(),
            owner.as_deref(),
            *limit,
        ),
    }
}

fn s<'a>(e: &'a EntityRecord, key: &str) -> &'a str {
    frontmatter::get_str_or(&e.frontmatter, key, "")
}

fn priority_of(e: &EntityRecord) -> u32 {
    data::get_number(&e.frontmatter, "priority").unwrap_or(3)
}

fn describe_filter(filter: &TaskFilter) -> Vec<String> {
    [
        ("project", filter.project),
        ("customer", filter.customer),
        ("sprint", filter.sprint),
        ("owner", filter.owner),
    ]
    .iter()
    .filter_map(|(k, v)| v.map(|v| format!("{k}={v}")))
    .collect()
}

// ---------------------------------------------------------------------------
// Board
// ---------------------------------------------------------------------------

fn run_board(cfg: &ResolvedConfig, filter: &TaskFilter, limit: usize, all: bool) -> McResult<()> {
    let tasks = data::collect_tasks_filtered(cfg, filter)?;

    // Columns follow the configured task statuses; cancelled is opt-in.
    let statuses: Vec<&str> = cfg
        .statuses
        .task
        .iter()
        .map(String::as_str)
        .filter(|st| all || *st != "cancelled")
        .collect();
    let mut columns: Vec<(&str, Vec<&EntityRecord>)> =
        statuses.iter().map(|st| (*st, Vec::new())).collect();
    for task in &tasks {
        let st = frontmatter::get_str_or(&task.frontmatter, "status", "backlog");
        if let Some((_, col)) = columns.iter_mut().find(|(c, _)| *c == st) {
            col.push(task);
        }
    }
    for (st, col) in columns.iter_mut() {
        if FINISHED_STATUSES.contains(st) {
            // Most recently touched first.
            col.sort_by(|a, b| s(b, "updated").cmp(s(a, "updated")).then(a.id.cmp(&b.id)));
        } else {
            col.sort_by(|a, b| priority_of(a).cmp(&priority_of(b)).then(a.id.cmp(&b.id)));
        }
    }

    if ui::get().json {
        let cols: Vec<JsonValue> = columns
            .iter()
            .map(|(st, col)| {
                serde_json::json!({
                    "status": st,
                    "count": col.len(),
                    "tasks": col.iter().map(|t| index::entity_json(t, cfg)).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "columns": cols }))?
        );
        return Ok(());
    }

    let g = ui::glyphs();
    let scope = describe_filter(filter);
    let mut title = format!("{} {}", g.brand.cyan(), ui::section("Task board"));
    if !scope.is_empty() {
        title = format!("{title}  {}", scope.join(" ").dimmed());
    }
    println!("\n  {title}\n");

    columns.retain(|(_, col)| !col.is_empty());
    if columns.is_empty() {
        ui::info("No tasks on the board.");
        ui::hint(format!(
            "create one: {}",
            ui::cmd("mc new task \"...\" --priority 2")
        ));
        return Ok(());
    }

    let cap = |st: &str| -> usize {
        if all {
            usize::MAX
        } else if FINISHED_STATUSES.contains(&st) {
            limit.min(5)
        } else {
            limit
        }
    };

    const GAP: usize = 3;
    let n = columns.len();
    let col_width = ui::get()
        .width
        .map(|w| (w.saturating_sub(2 + GAP * (n - 1)) / n).min(34));

    match col_width {
        Some(w) if w >= 18 => render_columns(&columns, w, GAP, &cap),
        _ => render_stacked(&columns, &cap),
    }

    let cancelled = tasks
        .iter()
        .filter(|t| s(t, "status") == "cancelled")
        .count();
    if !all && cancelled > 0 {
        ui::hint(format!(
            "{} hidden {} {}",
            ui::count(cancelled, "cancelled task", "cancelled tasks"),
            g.sep,
            ui::cmd("--all")
        ));
    }
    println!();
    Ok(())
}

/// Due date as `Oct 30` (`Oct 30 2027` outside the current year), colored by urgency.
fn short_due(t: &EntityRecord, today: &str) -> String {
    let due = s(t, "due_date");
    let Ok(date) = chrono::NaiveDate::parse_from_str(due, "%Y-%m-%d") else {
        return due.to_string();
    };
    let label = if today.get(..4) == due.get(..4) {
        date.format("%b %d").to_string()
    } else {
        date.format("%b %d %Y").to_string()
    };
    let colored = crate::commands::list::due_cell(t, today);
    colored.replace(due, &label)
}

/// Card footer: due date first (it matters most), then owner.
fn card_meta(t: &EntityRecord) -> String {
    let mut parts = Vec::new();
    if !s(t, "due_date").is_empty() {
        parts.push(format!("due {}", short_due(t, &util::today_str())));
    }
    let owner = s(t, "owner");
    if !owner.is_empty() {
        parts.push(format!("@{owner}").dimmed().to_string());
    }
    parts.join(&format!(" {} ", ui::glyphs().sep.dimmed()))
}

fn render_columns(
    columns: &[(&str, Vec<&EntityRecord>)],
    w: usize,
    gap: usize,
    cap: &dyn Fn(&str) -> usize,
) {
    let g = ui::glyphs();
    let spacer = " ".repeat(gap);
    let join = |cells: Vec<String>| -> String {
        let line: Vec<String> = cells.iter().map(|c| ui::pad(c, w)).collect();
        format!("  {}", line.join(&spacer)).trim_end().to_string()
    };

    println!(
        "{}",
        join(
            columns
                .iter()
                .map(|(st, col)| {
                    let tone = ui::status_tone(st);
                    format!(
                        "{} {}",
                        ui::tint(&st.to_uppercase(), tone).bold(),
                        col.len().to_string().dimmed()
                    )
                })
                .collect()
        )
    );
    println!(
        "{}",
        join(
            columns
                .iter()
                .map(|(st, _)| ui::tint(&g.heavy.repeat(w), ui::status_tone(st)).to_string())
                .collect()
        )
    );

    let rows = columns
        .iter()
        .map(|(st, col)| col.len().min(cap(st)))
        .max()
        .unwrap_or(0);
    for row in 0..rows {
        let cell = |line: usize| -> Vec<String> {
            columns
                .iter()
                .map(|(st, col)| {
                    if row >= col.len().min(cap(st)) {
                        return String::new();
                    }
                    let t = col[row];
                    let text = match line {
                        0 => format!("{} {}", ui::priority(priority_of(t)), t.id.cyan()),
                        1 => s(t, "title").to_string(),
                        _ => card_meta(t),
                    };
                    ui::truncate(&text, w)
                })
                .collect()
        };
        for line in 0..3 {
            println!("{}", join(cell(line)));
        }
        println!();
    }

    let more: Vec<String> = columns
        .iter()
        .map(|(st, col)| {
            let hidden = col.len().saturating_sub(cap(st));
            if hidden > 0 {
                format!("+{hidden} more").dimmed().to_string()
            } else {
                String::new()
            }
        })
        .collect();
    if more.iter().any(|m| !m.is_empty()) {
        println!("{}", join(more));
    }
}

fn render_stacked(columns: &[(&str, Vec<&EntityRecord>)], cap: &dyn Fn(&str) -> usize) {
    let width = ui::get().width;
    for (st, col) in columns {
        let tone = ui::status_tone(st);
        println!(
            "  {} {}",
            ui::tint(&st.to_uppercase(), tone).bold(),
            col.len().to_string().dimmed()
        );
        for t in col.iter().take(cap(st)) {
            let meta = card_meta(t);
            let mut line = format!(
                "    {} {}  {}",
                ui::priority(priority_of(t)),
                t.id.cyan(),
                s(t, "title")
            );
            if !meta.is_empty() {
                line = format!("{line}  {meta}");
            }
            match width {
                Some(w) => println!("{}", ui::truncate(&line, w)),
                None => println!("{line}"),
            }
        }
        let hidden = col.len().saturating_sub(cap(st));
        if hidden > 0 {
            println!("    {}", format!("+{hidden} more").dimmed());
        }
        println!();
    }
}

// ---------------------------------------------------------------------------
// Move
// ---------------------------------------------------------------------------

fn run_move(
    cfg: &ResolvedConfig,
    id: &str,
    new_status: &str,
    sprint: Option<&str>,
) -> McResult<()> {
    let new_status = suggest::resolve_status(new_status, &cfg.statuses.task, EntityKind::Task)?;
    let task = suggest::find_entity(id, cfg, Some(EntityKind::Task))?;
    if task.kind != EntityKind::Task {
        return Err(McError::usage(
            format!("{} is a {}, not a task", task.id, task.kind.label()),
            Some("task move only works on task IDs (TASK-001)".into()),
        ));
    }
    let old_status = frontmatter::get_str_or(&task.frontmatter, "status", "backlog").to_string();
    let title = s(&task, "title").to_string();

    if old_status == new_status && sprint.is_none() {
        if ui::get().json {
            let out = serde_json::json!({
                "id": task.id,
                "old_status": old_status,
                "new_status": new_status,
                "path": task.source_path.display().to_string(),
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            ui::info(format!(
                "{} is already {} {}",
                task.id.cyan().bold(),
                ui::status(&new_status),
                "(nothing changed)".dimmed()
            ));
        }
        return Ok(());
    }

    let result = move_task_programmatic(cfg, &task.id, &new_status, sprint)?;

    if ui::get().json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let g = ui::glyphs();
    ui::success(format!("{} {}", task.id.cyan().bold(), title.bold()));
    let mut detail = format!(
        "{} {} {}",
        ui::status(&old_status),
        g.arrow.dimmed(),
        ui::status(&new_status)
    );
    let folder = entity::task_status_folder(&new_status);
    if entity::task_status_folder(&old_status) != folder {
        detail = format!(
            "{detail}  {}",
            format!("{} moved to {folder}/", g.sep).dimmed()
        );
    }
    if let Some(sp) = sprint {
        detail = format!("{detail}  {} sprint {}", g.sep.dimmed(), sp.cyan());
    }
    println!("  {detail}");

    if FINISHED_STATUSES.contains(&new_status.as_str()) && ui::get().interactive {
        if let Ok((queue, _)) = actionable(cfg, &no_filter()) {
            if let Some(next) = queue.first() {
                ui::hint(format!(
                    "next up: {} {}",
                    next.id.cyan(),
                    ui::truncate(s(next, "title"), 50)
                ));
            }
        }
    } else if new_status == "in-progress" {
        ui::hint(format!(
            "when finished: {}",
            ui::cmd(&format!("mc task move {} done", task.id))
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Next
// ---------------------------------------------------------------------------

fn no_filter<'a>() -> TaskFilter<'a> {
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

/// Actionable tasks (todo/backlog with all dependencies finished), best first,
/// plus the number of open tasks that are blocked by dependencies.
pub(crate) fn actionable(
    cfg: &ResolvedConfig,
    filter: &TaskFilter,
) -> McResult<(Vec<EntityRecord>, usize)> {
    let all = data::collect_tasks(cfg)?;
    let (candidates, blocked) = actionable_in(&all, filter);
    Ok((candidates.into_iter().cloned().collect(), blocked))
}

/// [`actionable`] over already-loaded tasks. `all` must be every task in the
/// repo: dependencies may live outside the filtered scope.
pub(crate) fn actionable_in<'a>(
    all: &'a [EntityRecord],
    filter: &TaskFilter,
) -> (Vec<&'a EntityRecord>, usize) {
    let done_ids: HashSet<&str> = all
        .iter()
        .filter(|t| FINISHED_STATUSES.contains(&s(t, "status")))
        .map(|t| t.id.as_str())
        .collect();

    let mut blocked = 0;
    let mut candidates: Vec<&EntityRecord> = all
        .iter()
        .filter(|t| filter.matches(&t.frontmatter))
        .filter(|t| matches!(s(t, "status"), "todo" | "backlog"))
        .filter(|t| {
            let ok = frontmatter::get_link_list(&t.frontmatter, "depends_on")
                .iter()
                .all(|dep| done_ids.contains(dep.as_str()));
            if !ok {
                blocked += 1;
            }
            ok
        })
        .collect();

    // todo before backlog, then priority (1 = critical first), due date, then ID.
    candidates.sort_by(|a, b| {
        let rank = |t: &EntityRecord| u8::from(s(t, "status") != "todo");
        let due = |t: &EntityRecord| {
            let d = s(t, "due_date");
            (d.is_empty(), d.to_string())
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| priority_of(a).cmp(&priority_of(b)))
            .then_with(|| due(a).cmp(&due(b)))
            .then(a.id.cmp(&b.id))
    });
    (candidates, blocked)
}

fn run_next(
    cfg: &ResolvedConfig,
    project: Option<&str>,
    customer: Option<&str>,
    owner: Option<&str>,
    limit: usize,
) -> McResult<()> {
    let filter = TaskFilter {
        status: None,
        tag: None,
        project,
        customer,
        priority: None,
        sprint: None,
        owner,
    };
    let (candidates, blocked) = actionable(cfg, &filter)?;
    let limit = limit.max(1);

    if ui::get().json {
        let arr: Vec<JsonValue> = candidates
            .iter()
            .take(limit)
            .map(|t| index::entity_json(t, cfg))
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    let g = ui::glyphs();
    let scope = describe_filter(&filter);
    if candidates.is_empty() {
        let scope = if scope.is_empty() {
            String::new()
        } else {
            format!(" for {}", scope.join(" "))
        };
        ui::info(format!("Nothing actionable{scope}."));
        if blocked > 0 {
            ui::hint(format!(
                "{} waiting on unfinished dependencies",
                ui::count(blocked, "task is", "tasks are")
            ));
        }
        return Ok(());
    }

    let next = &candidates[0];
    let pri = priority_of(next);
    println!();
    println!(
        "  {} {}  {}  {} {}",
        g.arrow.green().bold(),
        ui::section("Next up").green(),
        next.id.cyan().bold(),
        ui::priority(pri),
        ui::priority_label(pri).dimmed()
    );
    println!("    {}", s(next, "title").bold());

    let mut meta = vec![ui::status(s(next, "status"))];
    let owner = s(next, "owner");
    meta.push(if owner.is_empty() {
        "unassigned".dimmed().to_string()
    } else {
        format!("@{owner}")
    });
    let projects = frontmatter::get_link_list(&next.frontmatter, "projects");
    if let Some(p) = projects.first() {
        meta.push(p.to_string());
    }
    let today = crate::util::today_str();
    let due = crate::commands::list::due_cell(next, &today);
    if !due.is_empty() {
        meta.push(format!("due {due}"));
    }
    println!("    {}", meta.join(&format!(" {} ", g.sep.dimmed())));

    let deps = frontmatter::get_link_list(&next.frontmatter, "depends_on");
    if !deps.is_empty() {
        let shown: Vec<String> = deps
            .iter()
            .map(|d| format!("{} {}", d, g.ok.green()))
            .collect();
        println!("    {} {}", "depends on".dimmed(), shown.join(", "));
    }

    if limit > 1 && candidates.len() > 1 {
        println!();
        println!("  {}", ui::section("Then").dimmed());
        let width = ui::get().width;
        for t in candidates.iter().skip(1).take(limit - 1) {
            let line = format!(
                "    {} {}  {}",
                ui::priority(priority_of(t)),
                t.id.cyan(),
                s(t, "title")
            );
            match width {
                Some(w) => println!("{}", ui::truncate(&line, w)),
                None => println!("{line}"),
            }
        }
    }

    println!();
    let rest = candidates.len().saturating_sub(limit);
    let mut tail = Vec::new();
    if rest > 0 {
        tail.push(format!("{rest} more actionable"));
    }
    if blocked > 0 {
        tail.push(format!("{blocked} blocked"));
    }
    if !tail.is_empty() {
        ui::hint(tail.join(&format!(" {} ", g.sep)));
    }
    ui::hint(format!(
        "start it: {}",
        ui::cmd(&format!("mc task move {} in-progress", next.id))
    ));
    println!();

    Ok(())
}

// ---------------------------------------------------------------------------
// Programmatic move function (no prompts, no printing, returns JSON)
//
// Note: The rename from todo/ → done/ (or vice versa) is not atomic across
// the status-update and file-move steps. This is fine for a single-user CLI;
// concurrent moves of the same task are not a realistic scenario.
// ---------------------------------------------------------------------------

pub fn move_task_programmatic(
    cfg: &ResolvedConfig,
    id: &str,
    new_status: &str,
    sprint: Option<&str>,
) -> McResult<JsonValue> {
    // Validate the new status (a usage error: 400 over HTTP, invalid_params over MCP).
    let valid_statuses = &cfg.statuses.task;
    if !valid_statuses.iter().any(|s| s == new_status) {
        let suggestion =
            suggest::did_you_mean(new_status, valid_statuses.iter().map(String::as_str))
                .map(|s| format!(" Did you mean '{s}'?"))
                .unwrap_or_default();
        return Err(McError::usage(
            format!(
                "Invalid task status '{}'.{} Valid statuses: {}",
                new_status,
                suggestion,
                valid_statuses.join(", ")
            ),
            None,
        ));
    }

    // Find the task
    let task = data::find_entity_by_id(id, cfg)?;
    let old_path = task.source_path.clone();
    let old_status = frontmatter::get_str_or(&task.frontmatter, "status", "backlog").to_string();

    // Read the full file content
    let content = std::fs::read_to_string(&old_path)?;
    let (fm_str, body) = frontmatter::split_frontmatter(&content)
        .ok_or_else(|| McError::Other("Task file has no frontmatter".into()))?;
    let mut fm = frontmatter::parse_raw(&fm_str, &old_path)?;

    // Update frontmatter fields
    frontmatter::set_str(&mut fm, "status", new_status);
    frontmatter::set_str(&mut fm, "updated", &util::today_str());
    if let Some(sp) = sprint {
        frontmatter::set_str(&mut fm, "sprint", &frontmatter::wrap_wikilink(sp));
    }

    let new_doc = frontmatter::serialize_document(&fm, &body);

    // Tasks live in `<tasks>/todo/` or `<tasks>/done/` depending on status.
    let target_subfolder = entity::task_status_folder(new_status);
    let final_path = if entity::task_status_folder(&old_status) == target_subfolder {
        // Same folder -- just update the file in place
        util::atomic_write(&old_path, new_doc.as_bytes())?;
        old_path.clone()
    } else {
        let parent = old_path
            .parent()
            .and_then(|p| p.parent())
            .ok_or_else(|| McError::Other("Cannot determine task directory".into()))?;
        let filename = old_path
            .file_name()
            .ok_or_else(|| McError::Other("Cannot determine task filename".into()))?;

        let target_dir = parent.join(target_subfolder);
        std::fs::create_dir_all(&target_dir)?;

        let new_path = target_dir.join(filename);
        util::atomic_write(&new_path, new_doc.as_bytes())?;
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
    fn test_task_move_status_change() {
        let (_tmp, cfg) = setup_repo();

        // Create a task with status=todo
        new::create_task_programmatic(
            &cfg,
            "Test task",
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

        // Move to in-progress (stays in todo/)
        let result = move_task_programmatic(&cfg, "TASK-001", "in-progress", None).unwrap();
        assert_eq!(result["old_status"], "todo");
        assert_eq!(result["new_status"], "in-progress");

        // Verify frontmatter was updated
        let path_str = result["path"].as_str().unwrap();
        let content = std::fs::read_to_string(path_str).unwrap();
        let (fm_str, _) = frontmatter::split_frontmatter(&content).unwrap();
        let fm = frontmatter::parse_raw(&fm_str, std::path::Path::new(path_str)).unwrap();
        assert_eq!(frontmatter::get_str(&fm, "status").unwrap(), "in-progress");
    }

    #[test]
    fn test_task_move_todo_to_done_folder() {
        let (_tmp, cfg) = setup_repo();

        // Create a task (defaults to todo/)
        new::create_task_programmatic(
            &cfg,
            "Finish feature",
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

        let todo_path = cfg
            .tasks_dir
            .join("todo")
            .join("TASK-001-finish-feature.md");
        assert!(todo_path.is_file());

        // Move to done
        let result = move_task_programmatic(&cfg, "TASK-001", "done", None).unwrap();
        assert_eq!(result["new_status"], "done");

        // File should now be in done/, not todo/
        assert!(!todo_path.is_file());
        let done_path = cfg
            .tasks_dir
            .join("done")
            .join("TASK-001-finish-feature.md");
        assert!(done_path.is_file());

        // Verify status in frontmatter
        let content = std::fs::read_to_string(&done_path).unwrap();
        let (fm_str, _) = frontmatter::split_frontmatter(&content).unwrap();
        let fm = frontmatter::parse_raw(&fm_str, &done_path).unwrap();
        assert_eq!(frontmatter::get_str(&fm, "status").unwrap(), "done");
    }

    #[test]
    fn test_task_move_round_trip_keeps_file_unchanged() {
        let (_tmp, cfg) = setup_repo();
        let created = new::create_task_programmatic(
            &cfg,
            "Stable file",
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
        let path = std::path::PathBuf::from(created["path"].as_str().unwrap());
        let before = std::fs::read_to_string(&path).unwrap();

        move_task_programmatic(&cfg, "TASK-001", "in-progress", None).unwrap();
        move_task_programmatic(&cfg, "TASK-001", "done", None).unwrap();
        let back = move_task_programmatic(&cfg, "TASK-001", "todo", None).unwrap();
        assert_eq!(back["path"].as_str().unwrap(), path.to_str().unwrap());

        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after.lines().count(), before.lines().count(), "{after}");
        assert_eq!(after, before);
    }

    #[test]
    fn test_actionable_in_checks_dependencies_outside_the_filter() {
        let (_tmp, cfg) = setup_repo();
        let mk = |title: &str, status: &str, owner: Option<&str>, deps: Option<&str>| {
            new::create_task_programmatic(
                &cfg,
                title,
                None,
                None,
                owner,
                Some(status),
                None,
                None,
                None,
                deps,
                None,
            )
            .unwrap();
        };
        mk("Done dep", "done", None, None); // TASK-001
        mk("Open dep", "todo", None, None); // TASK-002
        mk("Ready", "todo", Some("ann"), Some("TASK-001")); // TASK-003
        mk("Blocked", "todo", Some("ann"), Some("TASK-002")); // TASK-004

        let all = data::collect_tasks(&cfg).unwrap();
        let filter = TaskFilter {
            owner: Some("ann"),
            ..TaskFilter::all()
        };
        let (queue, blocked) = actionable_in(&all, &filter);
        let ids: Vec<&str> = queue.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["TASK-003"]);
        assert_eq!(blocked, 1);
        // The loading wrapper agrees with the in-memory version.
        let (owned, owned_blocked) = actionable(&cfg, &filter).unwrap();
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].id, "TASK-003");
        assert_eq!(owned_blocked, 1);
    }

    #[test]
    fn test_task_move_invalid_status() {
        let (_tmp, cfg) = setup_repo();

        new::create_task_programmatic(
            &cfg,
            "Test task",
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

        let result = move_task_programmatic(&cfg, "TASK-001", "nonexistent", None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid task status"));
    }

    #[test]
    fn test_task_move_with_sprint() {
        let (_tmp, cfg) = setup_repo();

        new::create_task_programmatic(
            &cfg,
            "Sprint task",
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

        let result =
            move_task_programmatic(&cfg, "TASK-001", "in-progress", Some("SPR-001")).unwrap();
        let path_str = result["path"].as_str().unwrap();

        let content = std::fs::read_to_string(path_str).unwrap();
        let (fm_str, _) = frontmatter::split_frontmatter(&content).unwrap();
        let fm = frontmatter::parse_raw(&fm_str, std::path::Path::new(path_str)).unwrap();
        assert_eq!(frontmatter::get_str(&fm, "sprint").unwrap(), "[[SPR-001]]");
    }
}
