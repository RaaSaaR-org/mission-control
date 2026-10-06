//! Reusable HTML components. Every function returns a fragment with all
//! dynamic text escaped; arguments named `*_html` are trusted markup.

use super::catalog::display_name;
use super::format::{
    due_short, due_title, entity_href, escape_html, fmt_date, initials, parse_date, query_string,
    status_label, status_slug,
};
use crate::data::{EntityRecord, StatusCounts};
use chrono::NaiveDate;
use regex::Regex;
use std::sync::LazyLock;

// ── Icons ───────────────────────────────────────────────────────────────

/// Inline SVG sprite, emitted once per page by the layout.
pub(crate) const ICON_SPRITE: &str = r#"<svg class="icon-sprite" xmlns="http://www.w3.org/2000/svg" aria-hidden="true" focusable="false">
<symbol id="i-search" viewBox="0 0 16 16"><circle cx="7" cy="7" r="4.5"/><path d="m10.5 10.5 3.5 3.5"/></symbol>
<symbol id="i-menu" viewBox="0 0 16 16"><path d="M2.5 4.5h11M2.5 8h11M2.5 11.5h11"/></symbol>
<symbol id="i-close" viewBox="0 0 16 16"><path d="m4 4 8 8M12 4l-8 8"/></symbol>
<symbol id="i-copy" viewBox="0 0 16 16"><rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M10.5 3.5v-.5a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h.5"/></symbol>
<symbol id="i-system" viewBox="0 0 16 16"><circle cx="8" cy="8" r="5.5"/><path d="M8 2.5a5.5 5.5 0 0 1 0 11z" fill="currentColor" stroke="none"/></symbol>
<symbol id="i-sun" viewBox="0 0 16 16"><circle cx="8" cy="8" r="2.75"/><path d="M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1.06 1.06M11.54 11.54l1.06 1.06M3.4 12.6l1.06-1.06M11.54 4.46l1.06-1.06"/></symbol>
<symbol id="i-moon" viewBox="0 0 16 16"><path d="M13.5 9.6A5.5 5.5 0 1 1 6.4 2.5a4.5 4.5 0 0 0 7.1 7.1z"/></symbol>
<symbol id="i-board" viewBox="0 0 16 16"><rect x="2" y="2.5" width="3.5" height="11" rx="1"/><rect x="6.25" y="2.5" width="3.5" height="7" rx="1"/><rect x="10.5" y="2.5" width="3.5" height="9" rx="1"/></symbol>
<symbol id="i-list" viewBox="0 0 16 16"><path d="M5.5 4h8M5.5 8h8M5.5 12h8M2.5 4h.5M2.5 8h.5M2.5 12h.5"/></symbol>
<symbol id="i-rows" viewBox="0 0 16 16"><path d="M2.5 3.5h11M2.5 6.5h11M2.5 9.5h11M2.5 12.5h11"/></symbol>
<symbol id="i-plus" viewBox="0 0 16 16"><path d="M8 3v10M3 8h10"/></symbol>
<symbol id="i-more" viewBox="0 0 16 16"><circle cx="8" cy="3.5" r=".9" fill="currentColor"/><circle cx="8" cy="8" r=".9" fill="currentColor"/><circle cx="8" cy="12.5" r=".9" fill="currentColor"/></symbol>
<symbol id="i-edit" viewBox="0 0 16 16"><path d="M10.5 3 13 5.5 6 12.5H3.5V10z"/><path d="m9 4.5 2.5 2.5"/></symbol>
<symbol id="i-chevron" viewBox="0 0 16 16"><path d="m6 3.5 4.5 4.5L6 12.5"/></symbol>
<symbol id="i-refresh" viewBox="0 0 16 16"><path d="M13 8a5 5 0 1 1-1.5-3.6"/><path d="M13 2.5v3h-3"/></symbol>
<symbol id="i-calendar" viewBox="0 0 16 16"><rect x="2.5" y="3.5" width="11" height="10" rx="1.5"/><path d="M2.5 6.5h11M5.5 2v3M10.5 2v3"/></symbol>
</svg>"#;

/// An icon from the sprite. Decorative; pair it with text or an `aria-label`.
pub(crate) fn icon(name: &str) -> String {
    format!(
        r##"<svg class="icon" aria-hidden="true" focusable="false"><use href="#i-{name}"/></svg>"##
    )
}

// ── Buttons ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Btn {
    Primary,
    Secondary,
    Ghost,
}

impl Btn {
    fn class(self) -> &'static str {
        match self {
            Btn::Primary => "btn btn-primary",
            Btn::Secondary => "btn",
            Btn::Ghost => "btn btn-ghost",
        }
    }
}

/// A link styled as a button. `href` must already be a safe URL; `extra`
/// adds classes such as `btn-sm`.
pub(crate) fn link_button(href: &str, label: &str, variant: Btn, extra: &str) -> String {
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!(" {extra}")
    };
    format!(
        r#"<a class="{}{extra}" href="{href}">{}</a>"#,
        variant.class(),
        escape_html(label)
    )
}

/// A form submit button.
pub(crate) fn submit_button(label: &str, variant: Btn) -> String {
    format!(
        r#"<button type="submit" class="{}">{}</button>"#,
        variant.class(),
        escape_html(label)
    )
}

/// A square icon-only button. `attrs_html` is appended verbatim.
pub(crate) fn icon_button(icon_name: &str, label: &str, class: &str, attrs_html: &str) -> String {
    let label = escape_html(label);
    format!(
        r#"<button type="button" class="icon-btn {class}" aria-label="{label}" title="{label}"{attrs_html}>{}</button>"#,
        icon(icon_name)
    )
}

pub(crate) fn kbd(keys: &str) -> String {
    format!("<kbd>{}</kbd>", escape_html(keys))
}

// ── Status ──────────────────────────────────────────────────────────────

/// Semantic tone for a status value, used for badge and bar colors.
pub fn status_tone(status: &str) -> &'static str {
    match status.to_ascii_lowercase().as_str() {
        "active" | "done" | "completed" | "final" | "accepted" | "won" => "positive",
        "in-progress" | "review" | "scheduled" | "planning" | "proposed" | "sent" => "progress",
        "todo" | "draft" | "prospect" | "on-hold" | "pending" | "waiting" => "pending",
        "cancelled" | "canceled" | "churned" | "rejected" | "lost" | "blocked" => "negative",
        _ => "neutral",
    }
}

/// Render a status badge: a shape-coded lamp plus the status in sentence case.
pub fn status_badge(status: &str) -> String {
    badge(status, "")
}

/// A larger, tinted badge for title blocks.
pub(crate) fn status_badge_lg(status: &str) -> String {
    badge(status, " badge-lg")
}

fn badge(status: &str, extra: &str) -> String {
    if status.is_empty() {
        return String::new();
    }
    format!(
        r#"<span class="badge tone-{} badge-{}{extra}">{}</span>"#,
        status_tone(status),
        status_slug(status),
        escape_html(&status_label(status))
    )
}

/// A status lamp on its own (decorative; the status word must be nearby).
pub(crate) fn status_lamp(status: &str) -> String {
    let review = if status_slug(status) == "review" {
        " lamp-review"
    } else {
        ""
    };
    format!(
        r#"<span class="legend-dot tone-{}{review}" aria-hidden="true"></span>"#,
        status_tone(status)
    )
}

/// A lamp in a given tone. `extra` adds classes such as `lamp-solid`.
pub(crate) fn lamp(tone: &str, extra: &str) -> String {
    format!(r#"<span class="lamp tone-{tone}{extra}" aria-hidden="true"></span>"#)
}

/// A segmented bar showing the share of each status.
pub(crate) fn status_bar(sc: &StatusCounts, labeled: bool) -> String {
    if sc.total == 0 {
        return r#"<div class="status-bar status-bar-empty"></div>"#.to_string();
    }
    let segs: String = sc
        .by_status
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(status, n)| {
            let label = if labeled {
                format!("<span>{n}</span>")
            } else {
                String::new()
            };
            format!(
                r#"<div class="seg tone-{}" style="flex-grow:{n}" title="{}: {n}">{label}</div>"#,
                status_tone(status),
                escape_html(&status_label(status))
            )
        })
        .collect();
    format!(r#"<div class="status-bar" aria-hidden="true">{segs}</div>"#)
}

// ── Priority ────────────────────────────────────────────────────────────

pub(crate) fn priority_label(priority: u32) -> &'static str {
    match priority {
        1 => "Critical",
        2 => "High",
        3 => "Medium",
        4 => "Low",
        _ => "Unknown",
    }
}

pub(crate) fn priority_class(priority: u32) -> &'static str {
    match priority {
        1 => "critical",
        2 => "high",
        3 => "medium",
        _ => "low",
    }
}

/// Priority as a four-bar gauge plus its label.
pub(crate) fn priority_html(priority: u32) -> String {
    priority_gauge(priority, "")
}

/// Priority gauge with the label visually hidden, for cards and rows.
pub(crate) fn priority_compact(priority: u32) -> String {
    priority_gauge(priority, " priority-compact")
}

fn priority_gauge(priority: u32, extra: &str) -> String {
    let label = priority_label(priority);
    format!(
        r#"<span class="priority pri-{}{extra}" title="{label} priority"><span class="pri-gauge" aria-hidden="true"><i></i><i></i><i></i><i></i></span><span class="pri-label">{label}</span></span>"#,
        priority_class(priority),
    )
}

// ── People, tags, IDs ───────────────────────────────────────────────────

/// Monochrome initials avatar; an empty dashed square when there's no owner.
pub(crate) fn avatar(owner: &str) -> String {
    if owner.trim().is_empty() {
        return r#"<span class="avatar avatar-empty" title="No owner"></span>"#.to_string();
    }
    format!(
        r#"<span class="avatar" title="{}">{}</span>"#,
        escape_html(owner),
        escape_html(&initials(owner))
    )
}

/// Avatar plus the owner's name. Empty when there's no owner.
pub(crate) fn owner_html(owner: &str) -> String {
    if owner.trim().is_empty() {
        return String::new();
    }
    format!(
        r#"<span class="owner"><span class="avatar" aria-hidden="true">{}</span><span class="owner-name">{}</span></span>"#,
        escape_html(&initials(owner)),
        escape_html(owner)
    )
}

fn tag_html(tag: &str, list_href: Option<&str>) -> String {
    match list_href {
        Some(href) => format!(
            r#"<a class="tag" href="{href}?{}">{}</a>"#,
            query_string(&[("tag", tag)]),
            escape_html(tag)
        ),
        None => format!(r#"<span class="tag">{}</span>"#, escape_html(tag)),
    }
}

/// Tag chips. When `list_href` is given, each tag links to that list filtered by tag.
pub(crate) fn tag_chips(tags: &[String], list_href: Option<&str>) -> String {
    tag_chips_compact(tags, list_href, usize::MAX)
}

/// At most `max` tag chips, then a `+n` chip listing the rest in its title.
pub(crate) fn tag_chips_compact(tags: &[String], list_href: Option<&str>, max: usize) -> String {
    let tags: Vec<&str> = tags
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect();
    if tags.is_empty() {
        return String::new();
    }
    let mut html: String = tags
        .iter()
        .take(max)
        .map(|t| tag_html(t, list_href))
        .collect();
    if tags.len() > max {
        let rest = &tags[max..];
        html.push_str(&format!(
            r#"<span class="tag-more" title="{}">+{}</span>"#,
            escape_html(&rest.join(", ")),
            rest.len()
        ));
    }
    format!(r#"<span class="tags">{html}</span>"#)
}

/// An entity ID with its prefix de-emphasised: `TASK-` `068`.
pub(crate) fn id_chip(id: &str) -> String {
    static ID_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^([A-Z]+-)(\d+)$").expect("static regex"));
    match ID_RE.captures(id) {
        Some(c) => format!(
            r#"<span class="entity-id"><span class="id-prefix">{}</span>{}</span>"#,
            &c[1], &c[2]
        ),
        None => format!(r#"<span class="entity-id">{}</span>"#, escape_html(id)),
    }
}

/// Link to an entity, showing its name.
pub(crate) fn entity_name_link(e: &EntityRecord) -> String {
    format!(
        r#"<a class="name-link" href="{}">{}</a>"#,
        entity_href(&e.id),
        escape_html(display_name(e))
    )
}

/// Single-line clipped text with the full value in a tooltip.
pub(crate) fn clip(inner_html: &str, title: &str) -> String {
    if inner_html.is_empty() {
        return String::new();
    }
    format!(
        r#"<span class="clip" title="{}">{inner_html}</span>"#,
        escape_html(title)
    )
}

/// Plain text, clipped.
pub(crate) fn clip_text(text: &str) -> String {
    clip(&escape_html(text), text)
}

// ── Dates ───────────────────────────────────────────────────────────────

pub(crate) fn date_html(raw: &str, today: NaiveDate) -> String {
    match parse_date(raw) {
        Some(d) => format!(
            r#"<time datetime="{}" title="{}">{}</time>"#,
            d.format("%Y-%m-%d"),
            d.format("%a %-d %b %Y"),
            fmt_date(d, today)
        ),
        None => escape_html(raw),
    }
}

/// A due date as a compact, tone-coloured readout ("5d late", "in 4d").
pub(crate) fn due_html(raw: &str, today: NaiveDate) -> String {
    match parse_date(raw) {
        Some(d) => {
            let (text, class) = due_short(d, today);
            format!(
                r#"<time class="due due-{class}" datetime="{}" title="{}">{text}</time>"#,
                d.format("%Y-%m-%d"),
                escape_html(&due_title(d, today)),
            )
        }
        None => escape_html(raw),
    }
}

/// Due readout for open tasks, a muted plain date for finished ones.
pub(crate) fn task_due_html(raw: &str, status: &str, today: NaiveDate) -> String {
    if raw.trim().is_empty() {
        String::new()
    } else if super::is_closed(status) {
        format!(r#"<span class="muted">{}</span>"#, date_html(raw, today))
    } else {
        due_html(raw, today)
    }
}

// ── Progress ────────────────────────────────────────────────────────────

pub(crate) fn progress_bar(done: usize, total: usize) -> String {
    progress_bar_planned(done, total, None)
}

/// A trajectory bar: actual progress with a bead at its tip and, for sprints,
/// a tick where progress should be by today.
pub(crate) fn progress_bar_planned(done: usize, total: usize, planned: Option<u32>) -> String {
    let pct = (done * 100).checked_div(total).unwrap_or(0) as u32;
    let behind = total > 0 && planned.is_some_and(|p| p > pct + 15);
    let bead = if pct > 0 && pct < 100 {
        " has-bead"
    } else {
        ""
    };
    let plan = planned
        .map(|p| {
            format!(
                r#"<span class="progress-plan" style="--plan:{p}%" title="Planned by today: {p}%"></span>"#
            )
        })
        .unwrap_or_default();
    format!(
        r#"<div class="progress{}{bead}" style="--p:{pct}%" role="progressbar" aria-valuenow="{pct}" aria-valuemin="0" aria-valuemax="100" aria-label="{done} of {total} done"><div class="progress-fill" style="width:{pct}%"></div>{plan}</div>"#,
        if behind { " is-behind" } else { "" },
    )
}

// ── Containers ──────────────────────────────────────────────────────────

/// Page title row. `meta_html` and `actions_html` are trusted markup.
pub(crate) fn page_header(title: &str, meta_html: &str, actions_html: &str) -> String {
    let meta = if meta_html.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="page-meta">{meta_html}</p>"#)
    };
    let actions = if actions_html.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="page-actions">{actions_html}</div>"#)
    };
    format!(
        r#"<header class="page-header"><div class="page-heading"><h1 class="page-title">{}</h1>{meta}</div>{actions}</header>"#,
        escape_html(title)
    )
}

/// A section heading with an optional count readout.
pub(crate) fn section_title(title: &str, count: Option<usize>) -> String {
    let count = count
        .map(|n| format!(r#" <span class="count readout">{n}</span>"#))
        .unwrap_or_default();
    format!(
        r#"<h2 class="section-title">{}{count}</h2>"#,
        escape_html(title)
    )
}

/// A dashboard panel. `signal` adds a tone-coloured top edge.
pub(crate) fn panel(title: &str, link_html: &str, body_html: &str, signal: Option<&str>) -> String {
    let class = match signal {
        Some(tone) => format!("panel signal tone-{tone}"),
        None => "panel".to_string(),
    };
    format!(
        r#"<section class="{class}"><div class="panel-head">{}{link_html}</div>{body_html}</section>"#,
        section_title(title, None)
    )
}

/// Empty or error state: a title, a hint (trusted markup) and optional actions.
pub(crate) fn empty_state(title_html: &str, hint_html: &str, actions_html: &str) -> String {
    let hint = if hint_html.is_empty() {
        String::new()
    } else {
        format!("<p>{hint_html}</p>")
    };
    let actions = if actions_html.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="empty-actions">{actions_html}</div>"#)
    };
    format!(
        r#"<div class="empty-state"><p class="empty-title">{title_html}</p>{hint}{actions}</div>"#
    )
}

/// One option in a [`view_switch`]: link, icon, label, whether it's the
/// current view, and the single key that switches to it (`b`, `l`, ...).
pub(crate) struct View<'a> {
    pub href: &'a str,
    pub icon: &'a str,
    pub label: &'a str,
    pub active: bool,
    pub key: &'a str,
}

/// Segmented switch between views of the same data (board/list, list/calendar).
pub(crate) fn view_switch(aria_label: &str, views: &[View]) -> String {
    let items: String = views
        .iter()
        .map(|v| {
            let current = if v.active {
                r#" class="active" aria-current="page""#
            } else {
                ""
            };
            let key = if v.key.is_empty() {
                String::new()
            } else {
                format!(r#" data-view-key="{}""#, escape_html(v.key))
            };
            format!(
                r#"<a href="{}"{current}{key}>{}{}</a>"#,
                v.href,
                icon(v.icon),
                escape_html(v.label)
            )
        })
        .collect();
    format!(
        r#"<nav class="view-toggle" aria-label="{}">{items}</nav>"#,
        escape_html(aria_label)
    )
}

/// Segmented board/list switch for the task views.
pub(crate) fn view_toggle(board_active: bool) -> String {
    view_switch(
        "Task view",
        &[
            View {
                href: "/tasks",
                icon: "board",
                label: "Board",
                active: board_active,
                key: "b",
            },
            View {
                href: "/tasks/list",
                icon: "list",
                label: "List",
                active: !board_active,
                key: "l",
            },
        ],
    )
}

/// Segmented list/calendar switch for the meeting views.
pub(crate) fn meeting_view_toggle(calendar_active: bool) -> String {
    view_switch(
        "Meeting view",
        &[
            View {
                href: "/meetings",
                icon: "list",
                label: "List",
                active: !calendar_active,
                key: "l",
            },
            View {
                href: "/meetings/calendar",
                icon: "calendar",
                label: "Calendar",
                active: calendar_active,
                key: "",
            },
        ],
    )
}

/// One labelled select inside a `.filter-group`. Values and labels are escaped.
pub(crate) fn filter_cell(
    name: &str,
    label: &str,
    options: &[(String, String)],
    selected: Option<&str>,
) -> String {
    let set = selected.is_some_and(|s| options.iter().any(|(v, _)| v == s));
    let mut html = format!(
        r#"<label class="filter-cell{}"><span class="filter-label">{label}</span><select name="{name}" data-autosubmit><option value="">All</option>"#,
        if set { " is-set" } else { "" },
    );
    for (value, text) in options {
        let sel = if selected == Some(value.as_str()) {
            " selected"
        } else {
            ""
        };
        html.push_str(&format!(
            r#"<option value="{}"{sel}>{}</option>"#,
            escape_html(value),
            escape_html(text)
        ));
    }
    html.push_str("</select></label>");
    html
}

/// Live text filter over `[data-row]` items inside the element with id `target`.
pub(crate) fn row_filter(target: &str, placeholder: &str) -> String {
    format!(
        r#"<input type="search" class="row-filter" data-filter="{target}" placeholder="{}" aria-label="{}" autocomplete="off">"#,
        escape_html(placeholder),
        escape_html(placeholder)
    )
}

// ── Tables ──────────────────────────────────────────────────────────────

/// Current sort of a table.
pub(crate) struct Sort<'a> {
    pub field: Option<&'a str>,
    pub dir: &'a str,
}

/// A sortable column: its field, the table's current sort, and a function
/// building the link for `(field, dir)`.
pub(crate) type SortLink<'a> = (&'a str, &'a Sort<'a>, &'a dyn Fn(&str, &str) -> String);

/// A header cell. With `sort`, it links to the table sorted by `field`
/// (toggling direction when already active); `href` builds the link from
/// `(field, dir)`.
pub(crate) fn th(label: &str, class: &str, sort: Option<SortLink>) -> String {
    let class_attr = if class.is_empty() {
        String::new()
    } else {
        format!(r#" class="{class}""#)
    };
    let Some((field, current, href)) = sort else {
        return format!("<th{class_attr}>{label}</th>");
    };
    let active = current.field == Some(field);
    let asc = current.dir != "desc";
    let next_dir = if active && asc { "desc" } else { "asc" };
    let (arrow, aria) = match (active, asc) {
        (true, true) => (
            r#"<span class="sort-arrow" aria-hidden="true">▲</span>"#,
            r#" aria-sort="ascending""#,
        ),
        (true, false) => (
            r#"<span class="sort-arrow" aria-hidden="true">▼</span>"#,
            r#" aria-sort="descending""#,
        ),
        _ => ("", ""),
    };
    format!(
        r#"<th{class_attr}{aria}><a href="{}" class="sort-link">{label}{arrow}</a></th>"#,
        href(field, next_dir)
    )
}

/// A data table inside its scroll wrapper. Rows go in `tbody#rows` so the
/// row filter can find them. `extra_class` is added to the table.
pub(crate) fn data_table(extra_class: &str, head_html: &str, rows_html: &str) -> String {
    format!(
        r#"<div class="table-wrap"><table class="data-table {extra_class}"><thead><tr>{head_html}</tr></thead><tbody id="rows">{rows_html}</tbody></table></div>"#
    )
}

/// "Showing 61 of 74" under a filterable table.
pub(crate) fn table_count(shown: usize, total: usize) -> String {
    format!(
        r#"<p class="table-count">Showing <span data-filter-count>{shown}</span> of {total}</p>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_tone_groups_known_statuses() {
        assert_eq!(status_tone("done"), "positive");
        assert_eq!(status_tone("in-progress"), "progress");
        assert_eq!(status_tone("prospect"), "pending");
        assert_eq!(status_tone("churned"), "negative");
        assert_eq!(status_tone("backlog"), "neutral");
        assert_eq!(status_tone("something-custom"), "neutral");
    }

    #[test]
    fn status_badge_keeps_legacy_class() {
        let html = status_badge("in-progress");
        assert!(html.contains("badge-in-progress"));
        assert!(html.contains("tone-progress"));
        assert!(html.contains(">In progress<"));
        assert!(status_badge_lg("done").contains("badge-lg"));
        assert_eq!(status_badge(""), "");
    }

    #[test]
    fn status_badge_escapes_custom_values() {
        let html = status_badge("<x>\"");
        assert!(html.contains("&lt;x&gt;&quot;"));
        assert!(html.contains("badge--x--"));
    }

    #[test]
    fn id_chip_splits_prefix() {
        assert_eq!(
            id_chip("TASK-068"),
            r#"<span class="entity-id"><span class="id-prefix">TASK-</span>068</span>"#
        );
        assert_eq!(
            id_chip("odd<id>"),
            r#"<span class="entity-id">odd&lt;id&gt;</span>"#
        );
    }

    #[test]
    fn priority_gauge_has_label_and_class() {
        let html = priority_html(2);
        assert!(html.contains("pri-high"));
        assert!(html.contains(r#"<span class="pri-label">High</span>"#));
        assert_eq!(html.matches("<i></i>").count(), 4);
        assert!(priority_compact(1).contains("priority-compact"));
    }

    #[test]
    fn tag_chips_compact_limits_and_escapes() {
        let tags: Vec<String> = ["a b", "<c>", "d", ""]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let html = tag_chips_compact(&tags, Some("/tasks"), 2);
        assert!(html.contains(r#"href="/tasks?tag=a%20b""#));
        assert!(html.contains("&lt;c&gt;"));
        assert!(html.contains(r#"<span class="tag-more" title="d">+1</span>"#));
        assert_eq!(tag_chips(&[], None), "");
    }

    #[test]
    fn avatar_and_owner() {
        assert!(avatar("").contains("avatar-empty"));
        assert!(avatar("Jane Doe").contains(r#"title="Jane Doe">JD<"#));
        assert!(avatar("<b>").contains(r#"title="&lt;b&gt;">&lt;<"#));
        assert_eq!(owner_html("  "), "");
    }

    #[test]
    fn progress_bar_marks_bead_and_behind() {
        let html = progress_bar_planned(1, 4, Some(80));
        assert!(html.contains("--p:25%"));
        assert!(html.contains("is-behind"));
        assert!(html.contains("has-bead"));
        assert!(html.contains("--plan:80%"));
        let html = progress_bar(0, 0);
        assert!(!html.contains("has-bead"));
        assert!(!html.contains("progress-plan"));
        assert!(!progress_bar_planned(3, 4, Some(80)).contains("is-behind"));
        assert!(!progress_bar_planned(0, 0, Some(80)).contains("is-behind"));
    }

    #[test]
    fn due_html_uses_short_text_and_full_title() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let html = due_html("2026-10-09", today);
        assert!(html.contains(r#"class="due due-soon""#));
        assert!(html.contains(r#"title="Due Fri 9 Oct 2026, in 4 days""#));
        assert!(html.contains(">in 4d<"));
        assert_eq!(due_html("<soon>", today), "&lt;soon&gt;");
        assert!(task_due_html("2026-09-01", "done", today).contains("muted"));
        assert_eq!(task_due_html("", "todo", today), "");
    }

    #[test]
    fn filter_cell_marks_active_value() {
        let opts = vec![("a\"".to_string(), "A<".to_string())];
        let html = filter_cell("owner", "Owner", &opts, Some("a\""));
        assert!(html.contains("filter-cell is-set"));
        assert!(html.contains(r#"<option value="a&quot;" selected>A&lt;</option>"#));
        assert!(!filter_cell("owner", "Owner", &opts, Some("zzz")).contains("is-set"));
    }

    #[test]
    fn sortable_header_toggles_direction() {
        let href = |f: &str, d: &str| format!("/x?sort={f}&amp;dir={d}");
        let sort = Sort {
            field: Some("id"),
            dir: "asc",
        };
        let html = th("ID", "col-id", Some(("id", &sort, &href)));
        assert!(html.contains(r#"aria-sort="ascending""#));
        assert!(html.contains("dir=desc"));
        let html = th("Title", "", Some(("name", &sort, &href)));
        assert!(!html.contains("aria-sort"));
        assert!(html.contains("dir=asc"));
        assert_eq!(th("Tags", "", None), "<th>Tags</th>");
    }
}
