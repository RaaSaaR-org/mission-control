use crate::cli::suggest;
use crate::cli::ui::{self, Col, Table};
use crate::cli::{ListEntity, TaskSort};
use crate::commands::index;
use crate::config::ResolvedConfig;
use crate::data::{self, ContactFilter, EntityRecord, TaskFilter};
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::frontmatter;
use colored::*;
use std::borrow::Cow;

pub fn run(entity: &ListEntity, cfg: &ResolvedConfig) -> McResult<()> {
    match entity {
        ListEntity::Tasks { .. } => list_tasks(cfg, entity),
        ListEntity::Contacts {
            status,
            tag,
            customer,
        } => list_contacts(cfg, status, tag, customer),
        _ => {
            let (kind, status_filter, tag_filter) = match entity {
                ListEntity::Customers { status, tag } => (EntityKind::Customer, status, tag),
                ListEntity::Projects { status, tag } => (EntityKind::Project, status, tag),
                ListEntity::Meetings { status, tag } => (EntityKind::Meeting, status, tag),
                ListEntity::Research { status, tag } => (EntityKind::Research, status, tag),
                ListEntity::Milestones { status } => (EntityKind::Milestone, status, &None),
                ListEntity::Sprints { status, tag } => (EntityKind::Sprint, status, tag),
                ListEntity::Proposals { status, tag } => (EntityKind::Proposal, status, tag),
                ListEntity::Tasks { .. } | ListEntity::Contacts { .. } => {
                    unreachable!("Handled in outer match arms")
                }
            };
            if !cfg.entity_available(&kind) {
                return Err(McError::not_available(kind, cfg));
            }
            list_standard(kind, cfg, status_filter, tag_filter)
        }
    }
}

/// Resolve a `--status` filter against configured statuses and the statuses
/// actually present in the data, failing with a suggestion on typos.
fn resolve_status_filter(
    input: Option<&str>,
    kind: EntityKind,
    cfg: &ResolvedConfig,
    all: &[EntityRecord],
) -> McResult<Option<String>> {
    let Some(input) = input else {
        return Ok(None);
    };
    let mut known: Vec<String> = kind.statuses(cfg).to_vec();
    for e in all {
        if let Some(s) = frontmatter::get_str(&e.frontmatter, "status") {
            if !known.iter().any(|k| k == s) {
                known.push(s.to_string());
            }
        }
    }
    suggest::resolve_status(input, &known, kind).map(Some)
}

fn has_status(e: &EntityRecord, status: &Option<String>) -> bool {
    match status {
        Some(s) => frontmatter::get_str(&e.frontmatter, "status")
            .is_some_and(|v| v.eq_ignore_ascii_case(s)),
        None => true,
    }
}

/// Print entities as a JSON array (same shape as `data/*.json`).
fn print_json(entries: &[EntityRecord], cfg: &ResolvedConfig) -> McResult<()> {
    let arr: Vec<_> = entries.iter().map(|e| index::entity_json(e, cfg)).collect();
    println!("{}", serde_json::to_string_pretty(&arr)?);
    Ok(())
}

fn filter_summary(filters: &[(&str, Option<String>)]) -> Vec<String> {
    filters
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={v}")))
        .collect()
}

/// Footer line ("6 customers", "3 of 74 tasks · status=todo") or empty state.
fn print_footer(kind: EntityKind, shown: usize, total: usize, filters: &[String]) {
    let g = ui::glyphs();
    let noun = |n| ui::count(n, kind.label(), kind.label_plural());
    if shown == 0 && !ui::get().interactive {
        // Keep stdout empty for pipes (`| wc -l`); explain on stderr.
        eprintln!("No {} found.", kind.label_plural());
        return;
    }
    if shown == 0 {
        if total == 0 {
            ui::info(format!("No {} yet.", kind.label_plural()));
            ui::hint(format!(
                "create one: {}",
                ui::cmd(&format!("mc new {} \"...\"", kind.label()))
            ));
        } else {
            ui::info(format!(
                "No {} match {} ({} in total).",
                kind.label_plural(),
                filters.join(" "),
                noun(total)
            ));
            ui::hint(format!(
                "drop filters to see everything: {}",
                ui::cmd(&format!("mc list {}", kind.label_plural()))
            ));
        }
        return;
    }
    if !ui::get().interactive {
        return;
    }
    let mut line = if filters.is_empty() {
        noun(shown)
    } else {
        format!("{} of {}", shown, noun(total))
    };
    if !filters.is_empty() {
        line = format!("{line} {} {}", g.sep, filters.join(" "));
    }
    println!("\n  {}", line.dimmed());
}

/// A frontmatter value as display text (control characters removed).
fn s<'a>(e: &'a EntityRecord, key: &str) -> Cow<'a, str> {
    ui::clean(raw(e, key))
}

/// A frontmatter value as written, for comparisons.
fn raw<'a>(e: &'a EntityRecord, key: &str) -> &'a str {
    frontmatter::get_str_or(&e.frontmatter, key, "")
}

fn links(e: &EntityRecord, key: &str) -> String {
    ui::clean(&frontmatter::get_link_list(&e.frontmatter, key).join(", ")).into_owned()
}

fn id_cell(e: &EntityRecord) -> String {
    ui::clean(&e.id).cyan().to_string()
}

fn list_standard(
    kind: EntityKind,
    cfg: &ResolvedConfig,
    status_filter: &Option<String>,
    tag_filter: &Option<String>,
) -> McResult<()> {
    let mut all = data::collect_entities(kind, cfg)?;
    if kind == EntityKind::Meeting {
        // Chronological, so dates don't jump around when IDs were assigned
        // out of order.
        all.sort_by(|a, b| {
            (raw(a, "date"), raw(a, "time"), data::id_sort_key(&a.id)).cmp(&(
                raw(b, "date"),
                raw(b, "time"),
                data::id_sort_key(&b.id),
            ))
        });
    } else {
        all.sort_by(|a, b| data::id_sort_key(&a.id).cmp(&data::id_sort_key(&b.id)));
    }
    let status = resolve_status_filter(status_filter.as_deref(), kind, cfg, &all)?;
    let total = all.len();
    let entries: Vec<EntityRecord> = all
        .into_iter()
        .filter(|e| has_status(e, &status))
        .filter(|e| match tag_filter {
            Some(tag) => frontmatter::get_string_list(&e.frontmatter, "tags")
                .iter()
                .any(|t| t.eq_ignore_ascii_case(tag)),
            None => true,
        })
        .collect();

    if ui::get().json {
        return print_json(&entries, cfg);
    }

    let filters = filter_summary(&[("status", status.clone()), ("tag", tag_filter.clone())]);

    let table = match kind {
        EntityKind::Customer | EntityKind::Project => {
            let mut t = Table::new(vec![
                Col::new("ID").fixed(),
                Col::new("Name").flex(16),
                Col::new("Status"),
                Col::new("Owner").max(20).drop(2),
                Col::new(if kind == EntityKind::Project {
                    "Customers"
                } else {
                    "Tags"
                })
                .max(24)
                .drop(3),
            ]);
            for e in &entries {
                let extra = if kind == EntityKind::Project {
                    links(e, "customers")
                } else {
                    ui::clean(&frontmatter::get_string_list(&e.frontmatter, "tags").join(", "))
                        .into_owned()
                };
                t.row(vec![
                    id_cell(e),
                    s(e, "name").to_string(),
                    ui::status(raw(e, "status")),
                    s(e, "owner").dimmed().to_string(),
                    extra.dimmed().to_string(),
                ]);
            }
            t
        }
        EntityKind::Meeting => {
            let mut t = Table::new(vec![
                Col::new("ID").fixed(),
                Col::new("Date"),
                Col::new("Time").drop(2),
                Col::new("Title").flex(16),
                Col::new("Status"),
            ]);
            for e in &entries {
                t.row(vec![
                    id_cell(e),
                    s(e, "date").to_string(),
                    s(e, "time").dimmed().to_string(),
                    s(e, "title").to_string(),
                    ui::status(raw(e, "status")),
                ]);
            }
            t
        }
        EntityKind::Research => {
            let mut t = Table::new(vec![
                Col::new("ID").fixed(),
                Col::new("Title").flex(16),
                Col::new("Status"),
                Col::new("Owner").max(20).drop(2),
            ]);
            for e in &entries {
                t.row(vec![
                    id_cell(e),
                    s(e, "title").to_string(),
                    ui::status(raw(e, "status")),
                    s(e, "owner").dimmed().to_string(),
                ]);
            }
            t
        }
        EntityKind::Milestone => {
            let mut t = Table::new(vec![
                Col::new("ID").fixed(),
                Col::new("Title").flex(16),
                Col::new("Status"),
                Col::new("Start"),
                Col::new("Deadline"),
            ]);
            for e in &entries {
                t.row(vec![
                    id_cell(e),
                    s(e, "title").to_string(),
                    ui::status(raw(e, "status")),
                    s(e, "start_date").to_string(),
                    s(e, "due_date").to_string(),
                ]);
            }
            t
        }
        EntityKind::Sprint => {
            let mut t = Table::new(vec![
                Col::new("ID").fixed(),
                Col::new("Title").flex(14),
                Col::new("Status"),
                Col::new("Start").drop(3),
                Col::new("End").drop(3),
                Col::new("Owner").max(20).drop(2),
            ]);
            for e in &entries {
                t.row(vec![
                    id_cell(e),
                    s(e, "title").to_string(),
                    ui::status(raw(e, "status")),
                    s(e, "start_date").to_string(),
                    s(e, "end_date").to_string(),
                    s(e, "owner").dimmed().to_string(),
                ]);
            }
            t
        }
        EntityKind::Proposal => {
            let mut t = Table::new(vec![
                Col::new("ID").fixed(),
                Col::new("Title").flex(16),
                Col::new("Status"),
                Col::new("Type").drop(3),
                Col::new("Author").max(20).drop(2),
            ]);
            for e in &entries {
                t.row(vec![
                    id_cell(e),
                    s(e, "title").to_string(),
                    ui::status(raw(e, "status")),
                    s(e, "type").to_string(),
                    s(e, "author").dimmed().to_string(),
                ]);
            }
            t
        }
        EntityKind::Task => unreachable!("Tasks use list_tasks(), not list_standard()"),
        EntityKind::Contact => unreachable!("Contacts use list_contacts(), not list_standard()"),
    };

    if !table.is_empty() {
        table.print();
    }
    print_footer(kind, entries.len(), total, &filters);
    Ok(())
}

/// Whether a task is still open (not done/cancelled).
fn is_open(e: &EntityRecord) -> bool {
    !matches!(raw(e, "status"), "done" | "cancelled")
}

/// Whether an open task's due date has passed.
fn is_overdue(e: &EntityRecord, today: &str) -> bool {
    let due = raw(e, "due_date");
    is_open(e) && !due.is_empty() && due < today
}

/// Order tasks (already sorted by ID) by `key`; ties keep ID order.
fn sort_tasks(tasks: &mut [EntityRecord], key: TaskSort) {
    let pri = |e: &EntityRecord| data::get_number(&e.frontmatter, "priority").unwrap_or(3);
    // Tasks without a due date sort after those with one.
    let due = |e: &EntityRecord| {
        let d = raw(e, "due_date");
        (d.is_empty(), d.to_string())
    };
    match key {
        TaskSort::Id => {}
        TaskSort::Priority => tasks.sort_by(|a, b| pri(a).cmp(&pri(b)).then(due(a).cmp(&due(b)))),
        TaskSort::Due => tasks.sort_by(|a, b| due(a).cmp(&due(b)).then(pri(a).cmp(&pri(b)))),
        TaskSort::Updated => tasks.sort_by(|a, b| raw(b, "updated").cmp(raw(a, "updated"))),
    }
}

/// Due date colored by urgency: overdue red, within a week yellow.
pub(crate) fn due_cell(e: &EntityRecord, today: &str) -> String {
    let due = &*s(e, "due_date");
    if due.is_empty() {
        return String::new();
    }
    if !is_open(e) {
        return due.dimmed().to_string();
    }
    if due < today {
        return due.red().bold().to_string();
    }
    let soon = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d")
        .ok()
        .map(|d| {
            (d + chrono::Duration::days(7))
                .format("%Y-%m-%d")
                .to_string()
        });
    match soon {
        Some(limit) if due <= limit.as_str() => due.yellow().to_string(),
        _ => due.to_string(),
    }
}

fn list_tasks(cfg: &ResolvedConfig, args: &ListEntity) -> McResult<()> {
    let ListEntity::Tasks {
        status,
        tag,
        project,
        customer,
        priority,
        sprint,
        milestone,
        owner,
        open,
        overdue,
        sort,
    } = args
    else {
        unreachable!("list_tasks() is only called for `mc list tasks`");
    };
    let all = data::collect_tasks(cfg)?;
    let status = resolve_status_filter(status.as_deref(), EntityKind::Task, cfg, &all)?;
    let total = all.len();
    let today = crate::util::today_str();

    let milestone = milestone
        .as_deref()
        .map(|s| crate::commands::new::resolve_milestone(cfg, s))
        .transpose()?;
    let filter = TaskFilter {
        status: None,
        tag: tag.as_deref(),
        project: project.as_deref(),
        customer: customer.as_deref(),
        priority: *priority,
        sprint: sprint.as_deref(),
        owner: owner.as_deref(),

        milestone: milestone.as_deref(),
    };
    // A sprint ID or title matches tasks that store either.
    let sprints = sprint.as_deref().map(|s| data::sprint_aliases(cfg, s));
    let mut entries: Vec<EntityRecord> = all
        .into_iter()
        .filter(|e| {
            filter.matches_with_sprints(&e.frontmatter, sprints.as_deref())
                && has_status(e, &status)
        })
        .filter(|e| !*open || is_open(e))
        .filter(|e| !*overdue || is_overdue(e, &today))
        .collect();
    sort_tasks(&mut entries, *sort);

    if ui::get().json {
        return print_json(&entries, cfg);
    }

    let mut filters = filter_summary(&[
        ("status", status.clone()),
        ("project", project.clone()),
        ("customer", customer.clone()),
        ("priority", priority.map(|p| p.to_string())),
        ("sprint", sprint.clone()),
        (
            "milestone",
            milestone
                .clone()
                .map(|m| if m.is_empty() { "none".into() } else { m }),
        ),
        ("owner", owner.clone()),
        ("tag", tag.clone()),
    ]);
    filters.extend(
        [("open", *open), ("overdue", *overdue)]
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(flag, _)| flag.to_string()),
    );
    if *sort != TaskSort::Id {
        filters.push(format!("sort={}", format!("{sort:?}").to_lowercase()));
    }

    let mut table = Table::new(vec![
        Col::new("ID").fixed(),
        Col::new("Pri"),
        Col::new("Status"),
        Col::new("Title").flex(18),
        Col::new("Owner").max(16).drop(3),
        Col::new("Project").max(12).drop(4),
        Col::new("Due").drop(2),
        Col::new("Sprint").max(14).drop(5),
    ]);
    for e in &entries {
        let pri = data::get_number(&e.frontmatter, "priority").unwrap_or(3);
        let sprint = frontmatter::strip_wikilink(&s(e, "sprint")).to_string();
        let project = frontmatter::get_link_list(&e.frontmatter, "projects")
            .first()
            .map(|p| ui::clean(p).into_owned())
            .unwrap_or_default();
        let title = if is_open(e) {
            s(e, "title").to_string()
        } else {
            s(e, "title").dimmed().to_string()
        };
        table.row(vec![
            id_cell(e),
            ui::priority(pri),
            ui::status(raw(e, "status")),
            title,
            s(e, "owner").dimmed().to_string(),
            project,
            due_cell(e, &today),
            sprint.dimmed().to_string(),
        ]);
    }

    if !table.is_empty() {
        table.print();
    }
    print_footer(EntityKind::Task, entries.len(), total, &filters);
    Ok(())
}

fn list_contacts(
    cfg: &ResolvedConfig,
    status: &Option<String>,
    tag: &Option<String>,
    customer: &Option<String>,
) -> McResult<()> {
    if !cfg.entity_available(&EntityKind::Contact) {
        return Err(McError::not_available(EntityKind::Contact, cfg));
    }

    let all = data::collect_contacts(cfg)?;
    let status = resolve_status_filter(status.as_deref(), EntityKind::Contact, cfg, &all)?;
    let total = all.len();
    drop(all);

    let filter = ContactFilter {
        status: None,
        tag: tag.as_deref(),
        customer: customer.as_deref(),
    };
    let entries: Vec<EntityRecord> = data::collect_contacts_filtered(cfg, &filter)?
        .into_iter()
        .filter(|e| has_status(e, &status))
        .collect();

    if ui::get().json {
        return print_json(&entries, cfg);
    }

    let filters = filter_summary(&[
        ("status", status.clone()),
        ("customer", customer.clone()),
        ("tag", tag.clone()),
    ]);

    let mut table = Table::new(vec![
        Col::new("ID").fixed(),
        Col::new("Name").flex(14),
        Col::new("Role").max(26).drop(3),
        Col::new("Customer").drop(2),
        Col::new("Email").max(30).drop(4),
        Col::new("Status"),
    ]);
    for e in &entries {
        table.row(vec![
            id_cell(e),
            s(e, "name").to_string(),
            s(e, "role").dimmed().to_string(),
            frontmatter::strip_wikilink(&s(e, "customer")).to_string(),
            s(e, "email").dimmed().to_string(),
            ui::status(raw(e, "status")),
        ]);
    }

    if !table.is_empty() {
        table.print();
    }
    print_footer(EntityKind::Contact, entries.len(), total, &filters);
    Ok(())
}
