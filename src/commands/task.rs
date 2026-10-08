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
use serde_yaml::Value;
use std::collections::HashSet;

/// Finished statuses (end of the task lifecycle).
const FINISHED_STATUSES: &[&str] = entity::FINISHED_TASK_STATUSES;

pub fn run(subcmd: &TaskSubcommand, cfg: &ResolvedConfig) -> McResult<()> {
    match subcmd {
        TaskSubcommand::Board {
            project,
            customer,
            sprint,
            milestone,
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

                milestone: milestone.as_deref(),
            };
            run_board(cfg, &filter, *limit, *all)
        }
        TaskSubcommand::Move { id, status, sprint } => run_move(cfg, id, status, sprint.as_deref()),
        TaskSubcommand::Set {
            id,
            title,
            status,
            priority,
            owner,
            sprint,
            milestone,
            due_date,
            project,
            customer,
            tags,
            depends_on,
        } => {
            let list = |v: &Option<String>| v.as_deref().map(util::parse_comma_list);
            let update = TaskUpdate {
                title: title.clone(),
                status: status.clone(),
                priority: *priority,
                owner: owner.clone(),
                sprint: sprint.clone(),
                milestone: milestone.clone(),
                due_date: due_date.clone(),
                projects: list(project),
                customers: list(customer),
                tags: list(tags),
                depends_on: list(depends_on),
            };
            run_set(cfg, id, &update)
        }
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

fn repo_relative(cfg: &ResolvedConfig, path: &str) -> String {
    let p = std::path::Path::new(path);
    p.strip_prefix(&cfg.root).unwrap_or(p).display().to_string()
}

fn priority_of(e: &EntityRecord) -> u32 {
    data::get_number(&e.frontmatter, "priority").unwrap_or(3)
}

fn describe_filter(filter: &TaskFilter) -> Vec<String> {
    [
        ("project", filter.project),
        ("customer", filter.customer),
        ("sprint", filter.sprint),
        (
            "milestone",
            filter
                .milestone
                .map(|m| if m.is_empty() { "none" } else { m }),
        ),
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
    let owner = ui::clean(s(t, "owner"));
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
                        1 => ui::clean(s(t, "title")).into_owned(),
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
                ui::clean(s(t, "title"))
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
    // Sprint ID or title, stored as the sprint's ID.
    let sprint = sprint
        .map(|sp| crate::commands::new::resolve_sprint(cfg, sp))
        .transpose()?;
    let old_status = frontmatter::get_str_or(&task.frontmatter, "status", "backlog").to_string();
    let title = ui::clean(s(&task, "title")).into_owned();

    // A file in the wrong folder for its status is still moved into place.
    let in_place = task
        .source_path
        .parent()
        .and_then(|d| d.file_name())
        .is_some_and(|d| d == entity::task_status_folder(&new_status));
    if old_status == new_status && sprint.is_none() && in_place {
        if ui::get().json {
            let out = serde_json::json!({
                "id": task.id,
                "old_status": old_status,
                "new_status": new_status,
                "path": repo_relative(cfg, &task.source_path.display().to_string()),
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

    let mut result = move_task_programmatic(cfg, &task.id, &new_status, sprint.as_deref())?;

    if ui::get().json {
        // Repo-relative like `_source` and every other command's `path`.
        if let Some(p) = result.get_mut("path") {
            *p = JsonValue::from(repo_relative(cfg, p.as_str().unwrap_or_default()));
        }
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
    let moved = result["path"]
        .as_str()
        .map(std::path::Path::new)
        .and_then(|p| p.parent())
        .is_some_and(|dir| Some(dir) != task.source_path.parent());
    if moved {
        detail = format!(
            "{detail}  {}",
            format!("{} moved to {folder}/", g.sep).dimmed()
        );
    }
    if let Some(sp) = &sprint {
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
// Set
// ---------------------------------------------------------------------------

fn run_set(cfg: &ResolvedConfig, id: &str, update: &TaskUpdate) -> McResult<()> {
    if update.is_empty() {
        return Err(McError::usage(
            "Nothing to change",
            Some(format!(
                "pass at least one field, e.g. {}",
                ui::cmd(&format!("mc task set {id} --priority 2"))
            )),
        ));
    }
    let task = suggest::find_entity(id, cfg, Some(EntityKind::Task))?;
    if task.kind != EntityKind::Task {
        return Err(McError::usage(
            format!("{} is a {}, not a task", task.id, task.kind.label()),
            Some("task set only works on task IDs (TASK-001)".into()),
        ));
    }
    let updated = update_task(cfg, &task.id, update)?;

    if ui::get().json {
        println!("{}", serde_json::to_string_pretty(&updated.to_json(cfg))?);
        return Ok(());
    }
    if updated.changed.is_empty() {
        ui::info(format!(
            "{} already has those values {}",
            task.id.cyan().bold(),
            "(nothing changed)".dimmed()
        ));
        return Ok(());
    }
    let after = data::find_entity_by_id(&task.id, cfg).unwrap_or(task);
    let g = ui::glyphs();
    ui::success(format!(
        "{} {}",
        after.id.cyan().bold(),
        ui::clean(s(&after, "title")).bold()
    ));
    for field in &updated.changed {
        let value = match *field {
            "status" => format!(
                "{} {} {}",
                ui::status(&updated.old_status),
                g.arrow.dimmed(),
                ui::status(&updated.new_status)
            ),
            "priority" => {
                let p = priority_of(&after);
                format!("{} {}", ui::priority(p), ui::priority_label(p))
            }
            _ => {
                let text = match *field {
                    "projects" | "customers" | "tags" | "depends_on" => {
                        frontmatter::get_link_list(&after.frontmatter, field).join(", ")
                    }
                    _ => frontmatter::get_link_str(&after.frontmatter, field)
                        .unwrap_or("")
                        .to_string(),
                };
                if text.is_empty() {
                    "(cleared)".dimmed().to_string()
                } else {
                    ui::clean(&text).into_owned()
                }
            }
        };
        println!(
            "  {} {value}",
            format!("{:<10}", field.replace('_', " ")).dimmed()
        );
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

        milestone: None,
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
    let finished: Vec<&EntityRecord> = all
        .iter()
        .filter(|t| FINISHED_STATUSES.contains(&s(t, "status")))
        .collect();
    let done_ids: HashSet<&str> = finished.iter().map(|t| t.id.as_str()).collect();
    // Hand-written dependencies may differ in case or padding (`task-1`).
    let done_keys: HashSet<(String, u64)> = finished
        .iter()
        .filter_map(|t| data::loose_id_key(&t.id))
        .collect();
    let dep_done = |dep: &str| {
        done_ids.contains(dep) || data::loose_id_key(dep).is_some_and(|k| done_keys.contains(&k))
    };

    let mut blocked = 0;
    let mut candidates: Vec<&EntityRecord> = all
        .iter()
        .filter(|t| filter.matches(&t.frontmatter))
        .filter(|t| matches!(s(t, "status"), "todo" | "backlog"))
        .filter(|t| {
            let ok = frontmatter::get_link_list(&t.frontmatter, "depends_on")
                .iter()
                .all(|dep| dep_done(dep));
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

        milestone: None,
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
    println!("    {}", ui::clean(s(next, "title")).bold());

    let mut meta = vec![ui::status(s(next, "status"))];
    let owner = ui::clean(s(next, "owner"));
    meta.push(if owner.is_empty() {
        "unassigned".dimmed().to_string()
    } else {
        format!("@{owner}")
    });
    let projects = frontmatter::get_link_list(&next.frontmatter, "projects");
    if let Some(p) = projects.first() {
        meta.push(p.clone());
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
                ui::clean(s(t, "title"))
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
// ---------------------------------------------------------------------------

/// Change a task's status (and optionally its sprint), moving the file
/// between `todo/` and `done/` as needed. Shared by the CLI, MCP, the REST
/// API and the dashboard.
///
/// `id` must be a task ID; other kinds are a usage error. The status is
/// matched leniently (`In Progress` -> `in-progress`). `sprint` may be a
/// sprint ID or title and is stored as the sprint's ID; an empty string
/// clears it. The whole read-modify-write holds the repo's write lock.
pub fn move_task_programmatic(
    cfg: &ResolvedConfig,
    id: &str,
    new_status: &str,
    sprint: Option<&str>,
) -> McResult<JsonValue> {
    // Only tasks have todo/ and done/ folders; moving anything else would
    // drag it out of its own directory.
    let kind = EntityKind::from_id(id, cfg)?;
    if kind != EntityKind::Task {
        return Err(McError::usage(
            format!("{id} is a {}, not a task", kind.label()),
            Some("Only tasks can be moved (IDs like TASK-001).".into()),
        ));
    }

    let new_status = canonical_task_status(cfg, new_status)?;
    let new_status = new_status.as_str();
    let sprint = sprint
        .map(|sp| crate::commands::new::resolve_sprint(cfg, sp))
        .transpose()?;

    let _lock = crate::lock::acquire(cfg)?;

    // Find the task
    let task = data::find_entity_by_id(id, cfg)?;
    let old_path = task.source_path.clone();
    let old_status = frontmatter::get_str_or(&task.frontmatter, "status", "backlog").to_string();

    // Read the full file content
    let content = std::fs::read_to_string(&old_path)?;
    let (fm_str, body) = frontmatter::split_frontmatter(&content)
        .ok_or_else(|| McError::Other("Task file has no frontmatter".into()))?;
    let mut fm = frontmatter::parse_in_file(&content, &fm_str, &old_path)?;

    // Update frontmatter fields
    frontmatter::set_str(&mut fm, "status", new_status);
    frontmatter::set_str(&mut fm, "updated", &util::today_str());
    if let Some(sp) = &sprint {
        frontmatter::set_str(&mut fm, "sprint", &frontmatter::wrap_wikilink(sp));
    }

    let new_doc = frontmatter::serialize_document(&fm, &body);

    // Tasks live in `<tasks>/todo/` or `<tasks>/done/` depending on status.
    // Decide from where the file actually is, not from its old status: a
    // hand-edited status may not match its folder.
    let tasks_dir = old_path
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| McError::Other("Cannot determine task directory".into()))?;
    let filename = old_path
        .file_name()
        .ok_or_else(|| McError::Other("Cannot determine task filename".into()))?;
    let target_dir = tasks_dir.join(entity::task_status_folder(new_status));
    let new_path = target_dir.join(filename);
    let final_path = if new_path == old_path {
        util::atomic_write(&old_path, new_doc.as_bytes())?;
        old_path.clone()
    } else {
        if new_path.exists() {
            return Err(McError::conflict(
                format!("{} already exists.", new_path.display()),
                Some("Two copies of this task exist; remove one and try again.".into()),
            ));
        }
        std::fs::create_dir_all(&target_dir)?;
        util::atomic_write(&new_path, new_doc.as_bytes())?;
        std::fs::remove_file(&old_path)?;
        new_path
    };

    Ok(serde_json::json!({
        "id": task.id,
        "old_status": old_status,
        "new_status": new_status,
        "path": final_path.display().to_string(),
    }))
}

/// A task status in its configured spelling (`In Progress`, `wip` ->
/// `in-progress`), or a usage error listing the valid ones (400 over HTTP,
/// invalid_params over MCP).
fn canonical_task_status(cfg: &ResolvedConfig, status: &str) -> McResult<String> {
    let valid_statuses = &cfg.statuses.task;
    match suggest::match_status(status, valid_statuses) {
        Some(s) => Ok(s.to_string()),
        None => {
            let suggestion =
                suggest::did_you_mean(status, valid_statuses.iter().map(String::as_str))
                    .map(|s| format!(" Did you mean '{s}'?"))
                    .unwrap_or_default();
            Err(McError::usage(
                format!(
                    "Invalid task status '{}'.{} Valid statuses: {}",
                    status,
                    suggestion,
                    valid_statuses.join(", ")
                ),
                None,
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Field updates (`mc task set`, MCP `update_task`, REST and dashboard PATCH)
// ---------------------------------------------------------------------------

/// Changes for [`update_task`]; `None` leaves a field as it is. An empty
/// string clears a text field or the sprint, an empty list a list field.
#[derive(Debug, Clone, Default)]
pub struct TaskUpdate {
    pub title: Option<String>,
    /// Matched leniently like `mc task move`; may move the file between
    /// `todo/` and `done/`.
    pub status: Option<String>,
    /// 1 (critical) to 4 (low).
    pub priority: Option<u32>,
    pub owner: Option<String>,
    /// Sprint ID or title, stored as the sprint's ID.
    pub sprint: Option<String>,
    pub milestone: Option<String>,
    /// `YYYY-MM-DD`.
    pub due_date: Option<String>,
    /// Replace the linked projects (IDs of existing projects, loose forms ok).
    pub projects: Option<Vec<String>>,
    /// Replace the linked customers (IDs of existing customers).
    pub customers: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    /// Replace the dependencies (IDs of existing tasks).
    pub depends_on: Option<Vec<String>>,
}

impl TaskUpdate {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.status.is_none()
            && self.priority.is_none()
            && self.owner.is_none()
            && self.sprint.is_none()
            && self.milestone.is_none()
            && self.due_date.is_none()
            && self.projects.is_none()
            && self.customers.is_none()
            && self.tags.is_none()
            && self.depends_on.is_none()
    }
}

/// The outcome of [`update_task`].
#[derive(Debug, Clone)]
pub struct TaskUpdated {
    pub id: String,
    /// Fields whose value actually changed, `status` last. Empty when the
    /// task already had every given value (the file is left alone then).
    pub changed: Vec<&'static str>,
    pub old_status: String,
    pub new_status: String,
    /// The task file after the update.
    pub path: std::path::PathBuf,
}

impl TaskUpdated {
    /// `{id, changed, old_status, new_status, path}` with `path` relative to
    /// the repo root, as MCP, REST and `mc --json task set` return it.
    pub fn to_json(&self, cfg: &ResolvedConfig) -> JsonValue {
        serde_json::json!({
            "id": self.id,
            "changed": self.changed,
            "old_status": self.old_status,
            "new_status": self.new_status,
            "path": crate::mcp::repo_relative(cfg, &self.path),
        })
    }
}

/// One line of text: trimmed, no control characters.
fn single_line(field: &str, value: &str) -> McResult<String> {
    let value = value.trim();
    if value.chars().any(char::is_control) {
        return Err(McError::usage(
            format!("The {field} must be a single line of text"),
            None,
        ));
    }
    Ok(value.to_string())
}

/// Change a task's fields: title, status, priority, owner, sprint, due
/// date, projects, customers, tags and dependencies. Shared by `mc task set`,
/// MCP `update_task`, REST `PATCH /v1/tasks/{id}` and the dashboard, so every
/// surface validates the same way as `mc new task`.
///
/// `id` must be a task ID (callers resolve loose input first). Every value is
/// checked before the file is touched; references must name existing
/// entities and are stored as `[[ID]]` links. A new title also replaces the
/// body's leading `# Old title` heading. The status changes last, through
/// [`move_task_programmatic`], so the file lands in the right folder with
/// the new fields. Unchanged values leave the file alone.
pub fn update_task(cfg: &ResolvedConfig, id: &str, update: &TaskUpdate) -> McResult<TaskUpdated> {
    let kind = EntityKind::from_id(id, cfg)?;
    if kind != EntityKind::Task {
        return Err(McError::usage(
            format!("{id} is a {}, not a task", kind.label()),
            Some("Only tasks can be updated this way (IDs like TASK-001).".into()),
        ));
    }
    if update.is_empty() {
        return Err(McError::usage(
            "Nothing to change",
            Some("give at least one of title, status, priority, owner, sprint, due date, projects, customers, tags or depends_on".into()),
        ));
    }

    // Validate everything first, so a bad value changes nothing.
    let title = update
        .title
        .as_deref()
        .map(|t| single_line("title", t))
        .transpose()?;
    if title.as_deref() == Some("") {
        return Err(McError::usage("A task needs a title", None));
    }
    let status = update
        .status
        .as_deref()
        .map(|s| canonical_task_status(cfg, s))
        .transpose()?;
    if let Some(p) = update.priority {
        crate::commands::new::validate_priority(p)?;
    }
    let owner = update
        .owner
        .as_deref()
        .map(|o| single_line("owner", o))
        .transpose()?;
    let sprint = update
        .sprint
        .as_deref()
        .map(|sp| crate::commands::new::resolve_sprint(cfg, sp))
        .transpose()?;
    let milestone = update
        .milestone
        .as_deref()
        .map(|s| crate::commands::new::resolve_milestone(cfg, s))
        .transpose()?;
    let due_date = update.due_date.as_deref().map(str::trim);
    if let Some(d) = due_date.filter(|d| !d.is_empty()) {
        crate::commands::new::validate_date(d, "due date")?;
    }
    let refs = |kind, list: &Option<Vec<String>>| {
        list.as_deref()
            .map(|l| crate::commands::new::resolve_refs(cfg, kind, l))
            .transpose()
    };
    let projects = refs(EntityKind::Project, &update.projects)?;
    let customers = refs(EntityKind::Customer, &update.customers)?;
    let depends_on = refs(EntityKind::Task, &update.depends_on)?;
    if depends_on
        .as_ref()
        .is_some_and(|d| d.iter().any(|d| d == id))
    {
        return Err(McError::usage(format!("{id} can't depend on itself"), None));
    }
    let tags = update.tags.as_ref().map(|tags| {
        let mut out: Vec<String> = Vec::new();
        for t in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
            if !out.iter().any(|o| o == t) {
                out.push(t.to_string());
            }
        }
        out
    });

    let _lock = crate::lock::acquire(cfg)?;
    let task = data::find_entity_by_id(id, cfg)?;
    let fm = &task.frontmatter;
    let old_title = frontmatter::get_str_or(fm, "title", "").trim().to_string();
    let old_status = frontmatter::get_str_or(fm, "status", "backlog").to_string();

    // Only the values that differ from the file are written.
    let differs =
        |key: &str, new: &str| frontmatter::get_link_str(fm, key).unwrap_or("").trim() != new;
    let title = title.filter(|t| *t != old_title);
    let priority = update
        .priority
        .filter(|p| data::get_number(fm, "priority") != Some(*p));
    let owner = owner.filter(|o| differs("owner", o));
    let sprint = sprint.filter(|sp| differs("sprint", sp));
    let milestone = milestone.filter(|s| differs("milestone", s));
    let due_date = due_date.filter(|d| differs("due_date", d));
    let list_differs = |key: &str, new: &[String]| frontmatter::get_link_list(fm, key) != new;
    let projects = projects.filter(|l| list_differs("projects", l));
    let customers = customers.filter(|l| list_differs("customers", l));
    let depends_on = depends_on.filter(|l| list_differs("depends_on", l));
    let tags = tags.filter(|l| frontmatter::get_string_list(fm, "tags") != *l);
    let status = status.filter(|s| *s != old_status);

    let mut changed = Vec::new();
    for (field, is_changed) in [
        ("title", title.is_some()),
        ("priority", priority.is_some()),
        ("owner", owner.is_some()),
        ("sprint", sprint.is_some()),
        ("milestone", milestone.is_some()),
        ("due_date", due_date.is_some()),
        ("projects", projects.is_some()),
        ("customers", customers.is_some()),
        ("tags", tags.is_some()),
        ("depends_on", depends_on.is_some()),
    ] {
        if is_changed {
            changed.push(field);
        }
    }

    let mut path = task.source_path.clone();
    if !changed.is_empty() {
        let seq = |items: &[String], link: bool| {
            Value::Sequence(
                items
                    .iter()
                    .map(|i| {
                        Value::String(if link {
                            frontmatter::wrap_wikilink(i)
                        } else {
                            i.clone()
                        })
                    })
                    .collect(),
            )
        };
        frontmatter::update_file(&path, |fm| {
            if let Some(title) = &title {
                frontmatter::set_str(fm, "title", title);
            }
            if let Some(owner) = &owner {
                frontmatter::set_str(fm, "owner", owner);
            }
            if let Some(milestone) = &milestone {
                frontmatter::set_str(fm, "milestone", &frontmatter::wrap_wikilink(milestone));
            }
            if let Some(sprint) = &sprint {
                frontmatter::set_str(fm, "sprint", &frontmatter::wrap_wikilink(sprint));
            }
            if let Some(due) = due_date {
                frontmatter::set_str(fm, "due_date", due);
            }
            if let Some(map) = fm.as_mapping_mut() {
                let mut put = |key: &str, v: Value| {
                    map.insert(Value::String(key.into()), v);
                };
                if let Some(p) = priority {
                    put("priority", Value::Number(u64::from(p).into()));
                }
                for (key, list, link) in [
                    ("projects", &projects, true),
                    ("customers", &customers, true),
                    ("tags", &tags, false),
                    ("depends_on", &depends_on, true),
                ] {
                    if let Some(list) = list {
                        put(key, seq(list, link));
                    }
                }
            }
            frontmatter::set_str(fm, "updated", &util::today_str());
        })?;
        if let Some(title) = title.as_deref().filter(|_| !old_title.is_empty()) {
            retitle_body(&path, &old_title, title)?;
        }
    }
    if let Some(status) = &status {
        let moved = move_task_programmatic(cfg, &task.id, status, None)?;
        if let Some(p) = moved["path"].as_str() {
            path = p.into();
        }
        changed.push("status");
    }

    Ok(TaskUpdated {
        id: task.id,
        changed,
        new_status: status.unwrap_or_else(|| old_status.clone()),
        old_status,
        path,
    })
}

/// Replace the body's first line `# {old}` with `# {new}`; any other
/// opening is left alone.
fn retitle_body(path: &std::path::Path, old: &str, new: &str) -> McResult<()> {
    let content = std::fs::read_to_string(path)?;
    let Some((_, body)) = frontmatter::split_frontmatter(&content) else {
        return Ok(());
    };
    let head = &content[..content.len() - body.len()];
    let skipped = body.len() - body.trim_start().len();
    let rest = &body[skipped..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let line = rest[..line_end].trim_end_matches('\r');
    if line.trim_end() != format!("# {old}") {
        return Ok(());
    }
    let updated = format!("{head}{}# {new}{}", &body[..skipped], &rest[line.len()..]);
    util::atomic_write(path, updated.as_bytes())
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

    fn read_fm(path: &std::path::Path) -> (Value, String) {
        frontmatter::parse_file(path).unwrap()
    }

    #[test]
    fn update_task_changes_fields_and_moves_on_status() {
        let (_tmp, cfg) = setup_repo();
        new::create_customer(&cfg, &new::CustomerInput::new("Acme")).unwrap();
        new::create_sprint(&cfg, &new::SprintInput::new("2026-W05")).unwrap();
        new::create_task(&cfg, &new::TaskInput::new("Dependency")).unwrap();
        let created = new::create_task(&cfg, &new::TaskInput::new("Ship it")).unwrap();

        let update = TaskUpdate {
            title: Some("Ship it now".into()),
            status: Some("Done".into()),
            priority: Some(1),
            owner: Some("alice".into()),
            sprint: Some("2026-w05".into()),
            due_date: Some("2026-10-31".into()),
            customers: Some(vec!["cust-1".into()]),
            tags: Some(vec!["a".into(), " b ".into(), "a".into()]),
            depends_on: Some(vec!["task-1".into()]),
            ..Default::default()
        };
        let done = update_task(&cfg, "TASK-002", &update).unwrap();
        assert_eq!(
            done.changed,
            [
                "title",
                "priority",
                "owner",
                "sprint",
                "due_date",
                "customers",
                "tags",
                "depends_on",
                "status"
            ]
        );
        assert_eq!(
            (done.old_status.as_str(), done.new_status.as_str()),
            ("backlog", "done")
        );
        assert!(!created.path.exists());
        assert!(done.path.starts_with(cfg.tasks_dir.join("done")));
        assert_eq!(done.to_json(&cfg)["path"], "tasks/done/TASK-002-ship-it.md");

        let (fm, body) = read_fm(&done.path);
        assert_eq!(frontmatter::get_str(&fm, "title"), Some("Ship it now"));
        assert_eq!(frontmatter::get_str(&fm, "status"), Some("done"));
        assert_eq!(data::get_number(&fm, "priority"), Some(1));
        assert_eq!(frontmatter::get_str(&fm, "owner"), Some("alice"));
        // Sprint by title and loose references are stored as canonical links.
        assert_eq!(frontmatter::get_str(&fm, "sprint"), Some("[[SPR-001]]"));
        assert_eq!(
            frontmatter::get_string_list(&fm, "customers"),
            ["[[CUST-001]]"]
        );
        assert_eq!(
            frontmatter::get_string_list(&fm, "depends_on"),
            ["[[TASK-001]]"]
        );
        assert_eq!(frontmatter::get_string_list(&fm, "tags"), ["a", "b"]);
        assert!(body.trim_start().starts_with("# Ship it now"), "{body}");

        // The same values again change nothing, not even `updated`.
        let before = std::fs::read_to_string(&done.path).unwrap();
        let again = update_task(&cfg, "TASK-002", &update).unwrap();
        assert!(again.changed.is_empty(), "{:?}", again.changed);
        assert_eq!(std::fs::read_to_string(&done.path).unwrap(), before);

        // Empty values clear.
        let cleared = update_task(
            &cfg,
            "TASK-002",
            &TaskUpdate {
                owner: Some(String::new()),
                sprint: Some(String::new()),
                due_date: Some(String::new()),
                customers: Some(Vec::new()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            cleared.changed,
            ["owner", "sprint", "due_date", "customers"]
        );
        let (fm, _) = read_fm(&cleared.path);
        assert_eq!(frontmatter::get_str(&fm, "owner"), Some(""));
        assert_eq!(frontmatter::get_str(&fm, "sprint"), Some(""));
        assert!(frontmatter::get_string_list(&fm, "customers").is_empty());
    }

    #[test]
    fn update_task_rejects_bad_values_without_writing() {
        let (_tmp, cfg) = setup_repo();
        let created = new::create_task(&cfg, &new::TaskInput::new("Careful")).unwrap();
        new::create_meeting(&cfg, &new::MeetingInput::new("Sync")).unwrap();
        let before = std::fs::read_to_string(&created.path).unwrap();
        let bad = [
            TaskUpdate::default(),
            TaskUpdate {
                title: Some("  ".into()),
                ..Default::default()
            },
            TaskUpdate {
                title: Some("two\nlines".into()),
                ..Default::default()
            },
            TaskUpdate {
                status: Some("finished-ish".into()),
                ..Default::default()
            },
            TaskUpdate {
                priority: Some(5),
                ..Default::default()
            },
            TaskUpdate {
                due_date: Some("2026-1-5".into()),
                ..Default::default()
            },
            TaskUpdate {
                sprint: Some("SPR-404".into()),
                ..Default::default()
            },
            TaskUpdate {
                projects: Some(vec!["PROJ-404".into()]),
                ..Default::default()
            },
            TaskUpdate {
                depends_on: Some(vec!["TASK-001".into()]),
                ..Default::default()
            },
            // A valid change next to an invalid one is not applied either.
            TaskUpdate {
                owner: Some("bob".into()),
                priority: Some(0),
                ..Default::default()
            },
        ];
        for update in &bad {
            assert!(update_task(&cfg, "TASK-001", update).is_err(), "{update:?}");
        }
        assert_eq!(std::fs::read_to_string(&created.path).unwrap(), before);

        let owner = TaskUpdate {
            owner: Some("bob".into()),
            ..Default::default()
        };
        assert!(matches!(
            update_task(&cfg, "MTG-001", &owner),
            Err(McError::Usage { .. })
        ));
        assert!(matches!(
            update_task(&cfg, "TASK-009", &owner),
            Err(McError::EntityNotFound(_))
        ));
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
        new::create_sprint(&cfg, &new::SprintInput::new("Alpha")).unwrap();

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

        // A sprint title is stored as the sprint's ID.
        let result =
            move_task_programmatic(&cfg, "TASK-001", "in-progress", Some("alpha")).unwrap();
        let path_str = result["path"].as_str().unwrap();

        let content = std::fs::read_to_string(path_str).unwrap();
        let (fm_str, _) = frontmatter::split_frontmatter(&content).unwrap();
        let fm = frontmatter::parse_raw(&fm_str, std::path::Path::new(path_str)).unwrap();
        assert_eq!(frontmatter::get_str(&fm, "sprint").unwrap(), "[[SPR-001]]");
    }

    fn status_of(path: &std::path::Path) -> String {
        let (fm, _) = frontmatter::parse_file(path).unwrap();
        frontmatter::get_str(&fm, "status").unwrap().to_string()
    }

    #[test]
    fn test_move_keeps_a_task_already_in_the_target_folder() {
        let (_tmp, cfg) = setup_repo();
        let created = new::create_task(&cfg, &new::TaskInput::new("Write report")).unwrap();
        // Hand-moved to done/ while its status still says backlog.
        let misplaced = cfg
            .tasks_dir
            .join("done")
            .join(created.path.file_name().unwrap());
        std::fs::rename(&created.path, &misplaced).unwrap();
        let result = move_task_programmatic(&cfg, "TASK-001", "done", None).unwrap();
        assert_eq!(
            result["path"].as_str().unwrap(),
            misplaced.to_str().unwrap()
        );
        assert!(misplaced.is_file(), "task file was deleted");
        assert_eq!(status_of(&misplaced), "done");

        // The reverse: a done/ status in todo/, moved back to todo.
        let todo = cfg
            .tasks_dir
            .join("todo")
            .join(misplaced.file_name().unwrap());
        std::fs::rename(&misplaced, &todo).unwrap();
        move_task_programmatic(&cfg, "TASK-001", "todo", None).unwrap();
        assert!(todo.is_file(), "task file was deleted");
        assert_eq!(status_of(&todo), "todo");
    }

    #[test]
    fn test_move_never_overwrites_a_second_copy() {
        let (_tmp, cfg) = setup_repo();
        let created = new::create_task(&cfg, &new::TaskInput::new("Twice")).unwrap();
        let copy = cfg
            .tasks_dir
            .join("done")
            .join(created.path.file_name().unwrap());
        std::fs::copy(&created.path, &copy).unwrap();
        std::fs::write(&copy, std::fs::read_to_string(&copy).unwrap() + "copy\n").unwrap();
        let err = move_task_programmatic(&cfg, "TASK-001", "done", None).unwrap_err();
        assert!(matches!(err, McError::Conflict { .. }), "{err}");
        assert!(created.path.is_file());
        assert!(std::fs::read_to_string(&copy).unwrap().ends_with("copy\n"));
    }

    #[test]
    fn test_move_rejects_other_kinds() {
        let (_tmp, cfg) = setup_repo();
        new::create_customer(&cfg, &new::CustomerInput::new("Acme")).unwrap();
        let meeting = new::create_meeting(&cfg, &new::MeetingInput::new("Kickoff")).unwrap();
        for id in ["MTG-001", "CUST-001"] {
            let err = move_task_programmatic(&cfg, id, "todo", None).unwrap_err();
            assert!(matches!(err, McError::Usage { .. }), "{err}");
            assert!(err.to_string().contains("not a task"), "{err}");
        }
        assert!(meeting.path.is_file());
        assert!(!cfg.root.join("todo").exists());
        assert!(!cfg.customers_dir.join("todo").exists());
    }

    #[test]
    fn test_move_custom_status_and_unknown_sprint() {
        let (_tmp, mut cfg) = setup_repo();
        cfg.statuses.task.push("blocked".into());
        new::create_task(&cfg, &new::TaskInput::new("Vendor")).unwrap();
        let result = move_task_programmatic(&cfg, "TASK-001", "Blocked", None).unwrap();
        assert_eq!(result["new_status"], "blocked");
        assert!(result["path"].as_str().unwrap().contains("/todo/"));
        let err = move_task_programmatic(&cfg, "TASK-001", "todo", Some("SPR-999")).unwrap_err();
        assert!(matches!(err, McError::NotFound { .. }), "{err}");
    }

    #[test]
    fn test_loose_dependency_ids_unblock() {
        let (_tmp, cfg) = setup_repo();
        let mut done = new::TaskInput::new("Done dep");
        done.status = Some("done".into());
        new::create_task(&cfg, &done).unwrap();
        new::create_task(&cfg, &new::TaskInput::new("Open")).unwrap();
        // A hand-written legacy reference in another spelling.
        let path = cfg.tasks_dir.join("todo").join("TASK-002-open.md");
        frontmatter::update_file(&path, |fm| {
            fm.as_mapping_mut().unwrap().insert(
                "depends_on".into(),
                serde_yaml::Value::Sequence(vec!["[[task-1]]".into()]),
            );
        })
        .unwrap();
        let all = data::collect_tasks(&cfg).unwrap();
        let (queue, blocked) = actionable_in(&all, &TaskFilter::all());
        assert_eq!(queue.len(), 1);
        assert_eq!(blocked, 0);
    }
}
