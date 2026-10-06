//! The flight plan: open task deadlines and meetings on one six-week time
//! axis, with a magenta line for today. Server-rendered, no JS needed.

use crate::data::{self, EntityRecord};
use crate::entity::EntityKind;
use crate::frontmatter;
use crate::html::catalog::display_name;
use crate::html::format::{
    due_phrase, entity_href, escape_html, fmt_day, iso_week, parse_date, plural,
};
use crate::html::{is_cancelled, is_closed, Page};
use chrono::{Datelike, Duration, NaiveDate, Weekday};
use std::collections::BTreeMap;

/// Days shown, starting a week before today.
const DAYS: i64 = 49;
const DAYS_BEFORE: i64 = 7;
/// Tallest stack of ticks per day, in px, before the rest collapse into `+n`.
const STACK_MAX: u32 = 52;
const TICK_GAP: u32 = 2;
const WAYPOINTS_PER_DAY: usize = 2;

/// Horizontal centre of day `i` in percent of the track.
fn x_pct(i: i64) -> f64 {
    (i as f64 + 0.5) / DAYS as f64 * 100.0
}

/// Left edge of day `i` in percent of the track.
fn edge_pct(i: i64) -> f64 {
    i as f64 / DAYS as f64 * 100.0
}

/// Sweep delay factor for day `i` (0..1).
fn sweep_t(i: i64) -> f64 {
    i as f64 / (DAYS - 1) as f64
}

fn tick_height(priority: u32) -> u32 {
    match priority {
        1 => 22,
        2 => 16,
        3 => 11,
        _ => 7,
    }
}

fn tick_tone(due: NaiveDate, today: NaiveDate) -> &'static str {
    let days = (due - today).num_days();
    if days < 0 {
        "negative"
    } else if days <= 7 {
        "pending"
    } else {
        "neutral"
    }
}

/// Stack heights bottom-up with a gap. Returns the offset of each tick that
/// fits, the number that don't, and the offset where the next one would go.
fn stack(heights: &[u32]) -> (Vec<u32>, usize, u32) {
    let mut y = 0;
    let mut placed = Vec::new();
    for (i, h) in heights.iter().enumerate() {
        if y + h > STACK_MAX {
            return (placed, heights.len() - i, y);
        }
        placed.push(y);
        y += h + TICK_GAP;
    }
    (placed, 0, y)
}

struct Tick<'a> {
    due: NaiveDate,
    priority: u32,
    task: &'a EntityRecord,
}

/// Render the flight plan section for the dashboard.
pub(crate) fn flight_plan(page: &Page) -> String {
    let today = page.today;
    let start = today - Duration::days(DAYS_BEFORE);
    let end = start + Duration::days(DAYS);
    let day_of = |d: NaiveDate| (d - start).num_days();
    let catalog = page.catalog;

    // Tasks
    let mut older = 0;
    let mut overdue = 0;
    let mut due_soon = 0;
    let mut by_day: BTreeMap<i64, Vec<Tick>> = BTreeMap::new();
    for task in catalog.of_kind(EntityKind::Task) {
        if is_closed(frontmatter::get_str_or(&task.frontmatter, "status", "")) {
            continue;
        }
        let Some(due) = parse_date(frontmatter::get_str_or(&task.frontmatter, "due_date", ""))
        else {
            continue;
        };
        let days = (due - today).num_days();
        if days < 0 {
            overdue += 1;
        } else if days <= 7 {
            due_soon += 1;
        }
        if due < start {
            older += 1;
        } else if due < end {
            by_day.entry(day_of(due)).or_default().push(Tick {
                due,
                priority: data::get_number(&task.frontmatter, "priority").unwrap_or(3),
                task,
            });
        }
    }

    let mut marks = String::new();
    for (day, mut ticks) in by_day {
        ticks.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.task.id.cmp(&b.task.id)));
        let heights: Vec<u32> = ticks.iter().map(|t| tick_height(t.priority)).collect();
        let (offsets, overflow, next_y) = stack(&heights);
        for (t, y) in ticks.iter().zip(offsets) {
            let (phrase, _) = due_phrase(t.due, today);
            marks.push_str(&format!(
                r#"<a class="fp-tick tone-{tone}" style="--x:{x:.3}%;--h:{h}px;--y:{y}px;--t:{tt:.3}" href="{href}" title="{title}" tabindex="-1"></a>"#,
                tone = tick_tone(t.due, today),
                x = x_pct(day),
                h = tick_height(t.priority),
                tt = sweep_t(day),
                href = entity_href(&t.task.id),
                title = escape_html(&format!(
                    "{} {}, {}",
                    t.task.id,
                    display_name(t.task),
                    lower_first(&phrase)
                )),
            ));
        }
        if overflow > 0 {
            marks.push_str(&format!(
                r#"<span class="fp-more" style="--x:{:.3}%;--y:{next_y}px">+{overflow}</span>"#,
                x_pct(day)
            ));
        }
    }

    // Meetings
    let has_meetings = page.cfg.entity_available(&EntityKind::Meeting);
    let mut meetings_ahead = 0;
    let mut wp_by_day: BTreeMap<i64, Vec<(&str, &EntityRecord)>> = BTreeMap::new();
    for m in catalog.of_kind(EntityKind::Meeting) {
        if is_cancelled(frontmatter::get_str_or(&m.frontmatter, "status", "")) {
            continue;
        }
        let Some(date) = parse_date(frontmatter::get_str_or(&m.frontmatter, "date", "")) else {
            continue;
        };
        if date < start || date >= end {
            continue;
        }
        if date >= today {
            meetings_ahead += 1;
        }
        let time = frontmatter::get_str_or(&m.frontmatter, "time", "");
        wp_by_day.entry(day_of(date)).or_default().push((time, m));
    }
    for (day, mut list) in wp_by_day {
        list.sort_by(|a, b| a.0.cmp(b.0).then(a.1.id.cmp(&b.1.id)));
        let date = start + Duration::days(day);
        for (i, (time, m)) in list.iter().take(WAYPOINTS_PER_DAY).enumerate() {
            let dx = if i == 0 {
                String::new()
            } else {
                ";--dx:6px".into()
            };
            let when = format!("{} {}", fmt_day(date, today), time);
            marks.push_str(&format!(
                r#"<a class="fp-wp" style="--x:{x:.3}%;--t:{tt:.3}{dx}" href="{href}" title="{title}" tabindex="-1"></a>"#,
                x = x_pct(day),
                tt = sweep_t(day),
                href = entity_href(&m.id),
                title = escape_html(&format!("{}, {}", when.trim(), display_name(m))),
            ));
        }
        if list.len() > WAYPOINTS_PER_DAY {
            marks.push_str(&format!(
                r#"<span class="fp-wp-more" style="--x:{:.3}%">+{}</span>"#,
                x_pct(day),
                list.len() - WAYPOINTS_PER_DAY
            ));
        }
    }

    // Axis
    let mut axis = String::new();
    for i in 0..DAYS {
        let d = start + Duration::days(i);
        if d.weekday() == Weekday::Mon {
            // Leave room for the "Today" label when a week starts next to it.
            let label = if (i - DAYS_BEFORE).abs() <= 2 {
                String::new()
            } else {
                format!("<span>{}</span>", iso_week(d))
            };
            axis.push_str(&format!(
                r#"<span class="fp-week" style="--x:{:.3}%">{label}</span>"#,
                edge_pct(i),
            ));
        }
    }
    let now = x_pct(DAYS_BEFORE);
    axis.push_str(&format!(
        r#"<span class="fp-now" style="--x:{now:.3}%"><span>Today</span></span>"#
    ));
    if marks.is_empty() && older == 0 {
        axis.push_str(r#"<p class="fp-empty">Nothing scheduled in the next six weeks. Give a task a due date to see it here.</p>"#);
    }

    let older_html = if older > 0 {
        format!(
            r#"<a class="fp-older" href="/tasks/list?sort=due_date&amp;dir=asc" title="{}"><span class="readout">{older}</span><span>older</span></a>"#,
            escape_html(&format!(
                "{} overdue by more than a week",
                plural(older, "task", "tasks")
            ))
        )
    } else {
        r#"<span class="fp-older is-zero" title="No tasks overdue by more than a week"><span class="readout">0</span><span>older</span></span>"#.to_string()
    };

    // A key for the marks, not a control: tick colour is urgency, the
    // diamond is a meeting.
    let key = |tone: &str, label: &str| {
        format!(
            r#"<li><span class="fp-key-tick tone-{tone}" aria-hidden="true"></span>{label}</li>"#
        )
    };
    let mut legend = format!(
        "{}{}{}",
        key("negative", "Late"),
        key("pending", "Due within a week"),
        key("neutral", "Later")
    );
    if has_meetings {
        legend.push_str(r#"<li><span class="fp-wp-key" aria-hidden="true"></span>Meeting</li>"#);
    }

    let mut summary = format!("{} overdue, {} due within a week", overdue, due_soon);
    if has_meetings {
        summary.push_str(&format!(
            ", {} in the next six weeks",
            plural(meetings_ahead, "meeting", "meetings")
        ));
    }

    format!(
        r#"<section class="flight-plan" aria-labelledby="fp-title">
<div class="fp-head"><h2 class="section-title" id="fp-title">Next six weeks</h2><ul class="fp-legend" aria-label="Key">{legend}</ul></div>
<div class="fp-body">{older_html}<div class="fp-scroll"><div class="fp-track" style="--now:{now:.3}%" aria-hidden="true">{axis}{marks}</div></div></div>
<p class="fp-summary">{summary}</p>
</section>"#
    )
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    #[test]
    fn x_positions_centre_each_day() {
        assert_eq!(format!("{:.3}", x_pct(7)), "15.306");
        assert_eq!(format!("{:.3}", x_pct(0)), "1.020");
        assert_eq!(format!("{:.3}", edge_pct(0)), "0.000");
        assert_eq!(format!("{:.3}", sweep_t(48)), "1.000");
    }

    #[test]
    fn tones_follow_due_thresholds() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let at = |n| tick_tone(today + Duration::days(n), today);
        assert_eq!(at(-1), "negative");
        assert_eq!(at(0), "pending");
        assert_eq!(at(7), "pending");
        assert_eq!(at(8), "neutral");
    }

    #[test]
    fn stacking_overflows_past_the_limit() {
        // 22+2+16+2 = 42; another 11 would reach 53 > 52.
        let (ys, overflow, next) = stack(&[22, 16, 11, 7]);
        assert_eq!(ys, vec![0, 24]);
        assert_eq!(overflow, 2);
        assert_eq!(next, 42);
        let (ys, overflow, _) = stack(&[7, 7]);
        assert_eq!(ys, vec![0, 9]);
        assert_eq!(overflow, 0);
    }

    #[test]
    fn renders_ticks_older_bin_and_waypoints() {
        let (_d, cfg) = test_config();
        let today = chrono::Local::now().date_naive();
        let date = |n: i64| (today + Duration::days(n)).format("%Y-%m-%d").to_string();
        let cat = catalog(
            vec![
                rec(
                    EntityKind::Task,
                    "TASK-001",
                    &format!(
                        "title: Late\nstatus: todo\npriority: 1\ndue_date: {}",
                        date(-2)
                    ),
                ),
                rec(
                    EntityKind::Task,
                    "TASK-002",
                    &format!("title: Ancient\nstatus: todo\ndue_date: {}", date(-30)),
                ),
                rec(
                    EntityKind::Task,
                    "TASK-003",
                    &format!("title: Done\nstatus: done\ndue_date: {}", date(3)),
                ),
                rec(
                    EntityKind::Meeting,
                    "MTG-001",
                    &format!("title: Sync <x>\ndate: {}\ntime: '14:00'", date(3)),
                ),
            ],
            &cfg,
        );
        let page = Page {
            cfg: &cfg,
            catalog: &cat,
            custom_css: "",
            today,
            editable: false,
        };
        let html = flight_plan(&page);
        assert_eq!(html.matches("class=\"fp-tick").count(), 1);
        assert!(html.contains("fp-tick tone-negative"));
        assert!(html.contains("--h:22px"));
        assert!(html.contains(r#"<span class="readout">1</span><span>older</span>"#));
        assert_eq!(html.matches("class=\"fp-wp\"").count(), 1);
        assert!(html.contains("Sync &lt;x&gt;"));
        assert!(html.contains("2 overdue, 0 due within a week, 1 meeting in the next six weeks"));
        assert!(!html.contains("fp-empty"));
    }

    #[test]
    fn empty_plan_shows_hint() {
        let (_d, cfg) = test_config();
        let cat = catalog(Vec::new(), &cfg);
        let page = Page {
            cfg: &cfg,
            catalog: &cat,
            custom_css: "",
            today: NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(),
            editable: false,
        };
        let html = flight_plan(&page);
        assert!(html.contains("fp-empty"));
        assert!(html.contains("fp-older is-zero"));
        // Today is a Monday: its week label gives way to "Today".
        assert!(!html.contains("W41"));
        assert!(html.contains("W42"));
    }
}
