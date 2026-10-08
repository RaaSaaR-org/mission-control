//! Task board (kanban) and task list.

use super::lists::refs_clip;
use crate::data::{self, EntityRecord};
use crate::entity::EntityKind;
use crate::frontmatter;
use crate::html::catalog::display_name;
use crate::html::components::{
    avatar, clip, data_table, due_html, empty_state, entity_name_link, filter_cell, icon_button,
    id_chip, lamp, link_button, owner_html, page_header, priority_class, priority_compact,
    priority_html, priority_label, row_filter, status_badge, status_lamp, table_count,
    tag_chips_compact, task_due_html, th, view_toggle, Btn, Sort,
};
use crate::html::edit::{lane_add_button, move_button, move_menu_template, new_task_button};
use crate::html::format::{
    entity_href, escape_html, href_with, parse_date, status_label, status_slug,
};
use crate::html::layout::layout;
use crate::html::{is_cancelled, is_closed, Page};
use chrono::NaiveDate;
use std::collections::{BTreeSet, HashMap};

/// Cards shown in full in the done lane before the rest collapse.
const DONE_VISIBLE: usize = 10;

/// Sort key that puts open work first (in progress, review, to do,
/// backlog), finished work last, then by priority, due date and ID.
pub(crate) fn task_order(t: &EntityRecord) -> (u8, u32, String, String) {
    let fm = &t.frontmatter;
    let rank = match frontmatter::get_str_or(fm, "status", "") {
        "in-progress" => 0,
        "review" => 1,
        "todo" => 2,
        "backlog" => 3,
        "done" | "completed" => 5,
        "cancelled" | "canceled" => 6,
        _ => 4,
    };
    let due = parse_date(frontmatter::get_str_or(fm, "due_date", "")).map_or_else(
        || "9999-99-99".to_string(),
        |d| d.format("%Y-%m-%d").to_string(),
    );
    (
        rank,
        data::get_number(fm, "priority").unwrap_or(3),
        due,
        t.id.clone(),
    )
}

/// Filter options derived from all tasks for populating dropdowns.
pub struct TaskFilterOptions {
    pub owners: Vec<String>,
    pub projects: Vec<String>,
    pub sprints: Vec<String>,
    pub milestones: Vec<String>,
}

impl TaskFilterOptions {
    pub fn from_tasks(tasks: &[EntityRecord]) -> Self {
        Self::from_records(tasks)
    }

    /// Options from any collection of task records.
    pub fn from_records<'a>(tasks: impl IntoIterator<Item = &'a EntityRecord>) -> Self {
        let mut owners = BTreeSet::new();
        let mut projects = BTreeSet::new();
        let mut sprints = BTreeSet::new();
        let mut milestones = BTreeSet::new();
        for t in tasks {
            let owner = frontmatter::get_str_or(&t.frontmatter, "owner", "");
            if !owner.is_empty() {
                owners.insert(owner.to_string());
            }
            for p in frontmatter::get_link_list(&t.frontmatter, "projects") {
                if !p.is_empty() {
                    projects.insert(p);
                }
            }
            if let Some(m) =
                frontmatter::get_link_str(&t.frontmatter, "milestone").filter(|s| !s.is_empty())
            {
                milestones.insert(m.to_string());
            }
            let sprint =
                frontmatter::strip_wikilink(frontmatter::get_str_or(&t.frontmatter, "sprint", ""));
            if !sprint.is_empty() {
                sprints.insert(sprint.to_string());
            }
        }
        Self {
            owners: owners.into_iter().collect(),
            projects: projects.into_iter().collect(),
            sprints: sprints.into_iter().collect(),
            milestones: milestones.into_iter().collect(),
        }
    }
}

/// Active task filters and sort, shared by the board and list views.
#[derive(Default)]
pub struct TaskQuery<'a> {
    pub status: Option<&'a str>,
    pub priority: Option<u32>,
    pub owner: Option<&'a str>,
    pub project: Option<&'a str>,
    pub customer: Option<&'a str>,
    pub sprint: Option<&'a str>,
    pub milestone: Option<&'a str>,
    /// Sort field for the list view (`id`, `name`, `priority`, `due_date`).
    pub sort: Option<&'a str>,
    /// `asc` or `desc`.
    pub dir: &'a str,
}

impl TaskQuery<'_> {
    fn is_empty(&self) -> bool {
        self.status.is_none()
            && self.priority.is_none()
            && self.owner.is_none()
            && self.project.is_none()
            && self.customer.is_none()
            && self.sprint.is_none()
            && self.milestone.is_none()
    }

    /// Link to `path` with these filters plus extra pairs.
    fn href(&self, path: &str, extra: &[(&str, &str)]) -> String {
        let pri = self.priority.map(|p| p.to_string()).unwrap_or_default();
        let mut pairs = vec![
            ("status", self.status.unwrap_or("")),
            ("priority", pri.as_str()),
            ("owner", self.owner.unwrap_or("")),
            ("project", self.project.unwrap_or("")),
            ("customer", self.customer.unwrap_or("")),
            ("sprint", self.sprint.unwrap_or("")),
            ("milestone", self.milestone.unwrap_or("")),
        ];
        pairs.extend_from_slice(extra);
        href_with(path, &pairs)
    }
}

/// Filter strip with task dropdowns. `with_status` adds status and priority
/// (list view); `filter_target` is the id the live text filter applies to.
fn task_toolbar(
    page: &Page,
    action: &str,
    query: &TaskQuery,
    options: &TaskFilterOptions,
    with_status: bool,
    filter_target: &str,
) -> String {
    let catalog = page.catalog;
    let named = |ids: &[String]| -> Vec<(String, String)> {
        ids.iter()
            .map(|id| (id.clone(), catalog.name(id).unwrap_or(id).to_string()))
            .collect()
    };
    let mut cells = String::new();
    if with_status {
        let statuses: Vec<(String, String)> = EntityKind::Task
            .statuses(page.cfg)
            .iter()
            .map(|s| (s.clone(), status_label(s)))
            .collect();
        cells.push_str(&filter_cell("status", "Status", &statuses, query.status));
        let priorities: Vec<(String, String)> = (1..=4)
            .map(|p| (p.to_string(), priority_label(p).to_string()))
            .collect();
        let pri = query.priority.map(|p| p.to_string());
        cells.push_str(&filter_cell(
            "priority",
            "Priority",
            &priorities,
            pri.as_deref(),
        ));
    }
    if !options.projects.is_empty() {
        cells.push_str(&filter_cell(
            "project",
            "Project",
            &named(&options.projects),
            query.project,
        ));
    }
    if !options.owners.is_empty() {
        let owners: Vec<(String, String)> = options
            .owners
            .iter()
            .map(|o| (o.clone(), o.clone()))
            .collect();
        cells.push_str(&filter_cell("owner", "Owner", &owners, query.owner));
    }
    // Every milestone, not only those tasks already use: the timeline links
    // to `?milestone=` for milestones without tasks, and the active filter
    // must stay visible (and survive the next auto-submit) there too.
    let milestones: BTreeSet<String> = catalog
        .of_kind(EntityKind::Milestone)
        .map(|m| m.id.clone())
        .chain(options.milestones.iter().cloned())
        .collect();
    if !milestones.is_empty() {
        let milestones: Vec<String> = milestones.into_iter().collect();
        cells.push_str(&filter_cell(
            "milestone",
            "Milestone",
            &named(&milestones),
            query.milestone,
        ));
    }
    if !options.sprints.is_empty() {
        cells.push_str(&filter_cell(
            "sprint",
            "Sprint",
            &named(&options.sprints),
            query.sprint,
        ));
    }

    // autocomplete=off: going Back must not restore stale filter values over
    // the page the server rendered.
    let mut html = format!(
        r#"<form class="toolbar filter-form" method="get" action="{action}" autocomplete="off">"#
    );
    if !cells.is_empty() {
        html.push_str(&format!(r#"<div class="filter-group">{cells}</div>"#));
    }
    if let Some(c) = query.customer {
        html.push_str(&format!(
            r#"<input type="hidden" name="customer" value="{}"><span class="chip chip-active">Customer: {}</span>"#,
            escape_html(c),
            catalog.ref_html(c)
        ));
    }
    if let Some(s) = query.sort {
        html.push_str(&format!(
            r#"<input type="hidden" name="sort" value="{}"><input type="hidden" name="dir" value="{}">"#,
            escape_html(s),
            escape_html(query.dir)
        ));
    }
    html.push_str(
        r#"<noscript><button type="submit" class="btn btn-sm">Apply</button></noscript>"#,
    );
    if !query.is_empty() {
        html.push_str(&link_button(
            action,
            "Clear filters",
            Btn::Ghost,
            "btn-sm reset-link",
        ));
    }
    html.push_str(&row_filter(
        filter_target,
        if filter_target == "board" {
            "Filter cards"
        } else {
            "Filter tasks"
        },
    ));
    html.push_str("</form>");
    html
}

fn total_tasks(page: &Page) -> usize {
    page.catalog
        .count_for(EntityKind::Task)
        .map_or(0, |c| c.total)
}

/// Render the tasks list page.
pub fn tasks_list_page(
    page: &Page,
    tasks: &[EntityRecord],
    query: &TaskQuery,
    options: &TaskFilterOptions,
) -> String {
    let total = total_tasks(page);
    let meta = if query.is_empty() {
        format!("{total} total")
    } else {
        format!("{} of {total} shown", tasks.len())
    };
    let density = icon_button(
        "rows",
        "Compact rows",
        "density-toggle",
        r#" aria-pressed="false" hidden"#,
    );
    let mut body = page_header(
        "Tasks",
        &meta,
        &format!(
            "{}{density}{}",
            new_task_action(page),
            filtered_view_toggle(query, false)
        ),
    );

    if total == 0 {
        body.push_str(&no_tasks_yet(page));
        return layout(page, "Tasks", "/tasks", "", &body);
    }
    body.push_str(&task_toolbar(
        page,
        "/tasks/list",
        query,
        options,
        true,
        "rows",
    ));

    if tasks.is_empty() {
        body.push_str(&empty_state(
            "No tasks match these filters",
            "Clear the filters to see every task again.",
            &link_button("/tasks/list", "Clear filters", Btn::Secondary, ""),
        ));
        return layout(page, "Tasks", "/tasks", "", &body);
    }

    let sort = Sort {
        field: query.sort,
        dir: query.dir,
    };
    let sort_href =
        |field: &str, dir: &str| query.href("/tasks/list", &[("sort", field), ("dir", dir)]);
    let sorted = |label: &str, class: &str, field: &str| {
        th(
            label,
            class,
            Some((field, &sort, &sort_href as &dyn Fn(&str, &str) -> String)),
        )
    };
    let head = [
        sorted("ID", "col-id", "id"),
        sorted("Title", "col-name", "name"),
        th("Status", "col-status", None),
        sorted("Priority", "col-priority", "priority"),
        th("Owner", "col-owner", None),
        th("Project", "col-project", None),
        th("Milestone", "col-milestone", None),
        th("Sprint", "col-sprint", None),
        sorted("Due", "col-date", "due_date"),
    ]
    .concat();

    // Without a chosen sort, open work comes first.
    let mut ordered: Vec<&EntityRecord> = tasks.iter().collect();
    if query.sort.is_none() {
        ordered.sort_by_cached_key(|t| task_order(t));
    }
    let rows: String = ordered
        .into_iter()
        .map(|e| {
            let fm = &e.frontmatter;
            let status = frontmatter::get_str_or(fm, "status", "");
            let priority = data::get_number(fm, "priority").unwrap_or(3);
            let milestone = frontmatter::get_link_str(fm,"milestone").filter(|s| !s.is_empty()).map(|m| page.catalog.ref_html(m)).unwrap_or_default();
            let sprint = frontmatter::get_str_or(fm, "sprint", "");
            let sprint = if sprint.trim().is_empty() {
                String::new()
            } else {
                refs_clip(page, &[sprint.to_string()])
            };
            format!(
                r#"<tr data-row data-id="{id}" data-status="{st}"><td class="col-id">{}</td><td class="col-name">{}{}</td><td class="col-status">{}</td><td class="col-priority">{}</td><td class="col-owner">{}</td><td class="col-project">{}</td><td class="col-milestone">{milestone}</td><td class="col-sprint">{sprint}</td><td class="col-date">{}</td></tr>"#,
                id_chip(&e.id),
                entity_name_link(e),
                tag_chips_compact(&frontmatter::get_string_list(fm, "tags"), None, 2),
                status_badge(status),
                priority_html(priority),
                owner_html(frontmatter::get_str_or(fm, "owner", "")),
                refs_clip(page, &frontmatter::get_string_list(fm, "projects")),
                task_due_html(frontmatter::get_str_or(fm, "due_date", ""), status, page.today),
                id = escape_html(&e.id),
                st = escape_html(status),
            )
        })
        .collect();
    body.push_str(&data_table("task-table", &head, &rows));
    body.push_str(&table_count(tasks.len(), total));

    layout(page, "Tasks", "/tasks", "", &body)
}

/// The board/list switch, keeping the active filters. `query.href` is
/// already attribute-safe (percent-encoded values joined with `&amp;`).
fn filtered_view_toggle(query: &TaskQuery, board: bool) -> String {
    view_toggle(board)
        .replace(
            "href=\"/tasks\"",
            &format!("href=\"{}\"", query.href("/tasks", &[])),
        )
        .replace(
            "href=\"/tasks/list\"",
            &format!("href=\"{}\"", query.href("/tasks/list", &[])),
        )
}

fn new_task_action(page: &Page) -> String {
    if page.editable {
        new_task_button(true)
    } else {
        String::new()
    }
}

fn no_tasks_yet(page: &Page) -> String {
    let hint = if page.editable {
        r#"Press <kbd>c</kbd> or use New task, or run <code>mc new task "Title"</code>."#
    } else {
        r#"Create one with <code>mc new task "Title"</code>."#
    };
    empty_state("No tasks yet.", hint, "")
}

/// Render a kanban board page for tasks.
pub fn board_page(
    page: &Page,
    tasks: &[EntityRecord],
    query: &TaskQuery,
    options: &TaskFilterOptions,
) -> String {
    // Lanes follow the configured task statuses; cancelled tasks stay off the board.
    let mut lanes: Vec<String> = EntityKind::Task
        .statuses(page.cfg)
        .iter()
        .filter(|s| !is_cancelled(s))
        .cloned()
        .collect();
    let mut grouped: HashMap<String, Vec<&EntityRecord>> = HashMap::new();
    for task in tasks {
        let status = frontmatter::get_str_or(&task.frontmatter, "status", "backlog");
        if is_cancelled(status) {
            continue;
        }
        if !lanes.iter().any(|c| c == status) {
            lanes.push(status.to_string());
        }
        grouped.entry(status.to_string()).or_default().push(task);
    }
    for lane in grouped.values_mut() {
        lane.sort_by_key(|t| {
            (
                data::get_number(&t.frontmatter, "priority").unwrap_or(3),
                parse_date(frontmatter::get_str_or(&t.frontmatter, "due_date", ""))
                    .unwrap_or(NaiveDate::MAX),
                t.id.clone(),
            )
        });
    }

    let shown: usize = grouped.values().map(Vec::len).sum();
    let mut body = page_header(
        "Tasks",
        &format!("{shown} on the board, cancelled hidden"),
        &format!(
            "{}{}",
            new_task_action(page),
            filtered_view_toggle(query, true)
        ),
    );
    if total_tasks(page) == 0 {
        body.push_str(&no_tasks_yet(page));
        return layout(page, "Task board", "/tasks", "", &body);
    }
    body.push_str(&task_toolbar(
        page, "/tasks", query, options, false, "board",
    ));

    body.push_str(&format!(
        r#"<div class="kanban-board" id="board" style="--lanes:{}">"#,
        lanes.len()
    ));
    for lane in &lanes {
        let empty = Vec::new();
        let items = grouped.get(lane).unwrap_or(&empty);
        body.push_str(&kanban_lane(page, lane, items));
    }
    body.push_str("</div>");
    if page.editable {
        body.push_str(&move_menu_template(page));
    }

    layout(page, "Task board", "/tasks", "", &body)
}

fn kanban_lane(page: &Page, status: &str, items: &[&EntityRecord]) -> String {
    let slug = status_slug(status);
    let label = status_label(status);
    let is_done = is_done_lane(status);
    let late = if is_done {
        0
    } else {
        items
            .iter()
            .filter(|t| due_state(t, page.today) == Some("overdue"))
            .count()
    };
    let late_html = if late > 0 {
        format!(r#"<span class="kanban-late">{late} late</span>"#)
    } else {
        String::new()
    };
    let add = if page.editable {
        lane_add_button(status)
    } else {
        String::new()
    };
    let mut html = format!(
        r#"<section class="kanban-column col-{slug}" data-status="{st}" aria-labelledby="lane-{slug}"><h2 class="kanban-head" id="lane-{slug}">{}<span class="kanban-name">{}</span><span class="kanban-count readout" data-lane-count>{}</span>{late_html}{add}</h2><div class="kanban-cards" data-status="{st}">"#,
        status_lamp(status),
        escape_html(&label),
        items.len(),
        st = escape_html(status),
    );
    if items.is_empty() {
        html.push_str(&format!(
            r#"<div class="kanban-empty">Nothing in {}.</div>"#,
            escape_html(&label.to_lowercase())
        ));
    }
    for (i, task) in items.iter().enumerate() {
        if is_done && i == DONE_VISIBLE {
            html.push_str(&format!(
                r#"<details class="kanban-more"><summary>Show {} more</summary><div class="kanban-more-cards">"#,
                items.len() - DONE_VISIBLE
            ));
        }
        html.push_str(&board_item(task, page, is_done));
    }
    if is_done && items.len() > DONE_VISIBLE {
        html.push_str("</div></details>");
    }
    html.push_str("</div></section>");
    html
}

/// `overdue` or `soon` (0–7 days) for an open task with a due date.
pub(crate) fn due_state(task: &EntityRecord, today: NaiveDate) -> Option<&'static str> {
    if is_closed(frontmatter::get_str_or(&task.frontmatter, "status", "")) {
        return None;
    }
    let due = parse_date(frontmatter::get_str_or(&task.frontmatter, "due_date", ""))?;
    match (due - today).num_days() {
        d if d < 0 => Some("overdue"),
        0..=7 => Some("soon"),
        _ => None,
    }
}

/// Lanes whose cards render as compact one-line rows.
fn is_done_lane(status: &str) -> bool {
    matches!(status, "done" | "completed")
}

/// One board item: the card link plus, when editable, its move button. The
/// wrapper carries the hooks the script uses to filter, sort and move cards.
fn board_item(task: &EntityRecord, page: &Page, compact: bool) -> String {
    let fm = &task.frontmatter;
    let card = if compact {
        compact_card(task)
    } else {
        kanban_card(task, page)
    };
    let mv = if page.editable {
        move_button(&task.id)
    } else {
        String::new()
    };
    format!(
        r#"<div class="kanban-item" data-row data-id="{}" data-status="{}" data-pri="{}" data-due="{}">{card}{mv}</div>"#,
        escape_html(&task.id),
        escape_html(frontmatter::get_str_or(fm, "status", "")),
        data::get_number(fm, "priority").unwrap_or(3),
        escape_html(frontmatter::get_str_or(fm, "due_date", "").trim()),
    )
}

/// A task's board item as it would render in its own status lane.
pub fn task_card(page: &Page, task: &EntityRecord) -> String {
    let status = frontmatter::get_str_or(&task.frontmatter, "status", "backlog");
    board_item(task, page, is_done_lane(status))
}

fn kanban_card(task: &EntityRecord, page: &Page) -> String {
    let fm = &task.frontmatter;
    let priority = data::get_number(fm, "priority").unwrap_or(3);
    let owner = frontmatter::get_str_or(fm, "owner", "");
    let due_raw = frontmatter::get_str_or(fm, "due_date", "");
    let state = match due_state(task, page.today) {
        Some(s) => format!(" is-{s}"),
        None => String::new(),
    };
    let project = frontmatter::get_link_list(fm, "projects")
        .first()
        .map(|p| {
            let name = page.catalog.name(p).unwrap_or(p);
            format!(
                r#"<span class="kanban-card-project" title="{0}">{0}</span>"#,
                escape_html(name)
            )
        })
        .unwrap_or_default();
    let due = if due_raw.trim().is_empty() {
        String::new()
    } else {
        due_html(due_raw, page.today)
    };
    let footer = if due.is_empty() && owner.trim().is_empty() {
        String::new()
    } else {
        format!("{due}{}", avatar(owner))
    };
    format!(
        r#"<a class="kanban-card pri-{pri}{state}" href="{href}"><span class="kanban-card-title">{title}</span><span class="kanban-card-meta">{id}{pri_html}{project}</span><span class="kanban-card-footer">{footer}</span></a>"#,
        pri = priority_class(priority),
        href = entity_href(&task.id),
        title = escape_html(display_name(task)),
        id = id_chip(&task.id),
        pri_html = priority_compact(priority),
    )
}

/// One-line card for finished work.
fn compact_card(task: &EntityRecord) -> String {
    let num = task.id.rsplit('-').next().unwrap_or(&task.id);
    let name = display_name(task);
    format!(
        r#"<a class="kanban-card is-compact" href="{}" title="{}">{}{}<span class="kanban-card-num readout">{}</span></a>"#,
        entity_href(&task.id),
        escape_html(&format!("{} {}", task.id, name)),
        lamp("positive", " lamp-solid"),
        clip(
            &format!(
                r#"<span class="kanban-card-title">{}</span>"#,
                escape_html(name)
            ),
            name
        ),
        escape_html(num),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    fn tasks() -> Vec<EntityRecord> {
        let today = chrono::Local::now().date_naive();
        let late = (today - chrono::Duration::days(2)).format("%Y-%m-%d");
        let mut v = vec![
            rec(
                EntityKind::Task,
                "TASK-001",
                &format!("title: \"Fix <bug>\"\nstatus: todo\npriority: 1\nowner: Jane Doe\ndue_date: {late}"),
            ),
            rec(EntityKind::Task, "TASK-002", "title: Plan\nstatus: in-progress"),
        ];
        for i in 0..12 {
            v.push(rec(
                EntityKind::Task,
                &format!("TASK-1{i:02}"),
                "title: Old\nstatus: done",
            ));
        }
        v
    }

    #[test]
    fn board_has_hooks_states_and_compact_done_lane() {
        let (_d, cfg) = test_config();
        let all = tasks();
        let cat = catalog(tasks(), &cfg);
        let page = Page::new(&cfg, &cat, "");
        let html = board_page(
            &page,
            &all,
            &TaskQuery::default(),
            &TaskFilterOptions::from_tasks(&all),
        );
        assert!(html.contains(r#"data-id="TASK-001" data-status="todo""#));
        assert!(html.contains(r#"class="kanban-column col-todo" data-status="todo""#));
        assert!(html.contains("kanban-card pri-critical is-overdue"));
        assert!(html.contains("Fix &lt;bug&gt;"));
        assert!(html.contains(r#"<span class="kanban-late">1 late</span>"#));
        assert!(html.contains("Show 2 more"));
        assert_eq!(html.matches("kanban-card is-compact").count(), 12);
        assert!(html.contains("Nothing in review."));
        assert!(html.contains(r#"data-filter="board""#));
        assert!(html.contains("--lanes:5"));
    }

    #[test]
    fn task_list_sorts_and_keeps_filters() {
        let (_d, cfg) = test_config();
        let all = tasks();
        let cat = catalog(tasks(), &cfg);
        let page = Page::new(&cfg, &cat, "");
        let q = TaskQuery {
            owner: Some("Jane Doe"),
            sort: Some("due_date"),
            dir: "asc",
            ..Default::default()
        };
        let html = tasks_list_page(&page, &all[..1], &q, &TaskFilterOptions::from_tasks(&all));
        assert!(html.contains(r#"aria-sort="ascending""#));
        assert!(html.contains("/tasks/list?owner=Jane%20Doe&amp;sort=due_date&amp;dir=desc"));
        assert!(html.contains("filter-cell is-set"));
        assert!(html.contains("Showing <span data-filter-count>1</span> of 14"));
        assert!(html.contains("reset-link"));
        // Going Back must not restore stale filter values.
        assert!(html.contains(
            r#"class="toolbar filter-form" method="get" action="/tasks/list" autocomplete="off""#
        ));
    }

    #[test]
    fn task_list_without_sort_puts_open_work_first() {
        let (_d, cfg) = test_config();
        let mut all = tasks();
        all.push(rec(
            EntityKind::Task,
            "TASK-003",
            "title: Later\nstatus: todo\npriority: 1\ndue_date: 2099-01-01",
        ));
        all.push(rec(
            EntityKind::Task,
            "TASK-004",
            "title: Gone\nstatus: cancelled",
        ));
        all.reverse();
        let cat = catalog(tasks(), &cfg);
        let page = Page::new(&cfg, &cat, "");
        let html = tasks_list_page(
            &page,
            &all,
            &TaskQuery::default(),
            &TaskFilterOptions::from_tasks(&all),
        );
        let pos = |id: &str| html.find(&format!(r#"data-id="{id}""#)).unwrap();
        // In progress, then to do by priority and due date, then done, then cancelled.
        assert!(pos("TASK-002") < pos("TASK-001"));
        assert!(pos("TASK-001") < pos("TASK-003"));
        assert!(pos("TASK-003") < pos("TASK-100"));
        assert!(pos("TASK-100") < pos("TASK-111"));
        assert!(pos("TASK-111") < pos("TASK-004"));
    }
}
