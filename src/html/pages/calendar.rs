//! Meeting calendar: a month grid with weeks starting on Monday and ISO week
//! numbers, and an agenda list that replaces the grid on phones. Sprints can
//! be overlaid as bands and open task deadlines as ticks. Server-rendered;
//! the script only adds month keys.

use crate::data::{self, EntityRecord};
use crate::entity::EntityKind;
use crate::frontmatter;
use crate::html::catalog::display_name;
use crate::html::components::{
    clip_text, due_html, empty_state, entity_name_link, icon, link_button, meeting_view_toggle,
    page_header, panel, status_badge, status_lamp, status_tone, Btn,
};
use crate::html::format::{
    due_phrase, entity_href, escape_html, fmt_day, fmt_day_relative, href_with, iso_week,
    parse_date, plural, status_label,
};
use crate::html::layout::layout;
use crate::html::{is_cancelled, is_closed, Page};
use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Weekday};
use serde_yaml::Value;
use std::collections::BTreeMap;

/// Meetings shown per day before the rest fold into "+n more".
const VISIBLE: usize = 3;
/// Deadline ticks shown per day before the rest fold into "+n".
const TICKS: usize = 3;
const CALENDAR_HREF: &str = "/meetings/calendar";
const WEEKDAYS: [(&str, &str); 7] = [
    ("Mon", "Monday"),
    ("Tue", "Tuesday"),
    ("Wed", "Wednesday"),
    ("Thu", "Thursday"),
    ("Fri", "Friday"),
    ("Sat", "Saturday"),
    ("Sun", "Sunday"),
];

/// Query parameters for the calendar page.
#[derive(Default)]
pub struct CalendarQuery<'a> {
    /// `YYYY-MM`; the current month when absent.
    pub month: Option<&'a str>,
    /// Overlay sprints and open task deadlines.
    pub overlays: bool,
}

/// Parse a `YYYY-MM` month parameter into the first day of that month.
pub(crate) fn parse_month(s: &str) -> Option<NaiveDate> {
    let (y, m) = s.trim().split_once('-')?;
    let digits = |p: &str| p.bytes().all(|b| b.is_ascii_digit());
    if y.len() != 4 || !(1..=2).contains(&m.len()) || !digits(y) || !digits(m) {
        return None;
    }
    NaiveDate::from_ymd_opt(y.parse().ok()?, m.parse().ok()?, 1)
}

fn first_of_month(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}

/// The first day of the month `n` months after `first`.
fn add_months(first: NaiveDate, n: i32) -> NaiveDate {
    let idx = first.year() * 12 + first.month0() as i32 + n;
    NaiveDate::from_ymd_opt(idx.div_euclid(12), idx.rem_euclid(12) as u32 + 1, 1).unwrap_or(first)
}

/// Monday on or before the 1st through Sunday on or after the last day.
fn grid_range(month: NaiveDate) -> (NaiveDate, NaiveDate) {
    let start = month - Duration::days(month.weekday().num_days_from_monday() as i64);
    let last = add_months(month, 1) - Duration::days(1);
    let end = last + Duration::days(6 - last.weekday().num_days_from_monday() as i64);
    (start, end)
}

fn same_month(a: NaiveDate, b: NaiveDate) -> bool {
    a.year() == b.year() && a.month() == b.month()
}

fn month_label(d: NaiveDate) -> String {
    d.format("%B %Y").to_string()
}

/// A scalar frontmatter value as text (dates and times may be unquoted).
fn scalar(fm: &Value, key: &str) -> Option<String> {
    match fm.as_mapping()?.get(Value::String(key.to_string()))? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A time of day from the start of `s`: `14:00`, `9:30`, `14.00`, `14:00:00`,
/// or the start of a range like `14:00–15:30`.
fn parse_time(s: &str) -> Option<NaiveTime> {
    let lead: String = s
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ':' || *c == '.')
        .map(|c| if c == '.' { ':' } else { c })
        .collect();
    NaiveTime::parse_from_str(&lead, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(&lead, "%H:%M:%S"))
        .ok()
}

/// When a meeting takes place: `date` (or `start_date`/`start`) as
/// `YYYY-MM-DD`, optionally followed by a time (`2026-10-06T14:00`). A
/// `time` field wins over a time in the date.
pub(crate) fn meeting_when(fm: &Value) -> Option<(NaiveDate, Option<NaiveTime>)> {
    let raw = ["date", "start_date", "start"]
        .iter()
        .find_map(|k| scalar(fm, k).filter(|s| !s.trim().is_empty()))?;
    let raw = raw.trim();
    let (day, rest) = match raw.get(..10) {
        Some(day) if raw.len() > 10 => (day, &raw[10..]),
        _ => (raw, ""),
    };
    let date = parse_date(day)?;
    let time = scalar(fm, "time")
        .and_then(|t| parse_time(&t))
        .or_else(|| parse_time(rest.trim_start_matches(['T', 't', ' '])));
    Some((date, time))
}

struct Meeting<'a> {
    rec: &'a EntityRecord,
    date: NaiveDate,
    time: Option<NaiveTime>,
}

impl Meeting<'_> {
    fn status(&self) -> &str {
        frontmatter::get_str_or(&self.rec.frontmatter, "status", "")
    }
}

/// Dated meetings in day order (untimed first within a day, then by time and
/// title), and meetings without a usable date.
fn collect_meetings<'a>(page: &Page<'a>) -> (Vec<Meeting<'a>>, Vec<&'a EntityRecord>) {
    let mut dated = Vec::new();
    let mut undated = Vec::new();
    for rec in page.catalog.of_kind(EntityKind::Meeting) {
        match meeting_when(&rec.frontmatter) {
            Some((date, time)) => dated.push(Meeting { rec, date, time }),
            None => undated.push(rec),
        }
    }
    dated.sort_by_cached_key(|m| {
        (
            m.date,
            m.time.is_some(),
            m.time,
            display_name(m.rec).to_lowercase(),
        )
    });
    (dated, undated)
}

/// A sprint drawn as a band across the days it runs.
struct Band<'a> {
    rec: &'a EntityRecord,
    start: NaiveDate,
    end: NaiveDate,
    lane: usize,
}

/// Sprints overlapping `[from, to]`, each in the lowest lane free for its span.
fn sprint_bands<'a>(page: &Page<'a>, from: NaiveDate, to: NaiveDate) -> Vec<Band<'a>> {
    let mut sprints: Vec<(NaiveDate, NaiveDate, &EntityRecord)> = page
        .catalog
        .of_kind(EntityKind::Sprint)
        .filter_map(|s| {
            let fm = &s.frontmatter;
            let start = parse_date(frontmatter::get_str_or(fm, "start_date", ""))?;
            let end = parse_date(frontmatter::get_str_or(fm, "end_date", "")).unwrap_or(start);
            (start <= end && start <= to && end >= from).then_some((start, end, s))
        })
        .collect();
    sprints.sort_by(|a, b| a.0.cmp(&b.0).then(a.2.id.cmp(&b.2.id)));
    let mut lane_ends: Vec<NaiveDate> = Vec::new();
    sprints
        .into_iter()
        .map(|(start, end, rec)| {
            let lane = match lane_ends.iter().position(|e| *e < start) {
                Some(i) => {
                    lane_ends[i] = end;
                    i
                }
                None => {
                    lane_ends.push(end);
                    lane_ends.len() - 1
                }
            };
            Band {
                rec,
                start,
                end,
                lane,
            }
        })
        .collect()
}

/// Open tasks due in `[from, to]`, by day, most urgent priority first.
fn deadlines<'a>(
    page: &Page<'a>,
    from: NaiveDate,
    to: NaiveDate,
) -> BTreeMap<NaiveDate, Vec<&'a EntityRecord>> {
    let mut by_day: BTreeMap<NaiveDate, Vec<&EntityRecord>> = BTreeMap::new();
    for task in page.catalog.of_kind(EntityKind::Task) {
        let fm = &task.frontmatter;
        if is_closed(frontmatter::get_str_or(fm, "status", "")) {
            continue;
        }
        if let Some(due) = parse_date(frontmatter::get_str_or(fm, "due_date", "")) {
            if due >= from && due <= to {
                by_day.entry(due).or_default().push(task);
            }
        }
    }
    for tasks in by_day.values_mut() {
        tasks.sort_by_key(|t| {
            (
                data::get_number(&t.frontmatter, "priority").unwrap_or(3),
                &t.id,
            )
        });
    }
    by_day
}

/// Same tones as the flight plan: late, due within a week, later.
fn deadline_tone(due: NaiveDate, today: NaiveDate) -> &'static str {
    match (due - today).num_days() {
        d if d < 0 => "negative",
        0..=7 => "pending",
        _ => "neutral",
    }
}

/// Link to the calendar for `month`, keeping the overlay switch.
fn calendar_href(month: Option<NaiveDate>, overlays: bool) -> String {
    let m = month
        .map(|m| m.format("%Y-%m").to_string())
        .unwrap_or_default();
    href_with(
        CALENDAR_HREF,
        &[("month", &m), ("overlays", if overlays { "1" } else { "" })],
    )
}

/// Render the meeting calendar for the requested month.
pub fn calendar_page(page: &Page, query: &CalendarQuery) -> String {
    let today = page.today;
    let title = "Meeting calendar";
    let total = page
        .catalog
        .count_for(EntityKind::Meeting)
        .map_or(0, |c| c.total);
    let mut body = page_header(
        "Meetings",
        &format!("{total} total"),
        &meeting_view_toggle(true),
    );
    if total == 0 {
        body.push_str(&empty_state(
            "No meetings yet.",
            r#"Create one with <code>mc new meeting "Title"</code>. Meetings with a <code>date</code> show up on the calendar."#,
            "",
        ));
        return layout(page, title, "/meetings", "", &body);
    }

    let parsed = query.month.map(|raw| (raw, parse_month(raw)));
    let month = match parsed {
        Some((_, Some(m))) => m,
        _ => first_of_month(today),
    };
    if let Some((raw, None)) = parsed {
        body.push_str(&format!(
            r#"<p class="cal-notice" role="status">{}<span>Couldn’t read the month “{}”. Use <code>YYYY-MM</code>, like <code>{}</code>. Showing this month instead.</span></p>"#,
            status_lamp("on-hold"),
            escape_html(raw),
            today.format("%Y-%m"),
        ));
    }

    let (meetings, undated) = collect_meetings(page);
    let (grid_start, grid_end) = grid_range(month);
    let in_month: Vec<&Meeting> = meetings
        .iter()
        .filter(|m| same_month(m.date, month))
        .collect();
    let mut by_day: BTreeMap<NaiveDate, Vec<&Meeting>> = BTreeMap::new();
    for m in meetings
        .iter()
        .filter(|m| m.date >= grid_start && m.date <= grid_end)
    {
        by_day.entry(m.date).or_default().push(m);
    }

    let overlays_available = page.cfg.entity_available(&EntityKind::Sprint)
        || page.cfg.entity_available(&EntityKind::Task);
    let overlays = query.overlays && overlays_available;
    let bands = if overlays {
        sprint_bands(page, grid_start, grid_end)
    } else {
        Vec::new()
    };
    let ticks = if overlays {
        deadlines(page, grid_start, grid_end)
    } else {
        BTreeMap::new()
    };

    // Nearest months with meetings, for the empty state.
    let earlier = meetings
        .iter()
        .rev()
        .find(|m| m.date < month)
        .map(|m| first_of_month(m.date));
    let later = meetings
        .iter()
        .find(|m| m.date >= add_months(month, 1))
        .map(|m| first_of_month(m.date));
    let jumps = |class: &str| -> String {
        let mut html = String::new();
        if let Some(m) = earlier {
            html.push_str(&link_button(
                &calendar_href(Some(m), overlays),
                &format!("← {}", month_label(m)),
                Btn::Secondary,
                class,
            ));
        }
        if let Some(m) = later {
            html.push_str(&link_button(
                &calendar_href(Some(m), overlays),
                &format!("{} →", month_label(m)),
                Btn::Secondary,
                class,
            ));
        }
        html
    };

    body.push_str(&toolbar(
        page,
        month,
        in_month.len(),
        overlays,
        overlays_available,
        &jumps("btn-sm"),
    ));

    let mut grid = String::from(
        r#"<div class="cal-wrap"><table class="cal" aria-labelledby="cal-month"><thead><tr><th class="cal-wk" scope="col"><abbr title="ISO week">Wk</abbr></th>"#,
    );
    for (i, (short, long)) in WEEKDAYS.iter().enumerate() {
        let weekend = if i >= 5 { r#" class="is-weekend""# } else { "" };
        grid.push_str(&format!(
            r#"<th scope="col"{weekend}><abbr title="{long}">{short}</abbr></th>"#
        ));
    }
    grid.push_str("</tr></thead><tbody>");
    let mut monday = grid_start;
    while monday <= grid_end {
        // Bands keep their lane across the week; weeks without sprints get none.
        let sunday = monday + Duration::days(6);
        let lanes = bands
            .iter()
            .filter(|b| b.start <= sunday && b.end >= monday)
            .map(|b| b.lane + 1)
            .max()
            .unwrap_or(0);
        let week = iso_week(monday);
        grid.push_str(&format!(
            r#"<tr><th class="cal-wk" scope="row"><span title="ISO week {}">{week}</span></th>"#,
            monday.iso_week().week()
        ));
        for i in 0..7 {
            let d = monday + Duration::days(i);
            let events = by_day.get(&d).map(Vec::as_slice).unwrap_or_default();
            let day_ticks = ticks.get(&d).map(Vec::as_slice).unwrap_or_default();
            grid.push_str(&day_cell(page, d, month, events, &bands, lanes, day_ticks));
        }
        grid.push_str("</tr>");
        monday += Duration::days(7);
    }
    grid.push_str("</tbody></table></div>");
    body.push_str(&grid);
    if overlays {
        body.push_str(&overlay_legend(
            !bands.is_empty() || page.cfg.entity_available(&EntityKind::Sprint),
        ));
    }

    body.push_str(r#"<div class="cal-agenda">"#);
    if overlays {
        body.push_str(&agenda_overlays(page, month, &bands, &ticks));
    }
    body.push_str(&agenda(page, month, &in_month, &jumps("")));
    body.push_str("</div>");

    if !undated.is_empty() {
        body.push_str(&undated_panel(&undated));
    }

    layout(page, title, "/meetings", "", &body)
}

/// Month heading, summary, navigation and the overlay switch.
fn toolbar(
    page: &Page,
    month: NaiveDate,
    count: usize,
    overlays: bool,
    overlays_available: bool,
    jumps_html: &str,
) -> String {
    let today = page.today;
    let prev = add_months(month, -1);
    let next = add_months(month, 1);
    let summary = if count == 0 {
        format!(
            r#"<p class="cal-summary is-empty"><span>No meetings this month.</span>{jumps_html}</p>"#
        )
    } else {
        format!(
            r#"<p class="cal-summary">{}</p>"#,
            plural(count, "meeting", "meetings")
        )
    };
    let step = |target: NaiveDate, rel: &str, label: &str, key: &str| {
        let flip = if rel == "prev" { " icon-flip" } else { "" };
        format!(
            r#"<a class="icon-btn cal-step{flip}" href="{}" rel="{rel}" data-cal-{rel} aria-label="{label}, {}" title="{label} ({key})">{}</a>"#,
            calendar_href(Some(target), overlays),
            month_label(target),
            icon("chevron"),
        )
    };
    let this_month = same_month(month, today);
    let today_btn = format!(
        r#"<a class="btn btn-sm cal-today{}" href="{}" data-cal-today title="This month (.)"{}>Today</a>"#,
        if this_month { " is-current" } else { "" },
        calendar_href(None, overlays),
        if this_month {
            r#" aria-current="date""#
        } else {
            ""
        },
    );
    let overlay_switch = if overlays_available {
        let (href, label) = if overlays {
            (
                calendar_href(Some(month), false),
                "Hide sprints and task deadlines",
            )
        } else {
            (
                calendar_href(Some(month), true),
                "Show sprints and task deadlines",
            )
        };
        format!(
            r#"<a class="chip cal-overlay-switch{}" href="{href}" title="{label}" aria-label="{label}"><span class="cal-switch-box" aria-hidden="true"></span>Sprints &amp; deadlines</a>"#,
            if overlays { " chip-active" } else { "" },
        )
    } else {
        String::new()
    };
    format!(
        r#"<div class="cal-toolbar"><div class="cal-heading"><h2 class="cal-month" id="cal-month">{}</h2>{summary}</div><div class="cal-controls">{overlay_switch}<nav class="cal-nav" aria-label="Change month">{}{today_btn}{}</nav></div></div>"#,
        month_label(month),
        step(prev, "prev", "Previous month", "←"),
        step(next, "next", "Next month", "→"),
    )
}

fn day_cell(
    page: &Page,
    d: NaiveDate,
    month: NaiveDate,
    events: &[&Meeting],
    bands: &[Band],
    lanes: usize,
    ticks: &[&EntityRecord],
) -> String {
    let today = page.today;
    let mut class = String::from("cal-day");
    if !same_month(d, month) {
        class.push_str(" is-out");
    }
    if matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
        class.push_str(" is-weekend");
    }
    if d < today {
        class.push_str(" is-past");
    }
    let is_today = d == today;
    if is_today {
        class.push_str(" is-today");
    }
    if !events.is_empty() {
        class.push_str(" has-events");
    }

    let num = if d.day() == 1 {
        d.format("%-d %b").to_string()
    } else {
        d.day().to_string()
    };
    let mut html = format!(
        r#"<td class="{class}"{}><div class="cal-day-head"><time class="cal-num" datetime="{}" title="{}">{num}</time>"#,
        if is_today {
            r#" aria-current="date""#
        } else {
            ""
        },
        d.format("%Y-%m-%d"),
        d.format("%A %-d %B %Y"),
    );
    if is_today {
        html.push_str(r#"<span class="cal-today-label">Today</span>"#);
    }
    if !ticks.is_empty() {
        html.push_str(r#"<span class="cal-ticks">"#);
        let tone = deadline_tone(d, today);
        let (phrase, _) = due_phrase(d, today);
        let phrase = lower_first(&phrase);
        // A full row has room for TICKS marks; the rest fold into "+n".
        let shown = if ticks.len() > TICKS {
            TICKS - 1
        } else {
            ticks.len()
        };
        for t in &ticks[..shown] {
            let label = format!("{} {}, {phrase}", t.id, display_name(t));
            html.push_str(&format!(
                r#"<a class="cal-tick tone-{tone}" href="{}" title="{}" aria-label="{}"></a>"#,
                entity_href(&t.id),
                escape_html(&label),
                escape_html(&label),
            ));
        }
        if ticks.len() > shown {
            let rest = &ticks[shown..];
            let noun = if rest.len() == 1 { "task" } else { "tasks" };
            let label = format!("{} more {noun}, {phrase}", rest.len());
            html.push_str(&format!(
                r#"<details class="cal-tick-more"><summary title="{}" aria-label="{}">+{}</summary><ul class="cal-tick-list">"#,
                escape_html(&label),
                escape_html(&label),
                rest.len(),
            ));
            for t in rest {
                html.push_str(&format!(
                    r#"<li><a class="tone-{tone}" href="{}" title="{}"><span class="cal-tick-bar" aria-hidden="true"></span><span class="cal-tick-id">{}</span><span class="cal-tick-name">{}</span></a></li>"#,
                    entity_href(&t.id),
                    escape_html(display_name(t)),
                    escape_html(&t.id),
                    escape_html(display_name(t)),
                ));
            }
            html.push_str("</ul></details>");
        }
        html.push_str("</span>");
    }
    html.push_str("</div>");

    if lanes > 0 {
        html.push_str(r#"<div class="cal-bands">"#);
        for lane in 0..lanes {
            let band = bands
                .iter()
                .find(|b| b.lane == lane && b.start <= d && b.end >= d);
            let Some(b) = band else {
                html.push_str(r#"<span class="cal-band is-empty"></span>"#);
                continue;
            };
            let mut bclass = String::new();
            if b.start == d {
                bclass.push_str(" is-start");
            }
            if b.end == d {
                bclass.push_str(" is-end");
            }
            // Name the sprint where it starts and at the start of each week row.
            let label = if b.start == d || d.weekday() == Weekday::Mon {
                format!(
                    r#"<span class="cal-band-name">{}</span>"#,
                    escape_html(display_name(b.rec))
                )
            } else {
                String::new()
            };
            let status = frontmatter::get_str_or(&b.rec.frontmatter, "status", "");
            let tip = format!(
                "Sprint {}: {} – {}",
                display_name(b.rec),
                fmt_day(b.start, today),
                fmt_day(b.end, today)
            );
            html.push_str(&format!(
                r#"<a class="cal-band tone-{}{bclass}" href="{}" title="{}" tabindex="-1">{label}</a>"#,
                status_tone(status),
                entity_href(&b.rec.id),
                escape_html(&tip),
            ));
        }
        html.push_str("</div>");
    }

    if !events.is_empty() {
        let shown = if events.len() > VISIBLE {
            VISIBLE - 1
        } else {
            events.len()
        };
        html.push_str(r#"<ul class="cal-events">"#);
        for m in &events[..shown] {
            html.push_str(&chip(m));
        }
        html.push_str("</ul>");
        if events.len() > shown {
            let rest = &events[shown..];
            let names: Vec<&str> = rest.iter().map(|m| display_name(m.rec)).collect();
            html.push_str(&format!(
                r#"<details class="cal-more"><summary title="{}">+{} more</summary><ul class="cal-events">"#,
                escape_html(&names.join(", ")),
                rest.len()
            ));
            for m in rest {
                html.push_str(&chip(m));
            }
            html.push_str("</ul></details>");
        }
    }
    html.push_str("</td>");
    html
}

/// One meeting in the grid: status lamp, time and title.
fn chip(m: &Meeting) -> String {
    let status = m.status();
    let name = display_name(m.rec);
    let time = m.time.map(|t| t.format("%H:%M").to_string());
    let mut tip = String::new();
    if let Some(t) = &time {
        tip.push_str(t);
        tip.push(' ');
    }
    tip.push_str(name);
    if !status.is_empty() {
        tip.push_str(&format!(" ({})", status_label(status)));
    }
    let time_html = time
        .map(|t| format!(r#"<time class="cal-time">{t}</time>"#))
        .unwrap_or_default();
    format!(
        r#"<li><a class="cal-chip tone-{}{}" href="{}" title="{}"><span class="cal-chip-lead">{}{time_html}</span> <span class="cal-chip-title">{}</span></a></li>"#,
        status_tone(status),
        if is_cancelled(status) {
            " is-cancelled"
        } else {
            ""
        },
        entity_href(&m.rec.id),
        escape_html(&tip),
        if status.is_empty() {
            String::new()
        } else {
            status_lamp(status)
        },
        escape_html(name),
    )
}

/// Key for the overlay marks.
fn overlay_legend(sprints: bool) -> String {
    let tick = |tone: &str, label: &str| {
        format!(
            r#"<li><span class="fp-key-tick tone-{tone}" aria-hidden="true"></span>{label}</li>"#
        )
    };
    let sprint = if sprints {
        r#"<li><span class="cal-band-key" aria-hidden="true"></span>Sprint</li>"#
    } else {
        ""
    };
    format!(
        r#"<ul class="fp-legend cal-legend" aria-label="Legend">{sprint}{}{}{}</ul>"#,
        tick("negative", "Task overdue"),
        tick("pending", "Due within a week"),
        tick("neutral", "Due later"),
    )
}

/// The month as a list of days with meetings; replaces the grid on phones.
fn agenda(page: &Page, month: NaiveDate, meetings: &[&Meeting], jumps_html: &str) -> String {
    let today = page.today;
    if meetings.is_empty() {
        return empty_state(
            &format!("No meetings in {}", month_label(month)),
            "",
            jumps_html,
        );
    }
    let mut html = String::new();
    let mut i = 0;
    while i < meetings.len() {
        let d = meetings[i].date;
        let day: Vec<&&Meeting> = meetings[i..].iter().take_while(|m| m.date == d).collect();
        i += day.len();
        let is_today = d == today;
        html.push_str(&format!(
            r#"<section class="cal-agenda-day{}"><h3 class="cal-agenda-date"><time datetime="{}">{}</time><span class="cal-agenda-week">{}</span></h3><ul class="item-list">"#,
            if is_today { " is-today" } else { "" },
            d.format("%Y-%m-%d"),
            fmt_day_relative(d, today),
            iso_week(d),
        ));
        for m in day {
            let fm = &m.rec.frontmatter;
            let time = match m.time {
                Some(t) => format!(r#"<time>{}</time>"#, t.format("%H:%M")),
                None => r#"<span>No time</span>"#.to_string(),
            };
            let duration = frontmatter::get_str_or(fm, "duration", "");
            let duration = if m.time.is_some() && !duration.is_empty() {
                format!("<span>{}</span>", escape_html(duration))
            } else {
                String::new()
            };
            let customers = page
                .catalog
                .refs_html_compact(&frontmatter::get_string_list(fm, "customers"));
            html.push_str(&format!(
                r#"<li class="item{}"><span class="item-lead when">{time}{duration}</span><div class="item-main">{}<div class="item-meta">{}{customers}</div></div></li>"#,
                if is_cancelled(m.status()) {
                    " is-cancelled"
                } else {
                    ""
                },
                entity_name_link(m.rec),
                status_badge(m.status()),
            ));
        }
        html.push_str("</ul></section>");
    }
    html
}

/// The overlay on phones, where the grid is hidden: the month's sprints and
/// open task deadlines as one list above the agenda.
fn agenda_overlays(
    page: &Page,
    month: NaiveDate,
    bands: &[Band],
    ticks: &BTreeMap<NaiveDate, Vec<&EntityRecord>>,
) -> String {
    let today = page.today;
    let last = add_months(month, 1) - Duration::days(1);
    let mut items = String::new();
    let mut count = 0;
    for b in bands.iter().filter(|b| b.start <= last && b.end >= month) {
        count += 1;
        let status = frontmatter::get_str_or(&b.rec.frontmatter, "status", "");
        items.push_str(&format!(
            r#"<li class="item"><span class="item-lead when"><span>Sprint</span></span><div class="item-main">{}<div class="item-meta"><span>{} – {}</span>{}</div></div></li>"#,
            entity_name_link(b.rec),
            fmt_day(b.start, today),
            fmt_day(b.end, today),
            status_badge(status),
        ));
    }
    for (due, tasks) in ticks.range(month..=last) {
        for t in tasks {
            count += 1;
            items.push_str(&format!(
                r#"<li class="item"><span class="item-lead">{}</span><div class="item-main">{}<div class="item-meta">{}</div></div></li>"#,
                due_html(&due.format("%Y-%m-%d").to_string(), today),
                entity_name_link(t),
                status_badge(frontmatter::get_str_or(&t.frontmatter, "status", "")),
            ));
        }
    }
    let body = if items.is_empty() {
        r#"<p class="cal-agenda-none">No sprints or open deadlines this month.</p>"#.to_string()
    } else {
        format!(r#"<ul class="item-list">{items}</ul>"#)
    };
    format!(
        r#"<section class="cal-agenda-day cal-agenda-overlay"><h3 class="cal-agenda-date"><span>Sprints &amp; deadlines</span><span class="cal-agenda-week">{count}</span></h3>{body}</section>"#
    )
}

/// `s` with its first letter lower-cased, for use mid-sentence ("due 31 Oct").
fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_lowercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// Meetings the calendar can't place, so they don't silently disappear.
fn undated_panel(undated: &[&EntityRecord]) -> String {
    let mut body = String::from(
        r#"<p class="cal-undated-hint">Give these a <code>date</code> as <code>YYYY-MM-DD</code> to place them on the calendar.</p><ul class="item-list">"#,
    );
    for rec in undated {
        let raw = scalar(&rec.frontmatter, "date").unwrap_or_default();
        let lead = if raw.trim().is_empty() {
            r#"<span class="muted">No date</span>"#.to_string()
        } else {
            clip_text(raw.trim())
        };
        body.push_str(&format!(
            r#"<li class="item"><span class="item-lead cal-undated-date">{lead}</span><div class="item-main">{}<div class="item-meta">{}</div></div></li>"#,
            entity_name_link(rec),
            status_badge(frontmatter::get_str_or(&rec.frontmatter, "status", "")),
        ));
    }
    body.push_str("</ul>");
    format!(
        r#"<div class="section cal-undated">{}</div>"#,
        panel(
            &format!("Undated ({})", undated.len()),
            "",
            &body,
            Some("pending")
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn parse_month_accepts_year_month_only() {
        assert_eq!(parse_month("2026-10"), Some(d(2026, 10, 1)));
        assert_eq!(parse_month(" 2026-3 "), Some(d(2026, 3, 1)));
        for bad in [
            "2026-13",
            "2026-00",
            "26-10",
            "2026",
            "2026-10-01",
            "x-y",
            "",
            "2026-1a",
        ] {
            assert_eq!(parse_month(bad), None, "{bad}");
        }
    }

    #[test]
    fn months_wrap_across_years() {
        assert_eq!(add_months(d(2026, 12, 1), 1), d(2027, 1, 1));
        assert_eq!(add_months(d(2026, 1, 1), -1), d(2025, 12, 1));
        assert_eq!(add_months(d(2026, 10, 1), -22), d(2024, 12, 1));
    }

    #[test]
    fn grid_starts_on_monday_and_ends_on_sunday() {
        // October 2026 starts on a Thursday and ends on a Saturday.
        let (start, end) = grid_range(d(2026, 10, 1));
        assert_eq!(start, d(2026, 9, 28));
        assert_eq!(start.weekday(), Weekday::Mon);
        assert_eq!(end, d(2026, 11, 1));
        assert_eq!(end.weekday(), Weekday::Sun);
        // February 2027 starts on a Monday: exactly four weeks.
        let (start, end) = grid_range(d(2027, 2, 1));
        assert_eq!((start, end), (d(2027, 2, 1), d(2027, 2, 28)));
    }

    #[test]
    fn meeting_when_reads_date_and_time_variants() {
        let fm = |y: &str| serde_yaml::from_str::<Value>(y).unwrap();
        assert_eq!(
            meeting_when(&fm("date: 2026-10-06\ntime: \"14:00\"")),
            Some((d(2026, 10, 6), NaiveTime::from_hms_opt(14, 0, 0)))
        );
        assert_eq!(
            meeting_when(&fm("date: 2026-10-06T09:30:00")),
            Some((d(2026, 10, 6), NaiveTime::from_hms_opt(9, 30, 0)))
        );
        assert_eq!(
            meeting_when(&fm("date: 2026-10-06\ntime: 9.15–10.00")),
            Some((d(2026, 10, 6), NaiveTime::from_hms_opt(9, 15, 0)))
        );
        assert_eq!(
            meeting_when(&fm("start: 2026-10-06\ntime: soon")),
            Some((d(2026, 10, 6), None))
        );
        assert_eq!(meeting_when(&fm("date: next week")), None);
        assert_eq!(meeting_when(&fm("date: 2026-02-30")), None);
        assert_eq!(meeting_when(&fm("title: x")), None);
    }

    fn page_html(records: Vec<EntityRecord>, today: NaiveDate, q: &CalendarQuery) -> String {
        let (_d, cfg) = test_config();
        let cat = catalog(records, &cfg);
        let mut page = Page::new(&cfg, &cat, "");
        page.today = today;
        calendar_page(&page, q)
    }

    #[test]
    fn meetings_land_on_their_day_in_time_order() {
        let html = page_html(
            vec![
                rec(
                    EntityKind::Meeting,
                    "MTG-002",
                    "title: Late\ndate: 2026-10-15\ntime: \"16:00\"\nstatus: scheduled",
                ),
                rec(
                    EntityKind::Meeting,
                    "MTG-001",
                    "title: Early <b>\ndate: 2026-10-15\ntime: \"9:00\"\nstatus: completed",
                ),
                rec(
                    EntityKind::Meeting,
                    "MTG-003",
                    "title: Elsewhere\ndate: 2026-12-01",
                ),
            ],
            d(2026, 10, 6),
            &CalendarQuery {
                month: Some("2026-10"),
                overlays: false,
            },
        );
        let cell = html.split(r#"datetime="2026-10-15""#).nth(1).unwrap();
        let cell = &cell[..cell.find("</td>").unwrap()];
        let early = cell.find("/entity/MTG-001").unwrap();
        let late = cell.find("/entity/MTG-002").unwrap();
        assert!(early < late);
        assert!(cell.contains(r#"<time class="cal-time">09:00</time>"#));
        assert!(cell.contains("Early &lt;b&gt;"));
        assert!(
            !html.contains("/entity/MTG-003\""),
            "December meeting not in October grid"
        );
        assert!(html.contains(r#"<h2 class="cal-month" id="cal-month">October 2026</h2>"#));
        assert!(html.contains("2 meetings"));
        // Today in the magenta grammar, and the header row starts on Monday.
        assert!(html.contains(r#"class="cal-day is-today" aria-current="date""#));
        let head = &html[html.find("<thead>").unwrap()..html.find("</thead>").unwrap()];
        assert!(head.find("Mon").unwrap() < head.find("Sun").unwrap());
        assert!(html.contains(r#"<span title="ISO week 40">W40</span>"#));
        assert!(
            html.contains(r#"href="/meetings/calendar?month=2026-09" rel="prev" data-cal-prev"#)
        );
        assert!(
            html.contains(r#"href="/meetings/calendar?month=2026-11" rel="next" data-cal-next"#)
        );
    }

    #[test]
    fn busy_days_fold_into_more() {
        let records = (1..=5)
            .map(|i| {
                rec(
                    EntityKind::Meeting,
                    &format!("MTG-00{i}"),
                    &format!("title: M{i}\ndate: 2026-10-07\ntime: \"1{i}:00\""),
                )
            })
            .collect();
        let html = page_html(records, d(2026, 10, 6), &CalendarQuery::default());
        assert!(html.contains(
            r#"<details class="cal-more"><summary title="M3, M4, M5">+3 more</summary>"#
        ));
    }

    #[test]
    fn invalid_month_and_undated_meetings_are_reported() {
        let html = page_html(
            vec![
                rec(EntityKind::Meeting, "MTG-001", "title: Someday\ndate: tbd"),
                rec(
                    EntityKind::Meeting,
                    "MTG-002",
                    "title: Dated\ndate: 2026-06-10",
                ),
            ],
            d(2026, 10, 6),
            &CalendarQuery {
                month: Some("2026-<13>"),
                overlays: false,
            },
        );
        assert!(html.contains("Couldn’t read the month “2026-&lt;13&gt;”"));
        assert!(html.contains("October 2026"));
        assert!(html.contains("Undated (1)"));
        assert!(html.contains(r#"title="tbd">tbd</span>"#));
        // Empty month points at the nearest month with meetings.
        assert!(html.contains("No meetings this month."));
        assert!(html.contains(r#"href="/meetings/calendar?month=2026-06">← June 2026</a>"#));
    }

    #[test]
    fn overlays_draw_sprints_and_open_deadlines() {
        let records = vec![
            rec(EntityKind::Meeting, "MTG-001", "title: M\ndate: 2026-10-07"),
            rec(
                EntityKind::Sprint,
                "SPR-001",
                "title: Alpha\nstatus: active\nstart_date: 2026-10-05\nend_date: 2026-10-16",
            ),
            rec(
                EntityKind::Task,
                "TASK-001",
                "title: Late\nstatus: todo\ndue_date: 2026-10-02",
            ),
            rec(
                EntityKind::Task,
                "TASK-002",
                "title: Done\nstatus: done\ndue_date: 2026-10-02",
            ),
        ];
        let off = page_html(records.clone(), d(2026, 10, 6), &CalendarQuery::default());
        assert!(!off.contains(r#"class="cal-band"#));
        assert!(off.contains(r#"href="/meetings/calendar?month=2026-10&amp;overlays=1""#));
        let on = page_html(
            records,
            d(2026, 10, 6),
            &CalendarQuery {
                month: None,
                overlays: true,
            },
        );
        assert!(on.contains(r#"class="cal-band tone-positive is-start" href="/entity/SPR-001""#));
        assert!(on.contains(r#"class="cal-band tone-positive is-end""#));
        assert!(on.contains(r#"class="cal-tick tone-negative" href="/entity/TASK-001""#));
        assert!(!on.contains("/entity/TASK-002"));
        // Navigation keeps the overlays on.
        assert!(on.contains("?month=2026-11&amp;overlays=1"));
        // Phones get the overlay as a list above the agenda.
        assert!(on.contains(r#"<section class="cal-agenda-day cal-agenda-overlay">"#));
        let overlay = &on[on.find("cal-agenda-overlay").unwrap()..];
        assert!(
            overlay.contains(r#"href="/entity/SPR-001""#)
                && overlay.contains(r#"href="/entity/TASK-001""#)
        );
    }

    #[test]
    fn deadline_labels_and_overflow_stay_reachable() {
        let mut records = vec![rec(
            EntityKind::Meeting,
            "MTG-001",
            "title: M\ndate: 2026-10-07",
        )];
        for i in 1..=5 {
            records.push(rec(
                EntityKind::Task,
                &format!("TASK-00{i}"),
                &format!("title: T{i}\nstatus: todo\ndue_date: 2026-10-31"),
            ));
        }
        let html = page_html(
            records,
            d(2026, 10, 6),
            &CalendarQuery {
                month: None,
                overlays: true,
            },
        );
        assert!(
            html.contains(r#"title="TASK-001 T1, due 31 Oct""#),
            "{html}"
        );
        assert_eq!(html.matches(r#"class="cal-tick tone-neutral""#).count(), 2);
        assert!(html.contains(r#"aria-label="3 more tasks, due 31 Oct">+3</summary>"#));
        let list = &html[html.find("cal-tick-list").unwrap()..];
        for id in ["TASK-003", "TASK-004", "TASK-005"] {
            assert!(list.contains(&format!(r#"href="/entity/{id}""#)), "{id}");
        }
        assert_eq!(lower_first("Due today"), "due today");
        assert_eq!(lower_first("3 days overdue"), "3 days overdue");
    }

    #[test]
    fn no_meetings_at_all_shows_create_hint() {
        let html = page_html(Vec::new(), d(2026, 10, 6), &CalendarQuery::default());
        assert!(html.contains("No meetings yet."));
        assert!(!html.contains(r#"<table class="cal""#));
    }
}
