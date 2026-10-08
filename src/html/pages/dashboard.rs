//! The overview page: flight plan, attention and coming-up panels, status
//! board and recent activity.

use super::flight_plan::flight_plan;
use crate::config::ResolvedConfig;
use crate::data::{self, EntityRecord, RecentFile};
use crate::entity::EntityKind;
use crate::frontmatter;
use crate::html::catalog::display_name;
use crate::html::components::{
    due_html, empty_state, entity_name_link, id_chip, page_header, panel, priority_compact,
    progress_bar_planned, section_title, status_bar, status_lamp,
};
use crate::html::format::{
    capitalize, entity_href, escape_html, files_href, fmt_date, fmt_day_relative, href_with,
    iso_week, parse_date, plural, status_label, time_ago,
};
use crate::html::layout::layout;
use crate::html::{is_cancelled, is_closed, Page, NAV_GROUPS};
use chrono::NaiveDate;
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;
use std::time::SystemTime;

const ATTENTION_ROWS: usize = 8;
const MEETING_ROWS: usize = 6;
const ACTIVITY_ROWS: usize = 12;

/// Render the dashboard page.
pub fn dashboard_page(page: &Page, recent: &[RecentFile]) -> String {
    let today = page.today;
    let actions = format!(
        r#"<time class="readout page-date" datetime="{}">{}</time><span class="readout muted">{}</span>"#,
        today.format("%Y-%m-%d"),
        today.format("%A, %-d %B %Y"),
        iso_week(today),
    );
    let mut body = page_header("Overview", "", &actions);
    body.push_str(&flight_plan(page));

    body.push_str(r#"<div class="overview-grid">"#);
    if page.catalog.records.is_empty() {
        body.push_str(&panel(
            "Get started",
            "",
            &empty_state(
                "Add your first task",
                r#"Run <code>mc new task "Title"</code> in this repo, then reload. Give it a due date to see it on the flight plan."#,
                "",
            ),
            None,
        ));
    } else {
        body.push_str(&attention_panel(page));
        body.push_str(&coming_up_panel(page));
    }
    body.push_str("</div>");

    body.push_str(&super::milestones::overview(page));
    body.push_str(&status_board(page));
    body.push_str(&activity(page, recent));

    layout(page, "Overview", "/", "", &body)
}

/// Overdue and soon-due tasks.
fn attention_panel(page: &Page) -> String {
    let today = page.today;
    let horizon = today + chrono::Duration::days(7);
    let tasks: Vec<&EntityRecord> = page.catalog.of_kind(EntityKind::Task).collect();

    let mut due: Vec<(NaiveDate, &EntityRecord)> = tasks
        .iter()
        .filter(|t| !is_closed(frontmatter::get_str_or(&t.frontmatter, "status", "")))
        .filter_map(|t| {
            parse_date(frontmatter::get_str_or(&t.frontmatter, "due_date", ""))
                .filter(|d| *d <= horizon)
                .map(|d| (d, *t))
        })
        .collect();
    due.sort_by_key(|(d, t)| {
        (
            *d,
            data::get_number(&t.frontmatter, "priority").unwrap_or(3),
        )
    });
    let overdue = due.iter().filter(|(d, _)| *d < today).count();
    let soon = due.len() - overdue;
    let count_status = |s: &str| {
        tasks
            .iter()
            .filter(|t| frontmatter::get_str_or(&t.frontmatter, "status", "") == s)
            .count()
    };

    let mut body = String::new();
    if tasks.is_empty() {
        body.push_str(&empty_state(
            "Add your first task",
            r#"Run <code>mc new task "Title"</code> and give it a due date."#,
            "",
        ));
    } else if due.is_empty() {
        body.push_str(&empty_state(
            "Nothing overdue",
            "Open tasks due in the next 7 days show up here.",
            "",
        ));
    } else {
        body.push_str(r#"<ul class="item-list">"#);
        for (d, t) in due.iter().take(ATTENTION_ROWS) {
            let project = frontmatter::get_link_list(&t.frontmatter, "projects")
                .first()
                .map(|p| {
                    let name = page.catalog.name(p).unwrap_or(p);
                    format!(
                        r#"<span class="item-context" title="{0}">{0}</span>"#,
                        escape_html(name)
                    )
                })
                .unwrap_or_default();
            let pri = data::get_number(&t.frontmatter, "priority").unwrap_or(3);
            body.push_str(&format!(
                r#"<li class="item"><span class="item-lead">{}</span><div class="item-main">{}<div class="item-meta">{}{}{project}</div></div></li>"#,
                due_html(&d.format("%Y-%m-%d").to_string(), page.today),
                entity_name_link(t),
                id_chip(&t.id),
                priority_compact(pri),
            ));
        }
        body.push_str("</ul>");
    }

    let mut more = Vec::new();
    if due.len() > ATTENTION_ROWS {
        more.push(format!(
            r#"<a href="/tasks/list?sort=due_date&amp;dir=asc">{} more on the task list</a>"#,
            due.len() - ATTENTION_ROWS
        ));
    }
    if !tasks.is_empty() {
        more.push(format!(
            r#"<a href="/tasks">{} in progress, {} in review</a>"#,
            count_status("in-progress"),
            count_status("review")
        ));
    }
    if !more.is_empty() {
        body.push_str(&format!(
            r#"<p class="panel-more">{}</p>"#,
            more.join(r#"<span class="sep" aria-hidden="true"></span>"#)
        ));
    }

    let signal = if overdue > 0 {
        Some("negative")
    } else if soon > 0 {
        Some("pending")
    } else {
        None
    };
    let link = format!(
        r#"<a class="panel-link" href="/tasks/list?sort=due_date&amp;dir=asc">{overdue} overdue, {soon} soon</a>"#
    );
    panel("Needs attention", &link, &body, signal)
}

/// Active sprints and upcoming meetings.
fn coming_up_panel(page: &Page) -> String {
    let today = page.today;
    let catalog = page.catalog;
    let has_meetings = catalog.count_for(EntityKind::Meeting).is_some();
    let has_sprints = catalog.count_for(EntityKind::Sprint).is_some();
    if !has_meetings && !has_sprints {
        return String::new();
    }

    let mut meetings: Vec<(NaiveDate, &EntityRecord)> = catalog
        .of_kind(EntityKind::Meeting)
        .filter(|m| !is_cancelled(frontmatter::get_str_or(&m.frontmatter, "status", "")))
        .filter_map(|m| {
            parse_date(frontmatter::get_str_or(&m.frontmatter, "date", ""))
                .filter(|d| *d >= today)
                .map(|d| (d, m))
        })
        .collect();
    meetings.sort_by(|a, b| {
        a.0.cmp(&b.0).then_with(|| {
            frontmatter::get_str_or(&a.1.frontmatter, "time", "").cmp(frontmatter::get_str_or(
                &b.1.frontmatter,
                "time",
                "",
            ))
        })
    });

    let mut body = String::new();
    for s in catalog
        .of_kind(EntityKind::Sprint)
        .filter(|s| frontmatter::get_str_or(&s.frontmatter, "status", "") == "active")
    {
        body.push_str(&sprint_card(page, s));
    }

    if meetings.is_empty() {
        if has_meetings {
            body.push_str(&empty_state(
                "No meetings scheduled",
                "Meetings dated today or later show up here.",
                "",
            ));
        }
    } else {
        body.push_str(r#"<ul class="item-list">"#);
        for (d, m) in meetings.iter().take(MEETING_ROWS) {
            let time = frontmatter::get_str_or(&m.frontmatter, "time", "");
            let customers = catalog
                .refs_html_compact(&frontmatter::get_string_list(&m.frontmatter, "customers"));
            let meta = if customers.is_empty() {
                String::new()
            } else {
                format!(r#"<div class="item-meta">{customers}</div>"#)
            };
            body.push_str(&format!(
                r#"<li class="item"><span class="item-lead when"><time datetime="{}">{}</time><span>{}</span></span><div class="item-main">{}{meta}</div></li>"#,
                d.format("%Y-%m-%d"),
                fmt_day_relative(*d, today),
                escape_html(time),
                entity_name_link(m),
            ));
        }
        body.push_str("</ul>");
    }

    let link = if has_meetings {
        r#"<a class="panel-link" href="/meetings">All meetings</a>"#
    } else {
        r#"<a class="panel-link" href="/sprints">All sprints</a>"#
    };
    panel("Coming up", link, &body, None)
}

fn sprint_card(page: &Page, s: &EntityRecord) -> String {
    let today = page.today;
    let tasks: Vec<&EntityRecord> = page
        .catalog
        .of_kind(EntityKind::Task)
        .filter(|t| {
            // Older files name the sprint by its title instead of its ID.
            let sprint = frontmatter::strip_wikilink(
                frontmatter::get_str_or(&t.frontmatter, "sprint", "").trim(),
            );
            sprint == s.id
                || frontmatter::get_str(&s.frontmatter, "title").is_some_and(|title| {
                    !sprint.is_empty() && title.trim().eq_ignore_ascii_case(sprint)
                })
        })
        .collect();
    let done = tasks
        .iter()
        .filter(|t| {
            matches!(
                frontmatter::get_str_or(&t.frontmatter, "status", ""),
                "done" | "completed"
            )
        })
        .count();
    let start = parse_date(frontmatter::get_str_or(&s.frontmatter, "start_date", ""));
    let end = parse_date(frontmatter::get_str_or(&s.frontmatter, "end_date", ""));
    let left = end
        .map(|d| {
            let left = (d - today).num_days();
            if left >= 0 {
                format!("{} left", plural(left as usize, "day", "days"))
            } else {
                format!("Ended {}", fmt_date(d, today))
            }
        })
        .unwrap_or_default();
    let planned = match (start, end) {
        (Some(a), Some(b)) if b > a => {
            let pct = (today - a).num_days() as f64 / (b - a).num_days() as f64 * 100.0;
            Some(pct.clamp(0.0, 100.0).round() as u32)
        }
        _ => None,
    };
    let pct = (done * 100).checked_div(tasks.len()).unwrap_or(0) as u32;
    let mut caption = if tasks.is_empty() {
        "No tasks assigned yet".to_string()
    } else {
        format!("{done} of {} done", tasks.len())
    };
    if let Some(p) = planned {
        caption.push_str(&format!(", {p}% of time used"));
        if !tasks.is_empty() && p > pct + 15 {
            caption.push_str(r#"<span class="behind">, behind plan</span>"#);
        }
    }
    format!(
        r#"<div class="sprint-card"><div class="sprint-head"><a class="name-link" href="{}">{}</a><span class="sprint-left readout">{left}</span></div>{}<p class="progress-caption">{caption}</p></div>"#,
        entity_href(&s.id),
        escape_html(display_name(s)),
        progress_bar_planned(done, tasks.len(), planned),
    )
}

/// One row per entity type with a status bar and a clickable legend.
fn status_board(page: &Page) -> String {
    let mut html = format!(
        r#"<section class="section">{}<div class="status-board">"#,
        section_title("Status", None)
    );
    for (_, kinds) in NAV_GROUPS {
        for kind in kinds.iter() {
            let Some(sc) = page.catalog.count_for(*kind) else {
                continue;
            };
            let plural = kind.label_plural();
            let legend: String = sc
                .by_status
                .iter()
                .map(|(s, n)| {
                    format!(
                        r#"<a class="legend-item" href="{}">{}{n} {}</a>"#,
                        href_with(&format!("/{plural}"), &[("status", s)]),
                        status_lamp(s),
                        escape_html(&status_label(s).to_lowercase())
                    )
                })
                .collect();
            let legend = if legend.is_empty() {
                format!(
                    r#"<span class="legend-empty">No {plural} yet. Add one with <code>mc new {}</code></span>"#,
                    kind.label()
                )
            } else {
                legend
            };
            html.push_str(&format!(
                r#"<div class="board-row{}"><a class="board-label" href="/{plural}"><span class="board-name">{}</span><span class="board-total readout">{}</span></a><div class="board-track">{}<div class="board-legend">{legend}</div></div></div>"#,
                if sc.total == 0 { " board-row-empty" } else { "" },
                capitalize(plural),
                sc.total,
                status_bar(sc, false),
            ));
        }
    }
    html.push_str("</div></section>");
    html
}

/// Recently modified files.
fn activity(page: &Page, recent: &[RecentFile]) -> String {
    let mut html = format!(
        r#"<section class="section">{}"#,
        section_title("Recently changed", None)
    );
    if recent.is_empty() {
        html.push_str(&empty_state(
            "No changes yet",
            "Files you edit in this repo show up here.",
            "",
        ));
        html.push_str("</section>");
        return html;
    }
    let now = SystemTime::now();
    html.push_str(r#"<ol class="activity">"#);
    for f in recent.iter().take(ACTIVITY_ROWS) {
        let kind_label = detect_entity_type(&f.id, &f.path, page.cfg);
        let rel = f
            .path
            .strip_prefix(&page.cfg.root)
            .unwrap_or(&f.path)
            .display()
            .to_string();
        let shown = if f.name.is_empty() { &rel } else { &f.name };
        let primary = if !f.id.is_empty() && page.catalog.name(&f.id).is_some() {
            format!(
                r#"<a class="name-link" href="{}">{}</a>"#,
                entity_href(&f.id),
                escape_html(if f.name.is_empty() { &f.id } else { &f.name })
            )
        } else if let Some(href) = f
            .path
            .strip_prefix(&page.cfg.root)
            .ok()
            .and_then(files_href)
        {
            // Other notes in the repo open as rendered files.
            format!(
                r#"<a class="name-link activity-name" href="{href}">{}</a>"#,
                escape_html(shown)
            )
        } else {
            format!(
                r#"<span class="activity-name">{}</span>"#,
                escape_html(shown)
            )
        };
        let id = if f.id.is_empty() {
            String::new()
        } else {
            id_chip(&f.id)
        };
        let ctx = match extract_path_context(&f.path, &page.cfg.root) {
            // The customer or project folder a file sits in, unless it's
            // that entity's own file.
            Some(dir) => match page.catalog.canonical_id(&dir) {
                Some(id) if id == f.id => String::new(),
                Some(id) if page.catalog.name(id).is_some() => format!(
                    r#"<span class="activity-context">in {}</span>"#,
                    page.catalog.ref_html(id)
                ),
                _ => format!(
                    r#"<span class="activity-context">in {}</span>"#,
                    escape_html(&strip_id_prefix(&dir).replace('-', " "))
                ),
            },
            None => String::new(),
        };
        html.push_str(&format!(
            r#"<li><span class="activity-kind">{}</span><span class="activity-main">{primary}{id}{ctx}</span><span class="activity-time" title="{}">{}</span></li>"#,
            capitalize(kind_label),
            escape_html(&rel),
            time_ago(f.modified, now),
        ));
    }
    html.push_str("</ol></section>");
    html
}

/// Detect entity type label from an entity ID and file path, using configured prefixes.
fn detect_entity_type(id: &str, path: &Path, cfg: &ResolvedConfig) -> &'static str {
    if !id.is_empty() {
        // Longest prefix wins so e.g. "TASK" doesn't shadow a longer prefix.
        let mut candidates: Vec<(&str, &'static str)> = EntityKind::ALL
            .iter()
            .map(|k| (k.prefix(cfg), k.label()))
            .collect();
        candidates.sort_by_key(|c| std::cmp::Reverse(c.0.len()));
        for (prefix, label) in candidates {
            if id.starts_with(&format!("{prefix}-")) {
                return label;
            }
        }
    }
    let path_str = path.to_string_lossy();
    if path_str.contains("/contacts/") || path_str.contains("/team/") {
        "contact"
    } else if path_str.contains("/customers/") {
        "customer"
    } else if path_str.contains("/projects/") {
        "project"
    } else if path_str.contains("/meetings/") {
        "meeting"
    } else if path_str.contains("/research/") {
        "research"
    } else if path_str.contains("/tasks/")
        || path_str.contains("/todo/")
        || path_str.contains("/done/")
    {
        "task"
    } else if path_str.contains("/sprints/") {
        "sprint"
    } else if path_str.contains("/proposals/") {
        "proposal"
    } else {
        "file"
    }
}

/// The customer or project folder a file sits in (`CUST-001-acme-inc`), if any.
fn extract_path_context(path: &Path, root: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let rel_str = rel.to_string_lossy();
    let parts: Vec<&str> = rel_str.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        if (*part == "customers" || *part == "projects") && parts.len() > i + 2 {
            if let Some(parent) = parts.get(i + 1).filter(|p| !p.is_empty()) {
                return Some(parent.to_string());
            }
        }
    }
    None
}

/// Strip entity ID prefix from a directory name (e.g. "CUST-001-acme" → "acme").
fn strip_id_prefix(dirname: &str) -> String {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Z]+-\d+-(.+)$").expect("static regex"));
    match RE.captures(dirname) {
        Some(caps) => caps[1].to_string(),
        None => dirname.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::catalog::tests::{catalog, rec, test_config};
    use std::path::PathBuf;

    #[test]
    fn path_context_uses_parent_entity_dir() {
        let root = PathBuf::from("/r");
        assert_eq!(
            extract_path_context(
                &PathBuf::from("/r/customers/CUST-001-acme-inc/contacts/CONT-001.md"),
                &root
            )
            .as_deref(),
            Some("CUST-001-acme-inc")
        );
        assert_eq!(
            extract_path_context(&PathBuf::from("/r/customers/CUST-001.md"), &root),
            None
        );
    }

    #[test]
    fn activity_links_documents_and_names_their_folder() {
        let (_d, cfg) = test_config();
        let dir = cfg.projects_dir.join("PROJ-003-sprind-next");
        let mut project = rec(
            EntityKind::Project,
            "PROJ-003",
            "name: SPRIND Next Frontier",
        );
        project.source_path = dir.join("PROJ-003.md");
        let cat = catalog(vec![project], &cfg);
        let page = Page::new(&cfg, &cat, "");
        let file = |id: &str, name: &str, path: std::path::PathBuf| RecentFile {
            id: id.into(),
            name: name.into(),
            modified: SystemTime::now(),
            path,
        };
        let html = activity(
            &page,
            &[
                file("PROJ-003", "SPRIND Next Frontier", dir.join("PROJ-003.md")),
                file("", "Track 1 brief", dir.join("application/track 1.md")),
            ],
        );
        // The project's own file doesn't repeat itself as context.
        assert_eq!(html.matches("activity-context").count(), 1, "{html}");
        assert!(html.contains(
            r#"<span class="activity-context">in <a class="ref" href="/entity/PROJ-003""#
        ));
        assert!(html.contains(r#"<a class="name-link activity-name" href="/files/projects/PROJ-003-sprind-next/application/track%201.md">Track 1 brief</a>"#), "{html}");
    }

    #[test]
    fn dashboard_signals_overdue_work() {
        let (_d, cfg) = test_config();
        let today = chrono::Local::now().date_naive();
        let late = (today - chrono::Duration::days(3)).format("%Y-%m-%d");
        let cat = catalog(
            vec![rec(
                EntityKind::Task,
                "TASK-001",
                &format!("title: \"<Late>\"\nstatus: todo\npriority: 2\ndue_date: {late}"),
            )],
            &cfg,
        );
        let page = Page {
            cfg: &cfg,
            catalog: &cat,
            custom_css: "",
            today,
            editable: false,
        };
        let html = attention_panel(&page);
        assert!(html.contains("panel signal tone-negative"));
        assert!(html.contains("1 overdue, 0 soon"));
        assert!(html.contains("&lt;Late&gt;"));
        assert!(html.contains(">3d late<"));
    }

    #[test]
    fn empty_repo_shows_get_started() {
        let (_d, cfg) = test_config();
        let cat = catalog(Vec::new(), &cfg);
        let page = Page::new(&cfg, &cat, "");
        let html = dashboard_page(&page, &[]);
        assert!(html.contains("Add your first task"));
        assert!(html.contains("flight-plan"));
        assert!(!html.contains("Needs attention"));
    }
}
