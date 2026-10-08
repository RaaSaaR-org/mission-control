//! Entity detail page: title block, notes, related entities and a field rail.

use super::lists::parent_customer_html;
use super::tasks::task_order;
use crate::data::EntityRecord;
use crate::entity::EntityKind;
use crate::frontmatter;
use crate::html::catalog::display_name;
use crate::html::catalog::split_wikilink;
use crate::html::components::{
    avatar, date_html, due_html, entity_name_link, icon_button, id_chip, owner_html, priority_html,
    progress_bar, section_title, status_badge, status_badge_lg, status_lamp, tag_chips,
    task_due_html,
};
use crate::html::edit::{edit_button, new_task_button, task_edit_form};
use crate::html::format::{capitalize, due_phrase, escape_html, fmt_day, parse_date};
use crate::html::layout::layout;
use crate::html::markdown::strip_leading_h1;
use crate::html::notes::{self, Notes};
use crate::html::{is_cancelled, is_closed, Page};
use serde_yaml::Value;

/// Related entities shown before the rest collapse behind "Show more".
const RELATED_VISIBLE: usize = 8;

/// Order of related-entity sections on detail pages.
const RELATED_ORDER: &[EntityKind] = &[
    EntityKind::Task,
    EntityKind::Meeting,
    EntityKind::Project,
    EntityKind::Contact,
    EntityKind::Customer,
    EntityKind::Proposal,
    EntityKind::Research,
    EntityKind::Sprint,
    EntityKind::Milestone,
];

/// Scalar fields that may be promoted into the title block, in order.
const TITLE_FIELDS: &[&str] = &[
    "owner",
    "priority",
    "due_date",
    "date",
    "time",
    "duration",
    "start_date",
    "end_date",
    "target_date",
    "role",
    "email",
];
/// Title-block cells after ID and status.
const TITLE_FIELD_SLOTS: usize = 4;
const TITLE_CELLS_MAX: usize = 6;

/// Frontmatter keys shown first in the details rail, in this order.
const LEADING_FIELDS: &[&str] = &[
    "owner",
    "priority",
    "due_date",
    "date",
    "time",
    "duration",
    "start_date",
    "end_date",
    "target_date",
    "role",
    "email",
    "phone",
    "customer",
    "customers",
    "project",
    "projects",
    "sprint",
    "milestone",
    "depends_on",
    "attendees",
    "contacts",
];
/// Frontmatter keys shown last.
const TRAILING_FIELDS: &[&str] = &["tags", "created", "updated"];
/// Frontmatter keys never shown in the rail (rendered elsewhere or internal).
const HIDDEN_FIELDS: &[&str] = &[
    "id", "status", "name", "title", "slug", "aliases", "summary",
];

fn field_label(key: &str) -> String {
    capitalize(&key.replace('_', " "))
}

/// Frontmatter keys that list tasks this one waits for or holds up.
const DEPENDENCY_FIELDS: &[&str] = &["depends_on", "blocks", "blocked_by"];

/// Render a single frontmatter value for the rail. Returns `None` for empty values.
fn field_value_html(key: &str, value: &Value, page: &Page) -> Option<String> {
    if DEPENDENCY_FIELDS.contains(&key) {
        return dependency_list(value, page);
    }
    match value {
        Value::Null => None,
        Value::Bool(b) => Some(if *b { "Yes" } else { "No" }.to_string()),
        Value::Number(n) => {
            if key == "priority" {
                n.as_u64().map(|p| priority_html(p as u32))
            } else {
                Some(n.to_string())
            }
        }
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() || s == "[]" {
                return None;
            }
            if s.starts_with("[[") || page.catalog.canonical_id(s).is_some() {
                return Some(page.catalog.ref_html(s));
            }
            if key == "due_date" {
                return Some(due_html(s, page.today));
            }
            if parse_date(s).is_some() {
                return Some(date_html(s, page.today));
            }
            if s.starts_with("http://") || s.starts_with("https://") {
                let shown = s
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .trim_end_matches('/');
                return Some(format!(
                    r#"<a class="plain-link" href="{}" target="_blank" rel="noopener">{}</a>"#,
                    escape_html(s),
                    escape_html(shown)
                ));
            }
            if key == "email" || (s.contains('@') && !s.contains(' ') && s.contains('.')) {
                return Some(format!(
                    r#"<a class="plain-link" href="mailto:{0}">{0}</a>"#,
                    escape_html(s)
                ));
            }
            Some(escape_html(s))
        }
        Value::Sequence(seq) => {
            if key == "tags" {
                let tags: Vec<String> = seq
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                let html = tag_chips(&tags, None);
                return (!html.is_empty()).then_some(html);
            }
            let items: Vec<String> = seq
                .iter()
                .filter_map(|v| field_value_html(key, v, page))
                .collect();
            match items.len() {
                0 => None,
                1 => items.into_iter().next(),
                _ => Some(format!(
                    r#"<ul class="value-list">{}</ul>"#,
                    items
                        .iter()
                        .map(|i| format!("<li>{i}</li>"))
                        .collect::<String>()
                )),
            }
        }
        Value::Mapping(map) => {
            // Inline objects such as `contacts: [{name, role, email}]`.
            let get = |k: &str| {
                map.get(Value::String(k.into()))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
            };
            let name = get("name").or_else(|| get("title"))?;
            let extra = get("role")
                .map(|r| format!(r#" <span class="muted">{}</span>"#, escape_html(r)))
                .unwrap_or_default();
            Some(format!("{}{extra}", escape_html(name)))
        }
        Value::Tagged(t) => field_value_html(key, &t.value, page),
    }
}

/// Linked tasks one per row, with their status and ID, so a finished
/// dependency reads as done.
fn dependency_list(value: &Value, page: &Page) -> Option<String> {
    let items: Vec<&str> = match value {
        Value::String(s) => vec![s.as_str()],
        Value::Sequence(seq) => seq.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let rows: String = items
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != "[]")
        .map(|raw| {
            let (target, alias) = split_wikilink(raw);
            let found = page
                .catalog
                .resolve(target, alias)
                .id()
                .and_then(|id| page.catalog.records.iter().find(|r| r.id == id));
            match found {
                Some(rec) => {
                    let status = frontmatter::get_str_or(&rec.frontmatter, "status", "");
                    format!(
                        r#"<li class="dep{}" title="{}">{}<span class="dep-name">{}</span>{}</li>"#,
                        if is_closed(status) { " is-done" } else { "" },
                        escape_html(&crate::html::format::status_label(status)),
                        status_lamp(status),
                        page.catalog.ref_html(raw),
                        id_chip(&rec.id),
                    )
                }
                None => format!(
                    r#"<li class="dep"><span class="dep-name">{}</span></li>"#,
                    page.catalog.ref_html(raw)
                ),
            }
        })
        .collect();
    (!rows.is_empty()).then(|| format!(r#"<ul class="value-list dep-list">{rows}</ul>"#))
}

/// A title-block value for a scalar field, or `None` if it isn't a scalar.
fn title_value_html(key: &str, value: &Value, status: &str, page: &Page) -> Option<String> {
    let today = page.today;
    let text = match value {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    if text.is_empty() {
        return None;
    }
    let day = |s: &str| {
        parse_date(s).map(|d| {
            format!(
                r#"<time datetime="{}">{}</time>"#,
                d.format("%Y-%m-%d"),
                fmt_day(d, today)
            )
        })
    };
    Some(match key {
        "owner" => owner_html(&text),
        "priority" => priority_html(text.parse().ok()?),
        "due_date" => {
            let d = parse_date(&text)?;
            let date = day(&text)?;
            if is_closed(status) {
                date
            } else {
                let (phrase, class) = due_phrase(d, today);
                format!(r#"{date}<span class="tb-sub due-{class}">{phrase}</span>"#)
            }
        }
        "date" | "start_date" | "end_date" | "target_date" => {
            day(&text).unwrap_or_else(|| escape_html(&text))
        }
        "email" => format!(
            r#"<a class="plain-link" href="mailto:{0}">{0}</a>"#,
            escape_html(&text)
        ),
        _ => {
            if page.catalog.canonical_id(&text).is_some() {
                return None;
            }
            escape_html(&text)
        }
    })
}

/// The ruled title block: ID, status and the most important scalar fields.
/// Returns the markup and the keys it shows.
fn title_block(entity: &EntityRecord, page: &Page) -> (String, Vec<&'static str>) {
    let fm = &entity.frontmatter;
    let status = frontmatter::get_str_or(fm, "status", "");
    let cell = |label: &str, value: &str| {
        format!(
            r#"<div class="tb-cell"><dt class="tb-label">{label}</dt><dd class="tb-value">{value}</dd></div>"#
        )
    };
    let mut cells = vec![cell("ID", &id_chip(&entity.id))];
    if !status.is_empty() {
        cells.push(cell("Status", &status_badge_lg(status)));
    }
    // Contacts belong to the customer whose folder they live in.
    if entity.kind == EntityKind::Contact
        && fm.get("customer").is_none()
        && fm.get("customers").is_none()
    {
        let customer = parent_customer_html(entity, page);
        if !customer.is_empty() {
            cells.push(cell("Customer", &customer));
        }
    }
    let mut shown = Vec::new();
    let is_task = entity.kind == EntityKind::Task;
    for key in TITLE_FIELDS {
        if shown.len() == TITLE_FIELD_SLOTS {
            break;
        }
        let value = fm.get(*key);
        let html = match value {
            Some(v) => title_value_html(key, v, status, page),
            None => None,
        };
        let html = match html {
            Some(h) => h,
            // Tasks always show who owns them.
            None if is_task && *key == "owner" => format!(
                r#"<span class="owner">{}<span class="owner-name muted">No owner</span></span>"#,
                avatar("")
            ),
            None => continue,
        };
        cells.push(cell(&field_label(key), &html));
        shown.push(*key);
    }
    if is_task && cells.len() < TITLE_CELLS_MAX {
        let milestone = frontmatter::get_str_or(fm, "milestone", "");
        if !milestone.trim().is_empty() {
            cells.push(cell("Milestone", &page.catalog.ref_html(milestone)));
            shown.push("milestone");
        }
        let sprint = frontmatter::get_str_or(fm, "sprint", "");
        if !sprint.trim().is_empty() {
            cells.push(cell("Sprint", &page.catalog.ref_html(sprint)));
            shown.push("sprint");
        }
    }
    (
        format!(r#"<dl class="title-block">{}</dl>"#, cells.concat()),
        shown,
    )
}

/// Render a detail page for a single entity.
pub fn detail_page(page: &Page, entity: &EntityRecord) -> String {
    let fm = &entity.frontmatter;
    let name = display_name(entity);
    let plural = entity.kind.label_plural();
    let list_href = format!("/{plural}");
    let id = escape_html(&entity.id);

    let mut body = format!(
        r#"<article class="detail" data-entity-id="{id}" data-entity-kind="{}" data-entity-title="{}">"#,
        entity.kind.label(),
        escape_html(name)
    );
    body.push_str(&format!(
        r#"<nav class="breadcrumb" aria-label="Breadcrumb"><a href="{list_href}">{}</a><span class="sep" aria-hidden="true">/</span><span class="crumb-id">{id}</span>{}</nav>"#,
        capitalize(plural),
        icon_button(
            "copy",
            "Copy ID",
            "copy-id",
            &format!(r#" data-copy="{id}" hidden"#)
        ),
    ));
    let actions = match entity.kind {
        _ if !page.editable => String::new(),
        EntityKind::Task => edit_button(),
        EntityKind::Project | EntityKind::Customer | EntityKind::Sprint | EntityKind::Milestone => {
            new_task_button(false)
        }
        _ => String::new(),
    };
    let actions = if actions.is_empty() {
        actions
    } else {
        format!(r#"<div class="detail-actions">{actions}</div>"#)
    };
    body.push_str(&format!(
        r#"<header class="detail-hero"><h1 class="detail-title">{}</h1>{actions}</header>"#,
        escape_html(name)
    ));
    let (block, promoted) = title_block(entity, page);
    body.push_str(&block);
    if page.editable && entity.kind == EntityKind::Task {
        body.push_str(&task_edit_form(page, entity));
    }

    body.push_str(r#"<div class="detail-layout"><div class="detail-main">"#);
    if let Some(summary) = frontmatter::get_str(fm, "summary").filter(|s| !s.trim().is_empty()) {
        body.push_str(&format!(
            r#"<p class="detail-summary">{}</p>"#,
            escape_html(summary)
        ));
    }
    // Hub entities (customers, projects, sprints) lead with what's linked to
    // them; documents (meetings, research, tasks) lead with their notes.
    let related = related_sections(entity, page).concat();
    let hub_first = matches!(
        entity.kind,
        EntityKind::Customer | EntityKind::Project | EntityKind::Sprint | EntityKind::Milestone
    ) && !related.is_empty();
    if hub_first {
        body.push_str(&related);
    }
    let notes = Notes::of(entity);
    if let Some(notes_html) = notes::body_html(page, entity, &notes) {
        // Notes that open with their own heading need no second one.
        let own_heading = strip_leading_h1(notes.md.trim())
            .trim_start()
            .starts_with('#');
        if hub_first && !own_heading {
            body.push_str(r#"<h2 class="section-title notes-title">Notes</h2>"#);
        }
        body.push_str(&notes_html);
    } else if related.is_empty() && notes.comments.is_empty() {
        body.push_str(r#"<p class="muted detail-empty">No notes yet. Add them below the frontmatter in the Markdown file.</p>"#);
    }
    if !hub_first {
        body.push_str(&related);
    }
    body.push_str(&notes::comments_section(page, entity, &notes));
    body.push_str("</div>");

    body.push_str(&rail(entity, page, &promoted));
    body.push_str("</div></article>");

    layout(
        page,
        &format!("{} ({})", name, entity.id),
        &list_href,
        "",
        &body,
    )
}

/// The title block and field rail of an entity's detail page, for updating
/// them in place after an edit.
pub fn detail_fragments(page: &Page, entity: &EntityRecord) -> (String, String) {
    let (block, promoted) = title_block(entity, page);
    let rail = rail(entity, page, &promoted);
    (block, rail)
}

/// The field rail: every remaining frontmatter field plus the source path.
fn rail(entity: &EntityRecord, page: &Page, promoted: &[&str]) -> String {
    let mut html = String::from(
        r#"<aside class="detail-sidebar" aria-label="Details"><h2 class="rail-title">Details</h2><dl class="fields">"#,
    );
    if let Some(map) = entity.frontmatter.as_mapping() {
        let mut keys: Vec<&str> = map
            .keys()
            .filter_map(|k| k.as_str())
            .filter(|k| !k.starts_with('_') && !HIDDEN_FIELDS.contains(k) && !promoted.contains(k))
            .collect();
        let rank = |k: &str| -> (usize, usize) {
            if let Some(i) = LEADING_FIELDS.iter().position(|f| *f == k) {
                (0, i)
            } else if let Some(i) = TRAILING_FIELDS.iter().position(|f| *f == k) {
                (2, i)
            } else {
                (1, 0)
            }
        };
        keys.sort_by_key(|k| rank(k)); // stable: other keys keep file order
        for key in keys {
            let Some(value) = map.get(Value::String(key.to_string())) else {
                continue;
            };
            if let Some(v) = field_value_html(key, value, page) {
                html.push_str(&format!(
                    r#"<div class="field"><dt>{}</dt><dd>{v}</dd></div>"#,
                    escape_html(&field_label(key))
                ));
            }
        }
    }
    html.push_str("</dl>");
    let source = entity
        .source_path
        .strip_prefix(&page.cfg.root)
        .unwrap_or(&entity.source_path)
        .display()
        .to_string();
    html.push_str(&format!(
        r#"<div class="detail-source" title="Source file"><code>{}</code></div></aside>"#,
        escape_html(&source)
    ));
    html
}

/// Sections listing entities related to `entity`: everything that references
/// its ID, plus contacts stored under a customer's directory.
fn related_sections(entity: &EntityRecord, page: &Page) -> Vec<String> {
    let catalog = page.catalog;
    // One bucket per kind, in RELATED_ORDER.
    let mut buckets: Vec<Vec<&EntityRecord>> = vec![Vec::new(); RELATED_ORDER.len()];
    // Tasks that only mention this entity in their text: listed apart, so
    // they don't count towards its progress.
    let mut mentioned: Vec<&EntityRecord> = Vec::new();

    // A customer's or project's own folder holds its contacts and tasks.
    let hub_dir = entity.source_path.parent().filter(|d| {
        matches!(entity.kind, EntityKind::Customer | EntityKind::Project)
            && *d != page.cfg.customers_dir
            && *d != page.cfg.projects_dir
    });
    let title = display_name(entity);
    for rec in &catalog.records {
        if rec.id == entity.id {
            continue;
        }
        let inside = hub_dir.is_some_and(|d| rec.source_path.starts_with(d));
        let is_child = inside && matches!(rec.kind, EntityKind::Contact | EntityKind::Task);
        let linked = is_child
            || catalog.record_links(rec, &entity.id)
            || (entity.kind == EntityKind::Sprint
                && rec.kind == EntityKind::Task
                && sprint_named(rec, title));
        let Some(i) = RELATED_ORDER.iter().position(|k| *k == rec.kind) else {
            continue;
        };
        if linked {
            buckets[i].push(rec);
        } else if catalog.record_mentions(rec, &entity.id) {
            if rec.kind == EntityKind::Task {
                mentioned.push(rec);
            } else {
                buckets[i].push(rec);
            }
        }
    }

    let mut sections = Vec::new();
    for (kind, mut recs) in RELATED_ORDER.iter().zip(buckets) {
        if recs.is_empty() {
            continue;
        }
        let total = recs.len();
        let mut html = format!(
            r#"<section class="related-section">{}"#,
            section_title(&capitalize(kind.label_plural()), Some(total))
        );
        match kind {
            EntityKind::Meeting => html.push_str(&meeting_rows(&mut recs, page)),
            EntityKind::Task => {
                // Open work first, finished work last.
                recs.sort_by_cached_key(|t| task_order(t));
                let done = recs
                    .iter()
                    .filter(|e| {
                        matches!(
                            frontmatter::get_str_or(&e.frontmatter, "status", ""),
                            "done" | "completed"
                        )
                    })
                    .count();
                // Cancelled work doesn't count towards progress.
                let planned = recs
                    .iter()
                    .filter(|e| {
                        !is_cancelled(frontmatter::get_str_or(&e.frontmatter, "status", ""))
                    })
                    .count();
                // All cancelled: there's nothing to make progress on.
                if planned > 0 {
                    html.push_str(&format!(
                        r#"<div class="related-progress">{}<span class="progress-caption">{done} of {planned} done</span></div>"#,
                        progress_bar(done, planned),
                    ));
                }
                html.push_str(&related_list(&recs, page, "more"));
            }
            _ => html.push_str(&related_list(&recs, page, "more")),
        }
        html.push_str("</section>");
        sections.push(html);
    }
    if !mentioned.is_empty() {
        mentioned.sort_by_cached_key(|t| task_order(t));
        sections.push(format!(
            r#"<section class="related-section related-mentions">{}{}</section>"#,
            section_title("Mentioned in", Some(mentioned.len())),
            related_list(&mentioned, page, "more")
        ));
    }
    sections
}

/// Whether a task names `sprint` by title (`sprint: Sprint 1 - Kickoff`)
/// rather than by ID, as hand-written files sometimes do.
fn sprint_named(task: &EntityRecord, sprint: &str) -> bool {
    let value = frontmatter::get_link_str(&task.frontmatter, "sprint").unwrap_or("");
    let value = value.trim();
    !value.is_empty() && value.eq_ignore_ascii_case(sprint.trim())
}

/// Upcoming meetings first (soonest first), then past ones (newest first)
/// behind a disclosure.
fn meeting_rows(recs: &mut [&EntityRecord], page: &Page) -> String {
    let today_s = page.today.format("%Y-%m-%d").to_string();
    let date = |e: &EntityRecord| frontmatter::get_str_or(&e.frontmatter, "date", "").to_string();
    recs.sort_by_key(|e| std::cmp::Reverse(date(e)));
    let split = recs
        .iter()
        .position(|e| date(e) < today_s)
        .unwrap_or(recs.len());
    let (upcoming, past) = recs.split_at_mut(split);
    if upcoming.is_empty() {
        return related_list(past, page, "more");
    }
    upcoming.reverse();
    let mut html = rows_html(upcoming, page);
    if !past.is_empty() {
        html.push_str(&format!(
            r#"<details class="related-more"><summary>Show {} earlier</summary>{}</details>"#,
            past.len(),
            rows_html(past, page)
        ));
    }
    html
}

/// A related list, collapsing the tail behind "Show N {word}".
fn related_list(recs: &[&EntityRecord], page: &Page, word: &str) -> String {
    let total = recs.len();
    if total <= RELATED_VISIBLE + 2 {
        return rows_html(recs, page);
    }
    format!(
        r#"{}<details class="related-more"><summary>Show {} {word}</summary>{}</details>"#,
        rows_html(&recs[..RELATED_VISIBLE], page),
        total - RELATED_VISIBLE,
        rows_html(&recs[RELATED_VISIBLE..], page)
    )
}

fn rows_html(recs: &[&EntityRecord], page: &Page) -> String {
    let rows: String = recs
        .iter()
        .map(|e| {
            let fm = &e.frontmatter;
            let status = frontmatter::get_str_or(fm, "status", "");
            let meta = match e.kind {
                EntityKind::Meeting => {
                    date_html(frontmatter::get_str_or(fm, "date", ""), page.today)
                }
                EntityKind::Task => {
                    let owner = frontmatter::get_str_or(fm, "owner", "");
                    let owner = if owner.is_empty() {
                        String::new()
                    } else {
                        format!(
                            r#"<span class="related-owner">{}</span>"#,
                            escape_html(owner)
                        )
                    };
                    format!(
                        "{owner}{}",
                        task_due_html(
                            frontmatter::get_str_or(fm, "due_date", ""),
                            status,
                            page.today
                        )
                    )
                }
                EntityKind::Contact => escape_html(frontmatter::get_str_or(fm, "role", "")),
                _ => escape_html(frontmatter::get_str_or(fm, "owner", "")),
            };
            format!(
                r#"<li><span class="related-status">{}</span><span class="related-name">{}{}</span><span class="related-meta">{meta}</span></li>"#,
                status_badge(status),
                entity_name_link(e),
                id_chip(&e.id),
            )
        })
        .collect();
    format!(r#"<ul class="related-list">{rows}</ul>"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    #[test]
    fn title_block_promotes_scalars_and_rail_skips_them() {
        let (_d, cfg) = test_config();
        let today = chrono::Local::now().date_naive();
        let due = (today + chrono::Duration::days(4)).format("%Y-%m-%d");
        let task = rec(
            EntityKind::Task,
            "TASK-068",
            &format!("title: \"Spec <A&B>\"\nstatus: todo\npriority: 3\nowner: Florian Fromm\ndue_date: {due}\ntags: [vla]\ncustom: \"<i>x</i>\""),
        );
        let cat = catalog(vec![rec(EntityKind::Task, "TASK-068", "title: x")], &cfg);
        let page = Page::new(&cfg, &cat, "");
        let html = detail_page(&page, &task);
        assert!(html.contains(r#"<dl class="title-block">"#));
        assert!(html.contains("badge-lg"));
        assert!(html.contains("Due in 4 days"));
        assert!(html.contains("Spec &lt;A&amp;B&gt;"));
        assert!(html.contains("&lt;i&gt;x&lt;/i&gt;"));
        assert!(html.contains(r#"data-copy="TASK-068""#));
        // Owner is in the title block, not repeated in the rail.
        assert!(!html.contains("<dt>Owner</dt>"));
        assert!(html.contains("<dt>Tags</dt>"));
    }

    fn at(mut r: EntityRecord, path: std::path::PathBuf) -> EntityRecord {
        r.source_path = path;
        r
    }

    #[test]
    fn hub_tasks_are_linked_ones_and_mentions_are_listed_apart() {
        let (_d, cfg) = test_config();
        let dir = cfg.projects_dir.join("PROJ-001-apollo");
        let mut mention = rec(
            EntityKind::Task,
            "TASK-002",
            "title: Elsewhere\nstatus: todo\nprojects: ['[[PROJ-009]]']",
        );
        mention.body = "Collides with [[PROJ-001]].".into();
        let cat = catalog(
            vec![
                at(
                    rec(EntityKind::Project, "PROJ-001", "name: Apollo"),
                    dir.join("PROJ-001.md"),
                ),
                rec(
                    EntityKind::Task,
                    "TASK-001",
                    "title: Linked\nstatus: done\nprojects: ['[[PROJ-001]]']",
                ),
                mention,
                at(
                    rec(EntityKind::Task, "TASK-003", "title: Scoped\nstatus: todo"),
                    dir.join("tasks/todo/TASK-003-scoped.md"),
                ),
            ],
            &cfg,
        );
        let page = Page::new(&cfg, &cat, "");
        let html = related_sections(&cat.records[0], &page).concat();
        let tasks = html.find(">Tasks <").expect("tasks section");
        let mentioned = html.find(">Mentioned in <").expect("mentions section");
        assert!(html.contains("1 of 2 done"), "{html}");
        assert!(tasks < html.find(">Linked<").unwrap());
        assert!(html.find(">Scoped<").unwrap() < mentioned);
        assert!(html.find(">Elsewhere<").unwrap() > mentioned);
    }

    #[test]
    fn sprint_lists_tasks_naming_it_by_title() {
        let (_d, cfg) = test_config();
        let cat = catalog(
            vec![
                rec(EntityKind::Sprint, "SPR-001", "title: Sprint 1 - Kickoff"),
                rec(
                    EntityKind::Task,
                    "TASK-001",
                    "title: By name\nsprint: sprint 1 - kickoff",
                ),
                rec(
                    EntityKind::Task,
                    "TASK-002",
                    "title: By ID\nsprint: '[[SPR-001]]'",
                ),
                rec(
                    EntityKind::Task,
                    "TASK-003",
                    "title: Other\nsprint: Sprint 2",
                ),
            ],
            &cfg,
        );
        let page = Page::new(&cfg, &cat, "");
        let html = related_sections(&cat.records[0], &page).concat();
        assert!(html.contains(">By name<") && html.contains(">By ID<"));
        assert!(!html.contains(">Other<"));
    }

    #[test]
    fn contact_shows_its_customer_and_tasks_show_dependency_status() {
        let (_d, cfg) = test_config();
        let folder = cfg.customers_dir.join("CUST-001-acme");
        let cat = catalog(
            vec![
                at(
                    rec(EntityKind::Customer, "CUST-001", "name: Acme"),
                    folder.join("CUST-001.md"),
                ),
                at(
                    rec(
                        EntityKind::Contact,
                        "CONT-001",
                        "name: Jane\nstatus: active",
                    ),
                    folder.join("contacts/CONT-001-jane.md"),
                ),
                rec(
                    EntityKind::Task,
                    "TASK-002",
                    "title: Register\nstatus: done",
                ),
                rec(
                    EntityKind::Task,
                    "TASK-003",
                    "title: Open one\nstatus: todo",
                ),
            ],
            &cfg,
        );
        let page = Page::new(&cfg, &cat, "");
        let (block, _) = title_block(&cat.records[1], &page);
        assert!(block.contains(r#"<dt class="tb-label">Customer</dt><dd class="tb-value"><a class="ref" href="/entity/CUST-001""#), "{block}");

        let task = rec(
            EntityKind::Task,
            "TASK-001",
            "title: T\nstatus: todo\ndepends_on: ['[[TASK-002]]', '[[TASK-003]]']",
        );
        let html = detail_page(&page, &task);
        assert!(
            html.contains(
                r#"<ul class="value-list dep-list"><li class="dep is-done" title="Done">"#
            ),
            "{html}"
        );
        assert!(html.contains(r#"<li class="dep" title="Todo">"#));
        assert!(html.contains(r#"<span class="id-prefix">TASK-</span>002"#));
    }

    #[test]
    fn hub_notes_get_no_second_heading_when_they_open_with_one() {
        let (_d, cfg) = test_config();
        let mut project = rec(EntityKind::Project, "PROJ-001", "name: Apollo");
        let task = rec(
            EntityKind::Task,
            "TASK-001",
            "title: T\nprojects: ['[[PROJ-001]]']",
        );
        let cat = catalog(vec![project.clone(), task], &cfg);
        let page = Page::new(&cfg, &cat, "");
        project.body = "# Apollo\n\n## Overview\n\nText".into();
        assert!(!detail_page(&page, &project).contains("notes-title"));
        project.body = "Just text".into();
        assert!(detail_page(&page, &project).contains("notes-title"));
    }

    #[test]
    fn meetings_split_upcoming_and_past() {
        let (_d, cfg) = test_config();
        let today = chrono::Local::now().date_naive();
        let d = |n: i64| (today + chrono::Duration::days(n)).format("%Y-%m-%d");
        let cat = catalog(
            vec![
                rec(EntityKind::Customer, "CUST-001", "name: Acme"),
                rec(
                    EntityKind::Meeting,
                    "MTG-001",
                    &format!("title: Past\ndate: {}\ncustomers: [CUST-001]", d(-3)),
                ),
                rec(
                    EntityKind::Meeting,
                    "MTG-002",
                    &format!("title: Later\ndate: {}\ncustomers: [CUST-001]", d(9)),
                ),
                rec(
                    EntityKind::Meeting,
                    "MTG-003",
                    &format!("title: Soon\ndate: {}\ncustomers: [CUST-001]", d(1)),
                ),
            ],
            &cfg,
        );
        let page = Page::new(&cfg, &cat, "");
        let cust = &cat.records[0];
        let html = related_sections(cust, &page).concat();
        let soon = html.find(">Soon<").unwrap();
        let later = html.find(">Later<").unwrap();
        let past = html.find(">Past<").unwrap();
        assert!(soon < later && later < past);
        assert!(html.contains("Show 1 earlier"));
    }
}
