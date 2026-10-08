//! Scheduled task groups with a seek-free, accessible HTML/CSS Gantt chart.
use crate::data::EntityRecord;
use crate::entity::EntityKind;
use crate::frontmatter;
use crate::html::catalog::display_name;
use crate::html::components::{
    empty_state, filter_cell, link_button, page_header, status_badge, Btn,
};
use crate::html::format::{entity_href, escape_html, href_with, parse_date, plural};
use crate::html::layout::layout;
use crate::html::{is_cancelled, is_closed, Page};
use chrono::{Datelike, Duration, NaiveDate};

fn value<'a>(r: &'a EntityRecord, key: &str) -> &'a str {
    frontmatter::get_str_or(&r.frontmatter, key, "")
}
fn tasks<'a>(page: &'a Page, m: &EntityRecord) -> Vec<&'a EntityRecord> {
    page.catalog
        .of_kind(EntityKind::Task)
        .filter(|t| {
            frontmatter::get_link_str(&t.frontmatter, "milestone").is_some_and(|id| id == m.id)
        })
        .collect()
}
fn groups<'a>(page: &'a Page, project: Option<&str>) -> Vec<&'a EntityRecord> {
    let mut groups: Vec<_> = page
        .catalog
        .of_kind(EntityKind::Milestone)
        .filter(|m| {
            project.is_none_or(|p| {
                frontmatter::get_link_list(&m.frontmatter, "projects")
                    .iter()
                    .any(|id| id == p)
            })
        })
        .collect();
    groups.sort_by_key(|m| {
        (
            parse_date(value(m, "start_date"))
                .or_else(|| parse_date(value(m, "due_date")))
                .unwrap_or(NaiveDate::MAX),
            m.id.clone(),
        )
    });
    groups
}

pub fn milestones_page(page: &Page, project: Option<&str>) -> String {
    let groups = groups(page, project);
    let projects: Vec<_> = page
        .catalog
        .of_kind(EntityKind::Project)
        .map(|p| (p.id.clone(), display_name(p).to_string()))
        .collect();
    let mut body = page_header(
        "Milestones",
        &format!(
            "{} · expand a row to see its tasks",
            plural(groups.len(), "work package", "work packages")
        ),
        "",
    );
    body.push_str(&format!(r#"<form class="toolbar filter-form" method="get" action="/milestones">{}<noscript><button class="btn" type="submit">Apply</button></noscript></form>"#,filter_cell("project","Project",&projects,project)));
    if groups.is_empty()
        && project.is_some()
        && page.catalog.of_kind(EntityKind::Milestone).next().is_some()
    {
        body.push_str(&empty_state(
            "No milestones for this project",
            "Link a milestone to the project with <code>--projects</code>, or clear the filter to see every work package.",
            &link_button("/milestones", "Clear filter", Btn::Secondary, ""),
        ));
    } else if groups.is_empty() {
        body.push_str(&empty_state("No milestones yet",r#"Create a work package with <code>mc new milestone &quot;AP3: Integration&quot; --start-date 2026-09-01 --due-date 2026-11-30 --description &quot;Integrate and train the system&quot;</code>, then assign tasks with <code>mc task set TASK-024 --milestone MS-001</code>."#,""));
    } else {
        body.push_str(&chart(page, &groups, false));
        body.push_str(r#"<p class="gantt-legend"><span class="gantt-key"></span> Planned window <span class="gantt-key filled"></span> Tasks completed <span class="gantt-diamond-key">◆</span> Deadline <span class="gantt-today-key"></span> Today</p><p class="muted">Progress excludes cancelled tasks. Task markers show deadlines; a task without a start date has no inferred duration.</p>"#);
    }
    layout(page, "Milestones", "/milestones", "", &body)
}

/// Most open milestones the dashboard overview shows.
const OVERVIEW_MAX: usize = 6;

/// The dashboard's compact chart: open milestones only, so finished work
/// neither piles up nor stretches the time axis.
pub(crate) fn overview(page: &Page) -> String {
    let open: Vec<_> = groups(page, None)
        .into_iter()
        .filter(|m| !is_closed(value(m, "status")))
        .collect();
    if open.is_empty() {
        return String::new();
    }
    let shown = &open[..open.len().min(OVERVIEW_MAX)];
    let more = if open.len() > shown.len() {
        format!(
            r#"<p class="muted">{} more open on the <a href="/milestones">timeline</a>.</p>"#,
            open.len() - shown.len()
        )
    } else {
        String::new()
    };
    format!(
        r#"<section class="section"><div class="section-head"><h2>Milestones</h2><a href="/milestones">Open timeline →</a></div>{}{more}</section>"#,
        chart(page, shown, true)
    )
}

fn chart(page: &Page, groups: &[&EntityRecord], compact: bool) -> String {
    let dates: Vec<_> = groups
        .iter()
        .flat_map(|m| {
            let mut dates = vec![
                parse_date(value(m, "start_date")),
                parse_date(value(m, "due_date")),
            ];
            dates.extend(
                tasks(page, m)
                    .iter()
                    .filter(|t| !is_cancelled(value(t, "status")))
                    .flat_map(|t| {
                        [
                            parse_date(value(t, "start_date")),
                            parse_date(value(t, "due_date")),
                        ]
                    }),
            );
            dates.into_iter().flatten()
        })
        .collect();
    let range = dates.iter().min().zip(dates.iter().max()).map(|(a, b)| {
        (
            a.checked_sub_signed(Duration::days(7)).unwrap_or(*a),
            b.checked_add_signed(Duration::days(7)).unwrap_or(*b),
        )
    });
    let Some((start, end)) = range else {
        return format!(
            r#"<div class="gantt-unscheduled">{}</div>"#,
            groups
                .iter()
                .map(|m| row(page, m, None, compact))
                .collect::<String>()
        );
    };
    let span = (end - start).num_days().max(1) as f64;
    let pct = |d: NaiveDate| ((d - start).num_days() as f64 / span * 100.0).clamp(0.0, 100.0);
    let mut ticks = String::new();
    let mut grid = String::new();
    // About six labelled ticks at most, so the labels never overlap.
    let step = if span <= 200.0 {
        1
    } else if span <= 366.0 {
        2
    } else if span <= 548.0 {
        3
    } else if span <= 1095.0 {
        6
    } else {
        12
    };
    let mut year = start.year();
    let mut month = start.month();
    for _ in 0..600 {
        let Some(d) = NaiveDate::from_ymd_opt(year, month, 1) else {
            break;
        };
        if d > end {
            break;
        }
        if d >= start && (month - 1) % step == 0 {
            let left = pct(d);
            grid.push_str(&format!(
                r#"<span class="gantt-gridline" style="left:{left:.3}%"></span>"#
            ));
            // A label near the right edge would overflow the chart: it
            // hangs to the left of its gridline instead.
            let end_class = if left > 88.0 { " is-end" } else { "" };
            let label = if step == 12 {
                d.format("%Y")
            } else {
                d.format("%b %Y")
            };
            ticks.push_str(&format!(
                r#"<span class="gantt-tick{end_class}" style="left:{left:.3}%"><time datetime="{d}">{label}</time></span>"#
            ));
        }
        month += 1;
        if month > 12 {
            year += 1;
            month = 1;
        }
    }
    let today = if (start..=end).contains(&page.today) {
        format!(
            r#"<span class="gantt-today" style="left:{:.3}%" title="Today: {}"><span>Today</span></span>"#,
            pct(page.today),
            page.today
        )
    } else {
        String::new()
    };
    let rows: String = groups
        .iter()
        .map(|m| row(page, m, Some((start, end)), compact))
        .collect();
    format!(
        r#"<div class="gantt-scroll" role="region" aria-label="Milestone timeline" tabindex="0"><div class="gantt"><div class="gantt-axis"><span>Work package / completion</span><div class="gantt-scale">{ticks}</div></div><div class="gantt-rows">{rows}<div class="gantt-now-track">{grid}{today}</div></div></div></div>"#
    )
}

fn row(
    page: &Page,
    m: &EntityRecord,
    range: Option<(NaiveDate, NaiveDate)>,
    compact: bool,
) -> String {
    let all = tasks(page, m);
    let active: Vec<_> = all
        .iter()
        .filter(|t| !is_cancelled(value(t, "status")))
        .collect();
    let done = active
        .iter()
        .filter(|t| is_closed(value(t, "status")))
        .count();
    let late = active
        .iter()
        .filter(|t| {
            !is_closed(value(t, "status"))
                && parse_date(value(t, "due_date")).is_some_and(|d| d < page.today)
        })
        .count();
    // No progress figure without tasks: "0%" would read as "nothing done".
    let progress = (done * 100).checked_div(active.len());
    let warning = if late > 0 {
        format!(r#"<span class="gantt-late">{late} overdue</span>"#)
    } else {
        String::new()
    };
    let dates = match (
        parse_date(value(m, "start_date")),
        parse_date(value(m, "due_date")),
    ) {
        (Some(a), Some(b)) => format!("{a} → {b}"),
        (_, Some(b)) => format!("Due {b}"),
        (Some(a), _) => format!("From {a} · no deadline"),
        _ => "Unscheduled".into(),
    };
    let count = if active.is_empty() {
        "no tasks".to_string()
    } else {
        format!("{done}/{} done", active.len())
    };
    let label = format!(
        r#"<span class="gantt-label"><strong>{}</strong><span class="gantt-meta">{} · {count} {warning}</span><span class="gantt-dates">{dates}</span></span>"#,
        escape_html(display_name(m)),
        escape_html(&m.id),
    );
    let bar = mark(m, range, progress, page.today);
    if compact {
        return format!(
            r#"<a class="gantt-row gantt-summary" href="{}">{label}<span class="gantt-track">{bar}</span></a>"#,
            entity_href(&m.id)
        );
    }
    let mut children = String::new();
    for t in &all {
        children.push_str(&format!(r#"<a class="gantt-row gantt-task" href="{}"><span class="gantt-label"><strong>{}</strong><span class="gantt-meta">{} {}</span></span><span class="gantt-track">{}</span></a>"#,entity_href(&t.id),escape_html(display_name(t)),escape_html(&t.id),status_badge(value(t,"status")),mark(t,range,Some(if is_closed(value(t,"status")) {100} else {0}),page.today)));
    }
    if children.is_empty() {
        children.push_str(r#"<p class="gantt-description">No tasks assigned yet.</p>"#);
    }
    format!(
        r#"<details class="gantt-group"><summary class="gantt-row gantt-summary">{label}<span class="gantt-track">{bar}</span></summary><div class="gantt-description">{} <a href="{}">Details</a> · <a href="{}">Task list</a></div>{children}</details>"#,
        escape_html(value(m, "description")),
        entity_href(&m.id),
        href_with("/tasks/list", &[("milestone", &m.id)])
    )
}

fn mark(
    r: &EntityRecord,
    range: Option<(NaiveDate, NaiveDate)>,
    progress: Option<usize>,
    today: NaiveDate,
) -> String {
    let due = parse_date(value(r, "due_date"));
    let start = parse_date(value(r, "start_date"));
    let Some((a, b)) = range else {
        return r#"<span class="gantt-no-date">No dates</span>"#.into();
    };
    let span = (b - a).num_days().max(1) as f64;
    let pct = |d: NaiveDate| ((d - a).num_days() as f64 / span * 100.0).clamp(0.0, 100.0);
    let late = due.is_some_and(|d| d < today) && !is_closed(value(r, "status"));
    let class = if is_cancelled(value(r, "status")) {
        "is-cancelled"
    } else if late {
        "is-late"
    } else if is_closed(value(r, "status")) {
        "is-complete"
    } else {
        ""
    };
    let name = escape_html(display_name(r));
    let (fill, text) = match progress {
        Some(p) => (
            format!(r#"<span style="width:{p}%"></span><b>{p}%</b>"#),
            format!(", {p}% tasks done"),
        ),
        None => (String::new(), ", no tasks".to_string()),
    };
    let mut html = String::new();
    match (start, due) {
        (Some(s), Some(d)) if d >= s => html.push_str(&format!(
            r#"<span class="gantt-bar {class}" style="left:{:.3}%;width:{:.3}%" title="{name}: {s} to {d}{text}">{fill}</span>"#,
            pct(s),
            (pct(d) - pct(s)).max(0.3)
        )),
        // Started, but open-ended: a dashed bar to the end of the axis.
        (Some(s), None) => html.push_str(&format!(
            r#"<span class="gantt-bar is-open {class}" style="left:{:.3}%;width:{:.3}%" title="{name}: from {s}, no deadline{text}">{fill}</span>"#,
            pct(s),
            (100.0 - pct(s)).max(0.3)
        )),
        _ => {}
    }
    match (start, due) {
        (_, Some(d)) => html.push_str(&format!(
            r#"<span class="gantt-deadline {class}" style="left:{:.3}%" title="Deadline: {d}" aria-label="Deadline: {d}"></span>"#,
            pct(d)
        )),
        (None, None) => html.push_str(r#"<span class="gantt-no-date">No deadline</span>"#),
        (Some(_), None) => {}
    }
    if start.zip(due).is_some_and(|(s, d)| d < s) {
        html.push_str(r#"<span class="gantt-no-date">Invalid date range</span>"#);
    }
    html
}
