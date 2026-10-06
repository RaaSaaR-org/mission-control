//! Edit UI: the quick-create dialog, the task edit form and the board's move
//! menu. It is rendered only when the server accepts edits, and every control
//! starts `hidden` so pages without JS stay a clean read-only view.

use super::catalog::display_name;
use super::components::{icon, kbd, priority_label, status_lamp};
use super::format::{escape_html, parse_date, status_label};
use super::{is_closed, Page};
use crate::data::{self, EntityRecord};
use crate::entity::EntityKind;
use crate::frontmatter;
use std::collections::BTreeSet;

/// Lowest and highest task priority the UI and the write API accept.
pub const PRIORITIES: std::ops::RangeInclusive<u32> = 1..=4;

/// Choices for the task fields that reference other entities.
pub(crate) struct EditOptions {
    pub owners: Vec<String>,
    pub projects: Vec<(String, String)>,
    pub customers: Vec<(String, String)>,
    pub sprints: Vec<(String, String)>,
}

impl EditOptions {
    pub(crate) fn from_page(page: &Page) -> Self {
        let catalog = page.catalog;
        let owners: BTreeSet<String> = catalog
            .records
            .iter()
            .map(|r| frontmatter::get_str_or(&r.frontmatter, "owner", "").trim())
            .filter(|o| !o.is_empty())
            .map(String::from)
            .collect();
        let named = |kind: EntityKind| -> Vec<(String, String)> {
            if !page.cfg.entity_available(&kind) {
                return Vec::new();
            }
            // Open entities first, then by name.
            let mut recs: Vec<&EntityRecord> = catalog.of_kind(kind).collect();
            recs.sort_by_key(|r| {
                (
                    is_closed(frontmatter::get_str_or(&r.frontmatter, "status", "")),
                    display_name(r).to_lowercase(),
                )
            });
            recs.iter()
                .map(|r| (r.id.clone(), display_name(r).to_string()))
                .collect()
        };
        let mut sprints: Vec<&EntityRecord> = if page.cfg.entity_available(&EntityKind::Sprint) {
            catalog.of_kind(EntityKind::Sprint).collect()
        } else {
            Vec::new()
        };
        // Open sprints first, newest first.
        sprints.sort_by_key(|r| {
            (
                is_closed(frontmatter::get_str_or(&r.frontmatter, "status", "")),
                std::cmp::Reverse(r.id.clone()),
            )
        });
        Self {
            owners: owners.into_iter().collect(),
            projects: named(EntityKind::Project),
            customers: named(EntityKind::Customer),
            sprints: sprints
                .iter()
                .map(|r| (r.id.clone(), display_name(r).to_string()))
                .collect(),
        }
    }
}

fn options_html(options: &[(String, String)], selected: &str) -> String {
    options
        .iter()
        .map(|(value, label)| {
            let sel = if value == selected { " selected" } else { "" };
            format!(
                r#"<option value="{}"{sel}>{}</option>"#,
                escape_html(value),
                escape_html(label)
            )
        })
        .collect()
}

/// A `<select>`. `none` adds a leading empty choice with that label; a
/// `selected` value missing from `options` is kept as an extra choice.
fn select(name: &str, options: &[(String, String)], selected: &str, none: Option<&str>) -> String {
    let mut opts = String::new();
    if let Some(label) = none {
        opts.push_str(&format!(
            r#"<option value="">{}</option>"#,
            escape_html(label)
        ));
    }
    opts.push_str(&options_html(options, selected));
    if !selected.is_empty() && !options.iter().any(|(v, _)| v == selected) {
        opts.push_str(&format!(
            r#"<option value="{0}" selected>{0}</option>"#,
            escape_html(selected)
        ));
    }
    format!(r#"<select name="{name}">{opts}</select>"#)
}

fn field(label: &str, control_html: &str, class: &str) -> String {
    format!(
        r#"<label class="form-field{class}"><span class="form-label">{label}</span>{control_html}</label>"#
    )
}

fn status_options(page: &Page) -> Vec<(String, String)> {
    EntityKind::Task
        .statuses(page.cfg)
        .iter()
        .map(|s| (s.clone(), status_label(s)))
        .collect()
}

fn priority_options() -> Vec<(String, String)> {
    PRIORITIES
        .map(|p| (p.to_string(), priority_label(p).to_string()))
        .collect()
}

fn owner_input(value: &str) -> String {
    format!(
        r#"<input type="text" name="owner" value="{}" list="mc-owners" maxlength="80" autocomplete="off" placeholder="No owner">"#,
        escape_html(value)
    )
}

fn due_input(value: &str) -> String {
    // A date input can only hold a real date; leave it empty otherwise.
    let value = if parse_date(value).is_some() {
        value
    } else {
        ""
    };
    format!(
        r#"<input type="date" name="due_date" value="{}">"#,
        escape_html(value)
    )
}

/// Owner suggestions shared by every owner input on the page.
fn owners_datalist(opts: &EditOptions) -> String {
    let items: String = opts
        .owners
        .iter()
        .map(|o| format!(r#"<option value="{}">"#, escape_html(o)))
        .collect();
    format!(r#"<datalist id="mc-owners">{items}</datalist>"#)
}

/// The "New task" dialog plus shared datalists, emitted once by the layout.
pub(crate) fn new_task_dialog(page: &Page) -> String {
    let opts = EditOptions::from_page(page);
    let first_status = EntityKind::Task
        .statuses(page.cfg)
        .first()
        .cloned()
        .unwrap_or_default();
    let mut grid = String::new();
    grid.push_str(&field(
        "Status",
        &select("status", &status_options(page), &first_status, None),
        "",
    ));
    grid.push_str(&field(
        "Priority",
        &select("priority", &priority_options(), "3", None),
        "",
    ));
    grid.push_str(&field("Owner", &owner_input(""), ""));
    grid.push_str(&field("Due date", &due_input(""), ""));
    if page.cfg.entity_available(&EntityKind::Project) {
        grid.push_str(&field(
            "Project",
            &select("project", &opts.projects, "", Some("No project")),
            "",
        ));
    }
    if page.cfg.entity_available(&EntityKind::Customer) {
        grid.push_str(&field(
            "Customer",
            &select("customer", &opts.customers, "", Some("No customer")),
            "",
        ));
    }
    if page.cfg.entity_available(&EntityKind::Sprint) {
        grid.push_str(&field(
            "Sprint",
            &select("sprint", &opts.sprints, "", Some("No sprint")),
            "",
        ));
    }
    format!(
        r#"{datalist}<dialog class="sheet new-task" aria-labelledby="new-task-title">
  <div class="sheet-head"><h2 class="sheet-title" id="new-task-title">New task</h2><button type="button" class="icon-btn" data-close aria-label="Close" title="Close">{close}</button></div>
  <form class="sheet-body form" data-new-task-form>
    {title}
    <div class="form-grid">{grid}</div>
    <p class="form-error" role="alert" hidden></p>
    <div class="form-actions"><label class="form-check"><input type="checkbox" name="another"> Create another</label><button type="button" class="btn btn-ghost" data-close>Cancel</button><button type="submit" class="btn btn-primary">Create task {submit_kbd}</button></div>
  </form>
</dialog>"#,
        datalist = owners_datalist(&opts),
        close = icon("close"),
        title = field(
            "Title",
            r#"<input type="text" name="title" required maxlength="200" autocomplete="off" placeholder="What needs doing?">"#,
            " form-field-wide",
        ),
        submit_kbd = r#"<kbd data-mod-enter>⌘↵</kbd>"#,
    )
}

/// The inline edit form on a task's detail page.
pub(crate) fn task_edit_form(page: &Page, task: &EntityRecord) -> String {
    let fm = &task.frontmatter;
    let opts = EditOptions::from_page(page);
    let status = frontmatter::get_str_or(fm, "status", "");
    let priority = data::get_number(fm, "priority").unwrap_or(3).to_string();
    let sprint = frontmatter::strip_wikilink(frontmatter::get_str_or(fm, "sprint", "").trim());
    let mut grid = String::new();
    grid.push_str(&field(
        "Status",
        &select("status", &status_options(page), status, None),
        "",
    ));
    grid.push_str(&field(
        "Priority",
        &select("priority", &priority_options(), &priority, None),
        "",
    ));
    grid.push_str(&field(
        "Owner",
        &owner_input(frontmatter::get_str_or(fm, "owner", "").trim()),
        "",
    ));
    if page.cfg.entity_available(&EntityKind::Sprint) {
        grid.push_str(&field(
            "Sprint",
            &select("sprint", &opts.sprints, sprint, Some("No sprint")),
            "",
        ));
    }
    grid.push_str(&field(
        "Due date",
        &due_input(frontmatter::get_str_or(fm, "due_date", "").trim()),
        "",
    ));
    format!(
        r#"<form class="edit-panel" id="task-edit" data-edit-task="{id}" aria-labelledby="task-edit-title" hidden>
  <h2 class="section-title" id="task-edit-title">Edit task</h2>
  <div class="form-grid form-grid-edit">{grid}</div>
  <p class="form-error" role="alert" hidden></p>
  <div class="form-actions"><button type="button" class="btn btn-ghost" data-cancel>Cancel</button><button type="submit" class="btn btn-primary">Save changes {kbd}</button></div>
</form>"#,
        id = escape_html(&task.id),
        kbd = r#"<kbd data-mod-enter>⌘↵</kbd>"#,
    )
}

/// The "Edit" button that reveals [`task_edit_form`].
pub(crate) fn edit_button() -> String {
    format!(
        r#"<button type="button" class="btn btn-sm" data-edit-open aria-controls="task-edit" aria-expanded="false" hidden>{}Edit {}</button>"#,
        icon("edit"),
        kbd("e")
    )
}

/// "New task" button; `status` preselects a lane.
pub(crate) fn new_task_button(label_kbd: bool) -> String {
    let k = if label_kbd { kbd("c") } else { String::new() };
    format!(
        r#"<button type="button" class="btn btn-primary btn-sm" data-new-task hidden>{}New task {k}</button>"#,
        icon("plus")
    )
}

/// Small "+" in a board lane head that opens the dialog with that status.
pub(crate) fn lane_add_button(status: &str) -> String {
    let label = format!("New task in {}", status_label(status));
    format!(
        r#"<button type="button" class="icon-btn lane-add" data-new-task data-status="{}" aria-label="{1}" title="{1}" hidden>{2}</button>"#,
        escape_html(status),
        escape_html(&label),
        icon("plus")
    )
}

/// The per-card button that opens the move menu.
pub(crate) fn move_button(id: &str) -> String {
    format!(
        r#"<button type="button" class="card-move" aria-haspopup="menu" aria-expanded="false" aria-label="Move {0}" title="Move {0} (m)" hidden>{1}</button>"#,
        escape_html(id),
        icon("more")
    )
}

/// Template for the move menu: one item per configured task status.
pub(crate) fn move_menu_template(page: &Page) -> String {
    let items: String = EntityKind::Task
        .statuses(page.cfg)
        .iter()
        .map(|s| {
            format!(
                r#"<button type="button" role="menuitemradio" aria-checked="false" data-status="{}">{}<span>{}</span></button>"#,
                escape_html(s),
                status_lamp(s),
                escape_html(&status_label(s))
            )
        })
        .collect();
    format!(
        r#"<template id="move-menu-tpl"><div class="menu" role="menu" aria-label="Move to">{items}</div></template>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    #[test]
    fn dialog_lists_entities_and_escapes() {
        let (_d, cfg) = test_config();
        let cat = catalog(
            vec![
                rec(
                    EntityKind::Project,
                    "PROJ-001",
                    "name: \"<Apollo>\"\nstatus: active",
                ),
                rec(
                    EntityKind::Sprint,
                    "SPR-001",
                    "title: Old\nstatus: completed",
                ),
                rec(EntityKind::Sprint, "SPR-002", "title: Now\nstatus: active"),
                rec(EntityKind::Task, "TASK-001", "title: A\nowner: \"Jo <b>\""),
            ],
            &cfg,
        );
        let page = Page::new(&cfg, &cat, "").with_editable(true);
        let html = new_task_dialog(&page);
        assert!(html.contains(r#"<option value="PROJ-001">&lt;Apollo&gt;</option>"#));
        assert!(html.contains(r#"<option value="Jo &lt;b&gt;">"#));
        // Open sprints come first.
        assert!(html.find("SPR-002").unwrap() < html.find("SPR-001").unwrap());
        assert!(html.contains(r#"name="title" required"#));
    }

    #[test]
    fn edit_form_preselects_current_values() {
        let (_d, cfg) = test_config();
        let task = rec(
            EntityKind::Task,
            "TASK-007",
            "title: T\nstatus: review\npriority: 2\nowner: Ann\nsprint: \"[[SPR-009]]\"\ndue_date: 2026-10-09",
        );
        let cat = catalog(vec![], &cfg);
        let page = Page::new(&cfg, &cat, "").with_editable(true);
        let html = task_edit_form(&page, &task);
        assert!(html.contains(r#"<option value="review" selected>"#));
        assert!(html.contains(r#"<option value="2" selected>High</option>"#));
        assert!(html.contains(r#"value="Ann""#));
        // A sprint that isn't in the catalog stays selectable.
        assert!(html.contains(r#"<option value="SPR-009" selected>SPR-009</option>"#));
        assert!(html.contains(r#"value="2026-10-09""#));
    }
}
